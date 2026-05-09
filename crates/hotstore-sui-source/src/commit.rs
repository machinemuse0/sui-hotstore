use anyhow::Result;
use hotstore_core::{
    ColumnFamily, META_BACKEND_NAME, META_DATASET_NAME, META_DATASET_NETWORK,
    META_DATASET_RANGE_END, META_DATASET_RANGE_START, META_WATERMARK_CHECKPOINT,
};
use hotstore_db::{HotWriteBatch, StorageEngine};
use serde::Serialize;

use crate::cli::Cli;
use crate::extract::BatchStats;

const META_RECORD_MODE: &str = "record:mode";
const META_SOURCE_LABEL: &str = "source:label";
const META_STATS_TOTALS: &str = "stats:totals";

#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestTotals {
    pub checkpoint_count: u64,
    pub tx_count: u64,
    pub event_count: u64,
    pub object_version_count: u64,
    pub owner_touched_count: u64,
}

impl IngestTotals {
    pub fn record(&mut self, batch: &BatchStats) {
        self.checkpoint_count += batch.checkpoint_count;
        self.tx_count += batch.tx_count;
        self.event_count += batch.event_count;
        self.object_version_count += batch.object_version_count;
        self.owner_touched_count += batch.owner_touched_count;
    }
}

pub fn persist_run_metadata(
    db: &dyn StorageEngine,
    cli: &Cli,
    totals: Option<&IngestTotals>,
) -> Result<()> {
    let mut batch = HotWriteBatch::new();
    batch.put(
        ColumnFamily::Meta,
        META_DATASET_NAME,
        b"sui-hotstore-demo-rpc".to_vec(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_DATASET_NETWORK,
        cli.network.as_bytes().to_vec(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_DATASET_RANGE_START,
        cli.first_checkpoint.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_DATASET_RANGE_END,
        cli.last_checkpoint.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_BACKEND_NAME,
        cli.backend_kind()?.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_RECORD_MODE,
        format!("{:?}", cli.record_mode_kind()?)
            .to_lowercase()
            .into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_SOURCE_LABEL,
        cli.source_label().into_bytes(),
    );

    if let Some(totals) = totals {
        batch.put(
            ColumnFamily::Meta,
            META_STATS_TOTALS,
            serde_json::to_vec(totals)?,
        );
    }

    db.write_batch(batch)
}

pub fn persist_aggregated_batch(
    db: &dyn StorageEngine,
    cli: &Cli,
    checkpoint_seq: u64,
    batch: HotWriteBatch,
    totals: &IngestTotals,
) -> Result<()> {
    let mut batch = batch;
    batch.put(
        ColumnFamily::Meta,
        META_WATERMARK_CHECKPOINT,
        checkpoint_seq.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_DATASET_RANGE_END,
        checkpoint_seq.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_BACKEND_NAME,
        cli.backend_kind()?.to_string().into_bytes(),
    );
    batch.put(
        ColumnFamily::Meta,
        META_STATS_TOTALS,
        serde_json::to_vec(totals)?,
    );

    db.write_batch(batch)
}
