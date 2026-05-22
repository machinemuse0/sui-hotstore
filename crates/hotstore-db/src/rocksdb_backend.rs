use std::array;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use hotstore_core::ColumnFamily;
use rocksdb::{
    Direction, DBIteratorWithThreadMode, IteratorMode, Options, ReadOptions, WriteBatch, DB,
};

use crate::traits::{HotWriteBatch, ScanOutcome, StorageEngine, ThreadContext};

const CF_COUNT: usize = ColumnFamily::ALL.len();

pub struct RocksDbBackend {
    // Cached CF handles to avoid db.cf_handle() RwLock+HashMap on every op.
    cf_handles: [Arc<rocksdb::BoundColumnFamily<'static>>; CF_COUNT],
    db: Arc<DB>,
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
        let db = Arc::new(db);

        let cf_handles = array::from_fn(|i| {
            let cf = ColumnFamily::ALL[i];
            let handle = db.cf_handle(cf.as_str())
                .unwrap_or_else(|| panic!("missing RocksDB column family `{cf}` after open"));
            // SAFETY: BoundColumnFamily<'a> has only raw CF handle + PhantomData<&DB>.
            // The Arc keeps the C handle alive, cf_handles drops before db.
            unsafe {
                std::mem::transmute::<
                    Arc<rocksdb::BoundColumnFamily<'_>>,
                    Arc<rocksdb::BoundColumnFamily<'static>>,
                >(handle)
            }
        });

        Ok(Self { cf_handles, db })
    }

    #[inline]
    fn cf_handle(&self, cf: ColumnFamily) -> &Arc<rocksdb::BoundColumnFamily<'static>> {
        &self.cf_handles[cf as usize]
    }

    fn with_read_options<T>(&self, f: impl FnOnce(&ReadOptions) -> T) -> T {
        let mut readopts = ReadOptions::default();
        let _pin = readopts.scope_pin();
        f(&readopts)
    }
}

/// Per-thread context for RocksDB/ToplingDB operations.
///
/// Holds one `ReadOptions` (per-DB) and one cached iterator per
/// column family, eliminating repeated C API calls in hot loops
/// such as [`scan_prefix_count`](StorageEngine::scan_prefix_count).
///
/// The `'static` on the cached iterators is safe because the caller
/// (`RocksDbBackend`, which owns `Arc<DB>`) outlives this context.
pub struct RocksDbThreadContext {
    readopts: ReadOptions,
    iters: [Option<DBIteratorWithThreadMode<'static, DB>>; CF_COUNT],
}

impl RocksDbThreadContext {
    fn new() -> Self {
        let mut readopts = ReadOptions::default();
        readopts.start_pin();
        Self {
            readopts,
            iters: array::from_fn(|_| None),
        }
    }

    /// Run a callback with the cached `ReadOptions`.
    fn with_read_options<T>(&self, f: impl FnOnce(&ReadOptions) -> T) -> T {
        f(&self.readopts)
    }

    /// Get or create the cached iterator for the given column family.
    fn iter_for_cf(&mut self, db: &DB, cf: ColumnFamily) -> &mut DBIteratorWithThreadMode<'static, DB> {
        let idx = cf as usize;
        if self.iters[idx].is_none() {
            let cf_handle = db.cf_handle(cf.as_str())
                .unwrap_or_else(|| panic!("missing RocksDB column family `{cf}` after open"));
            let iter = db.iterator_cf(&cf_handle, IteratorMode::Start);
            // SAFETY: the caller (`RocksDbBackend`) owns `Arc<DB>` and
            // outlives this context, so the DB lives at least as long
            // as any cached iterator.
            let iter: DBIteratorWithThreadMode<'static, DB> = unsafe {
                std::mem::transmute::<
                    DBIteratorWithThreadMode<'_, DB>,
                    DBIteratorWithThreadMode<'static, DB>,
                >(iter)
            };
            self.iters[idx] = Some(iter);
        }
        self.iters[idx].as_mut().unwrap()
    }
}

impl Drop for RocksDbThreadContext {
    fn drop(&mut self) {
        self.iters.iter_mut().for_each(|iter| *iter = None);
        self.readopts.finish_pin();
    }
}

impl std::fmt::Debug for RocksDbThreadContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocksDbThreadContext").finish()
    }
}

impl StorageEngine for RocksDbBackend {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let handle = self.cf_handle(cf);
        self.with_read_options(|readopts| {
            self.db
                .get_cf_opt(handle, key, readopts)
                .with_context(|| format!("RocksDB get failed for `{cf}`"))
        })
    }

    fn multi_get(&self, cf: ColumnFamily, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
        let handle = self.cf_handle(cf);
        self.with_read_options(|readopts| {
            self.db
                .batched_multi_get_cf_opt(handle, keys.iter(), false, readopts)
                .into_iter()
                .map(|result| {
                    result
                        .map(|value| value.map(|value| value.to_vec()))
                        .with_context(|| format!("RocksDB multi_get failed for `{cf}`"))
                })
                .collect()
        })
    }

    fn get_pinned_with(
        &self,
        ctx: &dyn ThreadContext,
        cf: ColumnFamily,
        key: &[u8],
        f: &mut dyn FnMut(Option<&[u8]>),
    ) -> Result<()> {
        let rocks_ctx = ctx
            .as_any_ref()
            .downcast_ref::<RocksDbThreadContext>()
            .expect("RocksDbBackend requires RocksDbThreadContext");
        let handle = self.cf_handle(cf);
        rocks_ctx.with_read_options(|readopts| {
            let pinned = self
                .db
                .get_pinned_cf_opt(handle, key, readopts)
                .with_context(|| format!("RocksDB get failed for `{cf}`"))?;
            f(pinned.as_ref().map(|slice| slice.as_ref()));
            Ok(())
        })
    }

    fn multi_get_pinned_with(
        &self,
        ctx: &dyn ThreadContext,
        cf: ColumnFamily,
        keys: &[&[u8]],
        f: &mut dyn FnMut(usize, Option<&[u8]>),
    ) -> Result<()> {
        let rocks_ctx = ctx
            .as_any_ref()
            .downcast_ref::<RocksDbThreadContext>()
            .expect("RocksDbBackend requires RocksDbThreadContext");
        let handle = self.cf_handle(cf);
        rocks_ctx.with_read_options(|readopts| {
            let results = self
                .db
                .batched_multi_get_cf_opt(handle, keys.iter(), false, readopts);
            for (idx, result) in results.into_iter().enumerate() {
                let pinned =
                    result.with_context(|| format!("RocksDB multi_get failed for `{cf}`"))?;
                f(idx, pinned.as_ref().map(|slice| slice.as_ref()));
            }
            Ok(())
        })
    }

    fn multi_get_impl(&self) -> &'static str {
        "native_batched_multi_get_cf"
    }

    fn cf_handle_mode(&self) -> &'static str {
        "cached-at-open"
    }

    fn create_thread_context(&self) -> Box<dyn ThreadContext> {
        Box::new(RocksDbThreadContext::new())
    }

    fn put(&self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        let handle = self.cf_handle(cf);
        self.db
            .put_cf(handle, key, value)
            .with_context(|| format!("RocksDB put failed for `{cf}`"))
    }

    fn write_batch(&self, batch: HotWriteBatch) -> Result<()> {
        let mut write_batch = WriteBatch::default();

        for op in batch.iter() {
            let handle = self.cf_handle(op.cf);
            write_batch.put_cf(handle, &op.key, &op.value);
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
        let handle = self.cf_handle(cf);
        let iter = self
            .db
            .iterator_cf(handle, IteratorMode::From(prefix, Direction::Forward));

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
        ctx: &mut dyn ThreadContext,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<ScanOutcome> {
        let rocks_ctx = ctx
            .as_any_mut()
            .downcast_mut::<RocksDbThreadContext>()
            .expect("RocksDbBackend requires RocksDbThreadContext");

        let iter = rocks_ctx.iter_for_cf(&*self.db, cf);
        iter.set_mode(IteratorMode::From(prefix, Direction::Forward));

        let mut outcome = ScanOutcome::default();
        while let Some(key) = iter.key() {
            if !key.starts_with(prefix) {
                break;
            }
            outcome.rows += 1;
            outcome.key_bytes += key.len();
            if outcome.rows >= limit {
                break;
            }
            iter.advance_by(1);
        }

        Ok(outcome)
    }

    fn compact_all(&self) -> Result<()> {
        for cf in ColumnFamily::ALL {
            let handle = self.cf_handle(cf);
            self.db
                .compact_range_cf(handle, None::<&[u8]>, None::<&[u8]>);
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

    #[test]
    fn rocksdb_backend_pinned_callbacks_return_borrowed_values() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db = RocksDbBackend::open(temp_dir.path()).expect("open rocksdb");

        db.put(ColumnFamily::Meta, b"a", b"alpha").expect("put a");
        db.put(ColumnFamily::Meta, b"b", b"beta").expect("put b");

        let ctx = db.create_thread_context();

        let mut got_a: Option<Vec<u8>> = None;
        db.get_pinned_with(&*ctx, ColumnFamily::Meta, b"a", &mut |slice| {
            got_a = slice.map(|value| value.to_vec());
        })
        .expect("get_pinned_with a");
        assert_eq!(got_a.as_deref(), Some(b"alpha".as_slice()));

        let mut got_missing: Option<Vec<u8>> = Some(vec![]);
        db.get_pinned_with(&*ctx, ColumnFamily::Meta, b"missing", &mut |slice| {
            got_missing = slice.map(|value| value.to_vec());
        })
        .expect("get_pinned_with missing");
        assert!(got_missing.is_none());

        let keys: &[&[u8]] = &[b"a", b"missing", b"b"];
        let mut seen: Vec<(usize, Option<Vec<u8>>)> = Vec::new();
        db.multi_get_pinned_with(&*ctx, ColumnFamily::Meta, keys, &mut |idx, slice| {
            seen.push((idx, slice.map(|value| value.to_vec())));
        })
        .expect("multi_get_pinned_with");
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], (0, Some(b"alpha".to_vec())));
        assert_eq!(seen[1], (1, None));
        assert_eq!(seen[2], (2, Some(b"beta".to_vec())));
    }
}
