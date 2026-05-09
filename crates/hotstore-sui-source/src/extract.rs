use anyhow::{bail, Context, Result};
use hotstore_core::{
    key_checkpoint, key_event_by_type, key_object_last_seen, key_object_version,
    key_owner_touched_object, key_tx_by_digest, serialize_record, CheckpointRecord, ColumnFamily,
    EventRecord, ObjectRecord, OwnerTouchedObjectRecord, TxRecord,
};
use hotstore_db::HotWriteBatch;
use serde_json::Value;

use crate::bench_keys::BenchKeyBatch;
use crate::source::{RpcCheckpoint, RpcTransactionBlockResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordMode {
    Lite,
    Raw,
    Full,
}

#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    pub checkpoint_count: u64,
    pub tx_count: u64,
    pub event_count: u64,
    pub object_version_count: u64,
    pub owner_touched_count: u64,
}

#[derive(Debug, Clone)]
pub struct ExtractedCheckpointBatch {
    pub batch: HotWriteBatch,
    pub stats: BatchStats,
    pub bench_keys: BenchKeyBatch,
}

pub fn extract_checkpoint_batch(
    checkpoint: &RpcCheckpoint,
    txs: &[RpcTransactionBlockResponse],
    network: &str,
    source: &str,
    record_mode: RecordMode,
) -> Result<ExtractedCheckpointBatch> {
    if matches!(record_mode, RecordMode::Full) {
        bail!("record-mode=full is not implemented in the demo ingest path");
    }

    let sequence_number = checkpoint.sequence_number.as_u64()?;
    let timestamp_ms = checkpoint.timestamp_ms.as_u64()?;

    let mut batch = HotWriteBatch::new();
    let mut bench_keys = BenchKeyBatch::default();
    let mut stats = BatchStats {
        checkpoint_count: 1,
        tx_count: txs.len() as u64,
        ..BatchStats::default()
    };

    let checkpoint_record = CheckpointRecord {
        network: network.to_owned(),
        sequence_number,
        timestamp_ms,
        tx_count: txs.len() as u32,
        event_count: txs
            .iter()
            .map(|tx| tx.events.as_ref().map_or(0, |v| v.len() as u32))
            .sum(),
        object_change_count: txs
            .iter()
            .map(|tx| tx.object_changes.as_ref().map_or(0, |v| v.len() as u32))
            .sum(),
        source: source.to_owned(),
    };
    batch.put(
        ColumnFamily::Checkpoint,
        key_checkpoint(sequence_number),
        serialize_record(&checkpoint_record)?,
    );

    for (tx_index, tx) in txs.iter().enumerate() {
        bench_keys.tx_digests.push(tx.digest.clone());
        let sender = tx
            .transaction
            .as_ref()
            .and_then(|txn| txn.data.sender.as_ref())
            .map(|sender| sender.as_bytes().to_vec());

        let status = tx
            .effects
            .as_ref()
            .and_then(|effects| effects.status.as_ref())
            .map(|status| status.status.clone())
            .unwrap_or_else(|| "unknown".to_owned());

        let gas_used = tx
            .effects
            .as_ref()
            .and_then(|effects| effects.gas_used.as_ref())
            .map(total_gas_used)
            .transpose()?;

        let raw_effects_bytes = if matches!(record_mode, RecordMode::Raw) {
            tx.effects
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .context("failed to serialize raw transaction effects")?
        } else {
            None
        };

        let events = tx.events.as_deref().unwrap_or(&[]);
        let object_changes = tx.object_changes.as_deref().unwrap_or(&[]);

        let tx_record = TxRecord {
            digest: tx.digest.as_bytes().to_vec(),
            checkpoint: sequence_number,
            tx_index: tx_index as u32,
            sender,
            status,
            gas_used,
            event_count: events.len() as u32,
            changed_object_count: object_changes.len() as u32,
            raw_effects_bytes,
        };

        batch.put(
            ColumnFamily::TxByDigest,
            key_tx_by_digest(tx.digest.as_bytes()),
            serialize_record(&tx_record)?,
        );

        for (event_index, event) in events.iter().enumerate() {
            bench_keys.event_types.push(event.event_type.clone());
            let payload = match (record_mode, event.parsed_json.as_ref()) {
                (RecordMode::Raw, _) => serde_json::to_vec(event)
                    .context("failed to serialize raw Sui event for storage")?,
                (_, Some(parsed_json)) => serde_json::to_vec(parsed_json)
                    .context("failed to serialize parsed Sui event payload")?,
                (_, None) => Vec::new(),
            };

            let event_record = EventRecord {
                event_type: event.event_type.clone(),
                checkpoint: sequence_number,
                tx_digest: tx.digest.as_bytes().to_vec(),
                sender: event
                    .sender
                    .as_ref()
                    .map(|sender| sender.as_bytes().to_vec()),
                package_id: event.package_id.clone(),
                module: event.transaction_module.clone(),
                event_name: event_name_from_type(&event.event_type),
                payload,
            };

            batch.put(
                ColumnFamily::EventByType,
                key_event_by_type(
                    &event.event_type,
                    sequence_number,
                    tx_index as u32,
                    event_index as u32,
                ),
                serialize_record(&event_record)?,
            );
            stats.event_count += 1;
        }

        for change in object_changes {
            let Some(object_id) = change.object_id.as_ref() else {
                continue;
            };

            let version = change
                .version
                .as_ref()
                .map(|value| value.as_u64())
                .transpose()?
                .unwrap_or_default();
            bench_keys
                .object_versions
                .push(format!("{object_id},{version}"));
            bench_keys.object_ids.push(object_id.clone());

            let owner_value = change.recipient.as_ref().or(change.owner.as_ref());
            let owner_bytes = owner_value.map(owner_bytes).transpose()?;
            let raw_object_bytes = if matches!(record_mode, RecordMode::Raw) {
                Some(
                    serde_json::to_vec(change)
                        .context("failed to serialize raw object change for storage")?,
                )
            } else {
                None
            };

            let object_record = ObjectRecord {
                object_id: object_id.as_bytes().to_vec(),
                version,
                checkpoint: sequence_number,
                owner: owner_bytes.clone(),
                type_tag: change.object_type.clone(),
                raw_object_bytes,
            };

            let object_record_bytes = serialize_record(&object_record)?;
            batch.put(
                ColumnFamily::ObjectVersion,
                key_object_version(object_id.as_bytes(), version),
                object_record_bytes.clone(),
            );
            batch.put(
                ColumnFamily::ObjectLastSeen,
                key_object_last_seen(object_id.as_bytes()),
                object_record_bytes,
            );
            stats.object_version_count += 1;

            if let Some(owner) = owner_bytes {
                let owner_record = OwnerTouchedObjectRecord {
                    owner: owner.clone(),
                    object_id: object_id.as_bytes().to_vec(),
                    version,
                    checkpoint: sequence_number,
                    type_tag: change.object_type.clone(),
                };

                batch.put(
                    ColumnFamily::OwnerTouchedObjects,
                    key_owner_touched_object(
                        &owner,
                        change.object_type.as_deref(),
                        object_id.as_bytes(),
                        version,
                    ),
                    serialize_record(&owner_record)?,
                );
                stats.owner_touched_count += 1;
            }
        }
    }

    Ok(ExtractedCheckpointBatch {
        batch,
        stats,
        bench_keys,
    })
}

fn total_gas_used(gas: &crate::source::RpcGasUsed) -> Result<u64> {
    let computation = gas.computation_cost.as_u64()?;
    let storage = gas.storage_cost.as_u64()?;
    let rebate = gas
        .storage_rebate
        .as_ref()
        .map(|value| value.as_u64())
        .transpose()?
        .unwrap_or_default();
    Ok(computation.saturating_add(storage).saturating_sub(rebate))
}

fn owner_bytes(owner: &Value) -> Result<Vec<u8>> {
    serde_json::to_string(owner)
        .context("failed to encode owner value")
        .map(|text| text.into_bytes())
}

fn event_name_from_type(event_type: &str) -> Option<String> {
    event_type.rsplit("::").next().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::event_name_from_type;

    #[test]
    fn event_name_is_derived_from_struct_tag() {
        assert_eq!(
            event_name_from_type("deepbook::order::Fill"),
            Some("Fill".to_owned())
        );
    }
}
