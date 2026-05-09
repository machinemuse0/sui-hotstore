mod bench_keys;
mod cli;
mod commit;
mod extract;
mod source;

use anyhow::{Context, Result};
use clap::Parser;
use hotstore_core::{ColumnFamily, META_WATERMARK_CHECKPOINT};
use hotstore_db::open_backend;
use hotstore_db::HotWriteBatch;
use std::time::Instant;

use crate::bench_keys::{BenchKeyBatch, BenchKeySink};
use crate::commit::{persist_aggregated_batch, persist_run_metadata, IngestTotals};
use crate::extract::extract_checkpoint_batch;
use crate::source::RpcSourceClient;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    cli.validate()?;

    let backend = open_backend(cli.backend_kind()?, &cli.db_path)
        .with_context(|| format!("failed to open backend at {}", cli.db_path.display()))?;
    let rpc_client = RpcSourceClient::new(cli.rpc_url(), cli.max_retries, cli.retry_backoff_ms)?;
    let bench_key_sink = cli
        .bench_keys_dir
        .as_ref()
        .map(|dir| BenchKeySink::open(dir, &cli.network, cli.first_checkpoint, cli.last_checkpoint))
        .transpose()?;
    let effective_first_checkpoint = resolve_effective_first_checkpoint(&*backend, &cli, bench_key_sink.as_ref())?;

    persist_run_metadata(&*backend, &cli, None)?;

    if effective_first_checkpoint > cli.last_checkpoint {
        if let Some(sink) = &bench_key_sink {
            let manifest = sink.finalize()?;
            eprintln!(
                "nothing to ingest; existing progress already covers checkpoints {}-{} (keys: txs={}, object_versions={}, object_ids={}, event_types={})",
                cli.first_checkpoint,
                cli.last_checkpoint,
                manifest.counts.tx_digests,
                manifest.counts.object_versions,
                manifest.counts.object_ids,
                manifest.counts.event_types
            );
        } else {
            eprintln!(
                "nothing to ingest; existing progress already covers checkpoints {}-{}",
                cli.first_checkpoint, cli.last_checkpoint
            );
        }
        return Ok(());
    }

    let started_at = Instant::now();
    let total_checkpoints = cli.last_checkpoint - cli.first_checkpoint + 1;
    let mut totals = IngestTotals::default();
    let mut pending_batch = HotWriteBatch::new();
    let mut pending_keys = BenchKeyBatch::default();
    let mut pending_checkpoints = 0usize;

    for checkpoint_seq in effective_first_checkpoint..=cli.last_checkpoint {
        let checkpoint = rpc_client.fetch_checkpoint(checkpoint_seq).await?;
        let txs = rpc_client
            .fetch_transaction_blocks(&checkpoint.transactions, cli.tx_batch_size)
            .await?;

        let extracted = extract_checkpoint_batch(
            &checkpoint,
            &txs,
            &cli.network,
            &cli.source_label(),
            cli.record_mode_kind()?,
        )?;

        totals.record(&extracted.stats);
        pending_keys.append(extracted.bench_keys);
        pending_batch.append(extracted.batch);
        pending_checkpoints += 1;

        let checkpoints_done = totals.checkpoint_count;
        let should_flush = pending_checkpoints >= cli.checkpoint_batch_size
            || checkpoint_seq == cli.last_checkpoint;

        if should_flush {
            let flush_count = pending_checkpoints;
            let batch_to_commit = std::mem::take(&mut pending_batch);
            persist_aggregated_batch(&*backend, &cli, checkpoint_seq, batch_to_commit, &totals)?;
            if let Some(sink) = &bench_key_sink {
                sink.append_batch(&pending_keys, checkpoint_seq)?;
            }
            pending_keys.clear();
            pending_checkpoints = 0;

            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let checkpoints_per_sec = checkpoints_done as f64 / elapsed;
            eprintln!(
                "flushed through checkpoint {} ({}/{}, just_flushed={}, total_txs={}, total_events={}, total_object_changes={}, {:.2} ckpt/s)",
                checkpoint_seq,
                checkpoints_done,
                total_checkpoints,
                flush_count,
                totals.tx_count,
                totals.event_count,
                totals.object_version_count,
                checkpoints_per_sec
            );
        } else {
            eprintln!(
                "buffered checkpoint {} (txs={}, events={}, object_changes={}, pending_flush={}/{})",
                checkpoint_seq,
                extracted.stats.tx_count,
                extracted.stats.event_count,
                extracted.stats.object_version_count,
                pending_checkpoints,
                cli.checkpoint_batch_size
            );
        }

        eprintln!(
            "ingested checkpoint {} (txs={}, events={}, object_changes={})",
            checkpoint_seq,
            extracted.stats.tx_count,
            extracted.stats.event_count,
            extracted.stats.object_version_count
        );
    }

    persist_run_metadata(&*backend, &cli, Some(&totals))?;
    if let Some(sink) = &bench_key_sink {
        let manifest = sink.finalize()?;
        eprintln!(
            "bench key files ready: txs={}, object_versions={}, object_ids={}, event_types={} ({})",
            manifest.counts.tx_digests,
            manifest.counts.object_versions,
            manifest.counts.object_ids,
            manifest.counts.event_types,
            cli.bench_keys_dir
                .as_ref()
                .expect("bench_keys_dir present when sink exists")
                .display()
        );
    }
    eprintln!(
        "ingest complete: checkpoints={}, txs={}, events={}, object_changes={}",
        totals.checkpoint_count, totals.tx_count, totals.event_count, totals.object_version_count
    );

    Ok(())
}

fn resolve_effective_first_checkpoint(
    db: &dyn hotstore_db::StorageEngine,
    cli: &cli::Cli,
    bench_key_sink: Option<&BenchKeySink>,
) -> Result<u64> {
    if !cli.resume {
        return Ok(cli.first_checkpoint);
    }

    let db_watermark = db
        .get(ColumnFamily::Meta, META_WATERMARK_CHECKPOINT.as_bytes())?
        .map(|bytes| String::from_utf8(bytes))
        .transpose()
        .context("failed to decode db watermark as utf-8")?
        .map(|text| {
            text.parse::<u64>()
                .with_context(|| format!("failed to parse db watermark checkpoint `{text}`"))
        })
        .transpose()?;

    let key_watermark = match (&cli.bench_keys_dir, bench_key_sink) {
        (Some(dir), Some(_)) => BenchKeySink::read_progress(dir)?
            .map(|progress| progress.last_flushed_checkpoint),
        _ => None,
    };

    let completed_checkpoint = if cli.bench_keys_dir.is_some() {
        match (db_watermark, key_watermark) {
            (Some(db_checkpoint), Some(key_checkpoint)) => Some(db_checkpoint.min(key_checkpoint)),
            _ => None,
        }
    } else {
        db_watermark
    };

    let effective_first_checkpoint = completed_checkpoint
        .map(|checkpoint| checkpoint.saturating_add(1))
        .unwrap_or(cli.first_checkpoint)
        .max(cli.first_checkpoint);

    if effective_first_checkpoint > cli.first_checkpoint {
        eprintln!(
            "resume enabled: continuing from checkpoint {} (requested range {}-{}, db_watermark={:?}, key_watermark={:?})",
            effective_first_checkpoint,
            cli.first_checkpoint,
            cli.last_checkpoint,
            db_watermark,
            key_watermark
        );
    } else {
        eprintln!(
            "resume enabled: no complete shared progress detected for requested range {}-{}; starting from the beginning",
            cli.first_checkpoint, cli.last_checkpoint
        );
    }

    Ok(effective_first_checkpoint)
}
