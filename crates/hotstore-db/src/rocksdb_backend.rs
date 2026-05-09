use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use hotstore_core::ColumnFamily;
use rocksdb::{Direction, IteratorMode, Options, WriteBatch, DB};

use crate::traits::{HotWriteBatch, ScanOutcome, StorageEngine};

#[derive(Debug)]
pub struct RocksDbBackend {
    db: DB,
}

impl RocksDbBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);

        let descriptors = ColumnFamily::ALL
            .into_iter()
            .map(|cf| rocksdb::ColumnFamilyDescriptor::new(cf.as_str(), Options::default()))
            .collect::<Vec<_>>();

        let db = DB::open_cf_descriptors(&options, path, descriptors)
            .context("failed to open RocksDB backend")?;

        Ok(Self { db })
    }

    fn cf_handle(&self, cf: ColumnFamily) -> Result<Arc<rocksdb::BoundColumnFamily<'_>>> {
        self.db
            .cf_handle(cf.as_str())
            .with_context(|| format!("missing RocksDB column family `{cf}`"))
    }
}

impl StorageEngine for RocksDbBackend {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let handle = self.cf_handle(cf)?;
        self.db
            .get_cf(&handle, key)
            .with_context(|| format!("RocksDB get failed for `{cf}`"))
    }

    fn multi_get(&self, cf: ColumnFamily, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
        let handle = self.cf_handle(cf)?;
        self.db
            .batched_multi_get_cf(&handle, keys.iter().copied(), false)
            .into_iter()
            .map(|result| {
                result
                    .map(|value| value.map(|value| value.to_vec()))
                    .with_context(|| format!("RocksDB multi_get failed for `{cf}`"))
            })
            .collect()
    }

    fn multi_get_impl(&self) -> &'static str {
        "native_batched_multi_get_cf"
    }

    fn put(&self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        let handle = self.cf_handle(cf)?;
        self.db
            .put_cf(&handle, key, value)
            .with_context(|| format!("RocksDB put failed for `{cf}`"))
    }

    fn write_batch(&self, batch: HotWriteBatch) -> Result<()> {
        let mut write_batch = WriteBatch::default();

        for op in batch.iter() {
            let handle = self.cf_handle(op.cf)?;
            write_batch.put_cf(&handle, &op.key, &op.value);
        }

        self.db
            .write(write_batch)
            .context("RocksDB write batch failed")
    }

    fn scan_prefix(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let handle = self.cf_handle(cf)?;
        let iter = self
            .db
            .iterator_cf(&handle, IteratorMode::From(prefix, Direction::Forward));

        let mut rows = Vec::with_capacity(limit);
        for entry in iter {
            let (key, value) =
                entry.with_context(|| format!("RocksDB iterator failed for `{cf}`"))?;
            if !key.starts_with(prefix) {
                break;
            }

            rows.push((key.to_vec(), value.to_vec()));
            if rows.len() >= limit {
                break;
            }
        }

        Ok(rows)
    }

    fn scan_prefix_count(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<ScanOutcome> {
        let handle = self.cf_handle(cf)?;
        let iter = self
            .db
            .iterator_cf(&handle, IteratorMode::From(prefix, Direction::Forward));

        let mut outcome = ScanOutcome::default();
        for entry in iter {
            let (key, value) =
                entry.with_context(|| format!("RocksDB iterator failed for `{cf}`"))?;
            if !key.starts_with(prefix) {
                break;
            }

            outcome.rows += 1;
            outcome.key_bytes += key.len();
            outcome.value_bytes += value.len();
            if outcome.rows >= limit {
                break;
            }
        }

        Ok(outcome)
    }

    fn compact_all(&self) -> Result<()> {
        for cf in ColumnFamily::ALL {
            let handle = self.cf_handle(cf)?;
            self.db
                .compact_range_cf(&handle, None::<&[u8]>, None::<&[u8]>);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use hotstore_core::key_checkpoint;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn rocksdb_backend_supports_batch_and_prefix_scan() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db = RocksDbBackend::open(temp_dir.path()).expect("open rocksdb");

        let mut batch = HotWriteBatch::new();
        batch.put(
            ColumnFamily::Meta,
            b"dataset:name".to_vec(),
            b"testnet-demo".to_vec(),
        );
        batch.put(
            ColumnFamily::Checkpoint,
            key_checkpoint(7),
            b"checkpoint-7".to_vec(),
        );
        batch.put(
            ColumnFamily::Checkpoint,
            key_checkpoint(8),
            b"checkpoint-8".to_vec(),
        );

        db.write_batch(batch).expect("write batch");

        assert_eq!(
            db.get(ColumnFamily::Meta, b"dataset:name")
                .expect("meta get")
                .as_deref(),
            Some(b"testnet-demo".as_slice())
        );

        let scanned = db
            .scan_prefix(ColumnFamily::Checkpoint, &key_checkpoint(7)[..4], 10)
            .expect("scan prefix");
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0].1, b"checkpoint-7".to_vec());
        assert_eq!(scanned[1].1, b"checkpoint-8".to_vec());
    }
}
