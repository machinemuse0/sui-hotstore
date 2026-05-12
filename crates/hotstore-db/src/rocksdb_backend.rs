use std::array;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use hotstore_core::ColumnFamily;
use rocksdb::{
    BoundColumnFamily, Direction, IteratorMode, Options, ReadOptions,
    ReadOptionsScopePinIfNotPinned, WriteBatch, DB,
};

use crate::traits::{HotWriteBatch, ScanOutcome, StorageEngine};

const CF_COUNT: usize = ColumnFamily::ALL.len();
const READ_OPTIONS_MODE_ENV: &str = "HOTSTORE_READ_OPTIONS_MODE";

static NEXT_READ_OPTIONS_KEY: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static READ_OPTIONS_CACHE: RefCell<HashMap<usize, Box<ThreadReadOptions>>> =
        RefCell::new(HashMap::new());
}

struct CachedCfHandle(Arc<BoundColumnFamily<'static>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadOptionsMode {
    ThreadLocalScopePin,
    ThreadLocalLongPin,
}

struct ThreadReadOptions {
    readopts: ReadOptions,
    mode: ReadOptionsMode,
    _db_guard: Option<Arc<DB>>,
}

#[derive(Debug)]
pub struct RocksDbBackend {
    // Drop order matters: cached handles must be released before the DB closes.
    cf_handles: [CachedCfHandle; CF_COUNT],
    db: Arc<DB>,
    read_options_key: usize,
    read_options_mode: ReadOptionsMode,
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
        let cf_handles =
            array::from_fn(|index| Self::cache_cf_handle(&db, ColumnFamily::ALL[index]));
        let read_options_key = NEXT_READ_OPTIONS_KEY.fetch_add(1, Ordering::Relaxed);
        let read_options_mode = configured_read_options_mode();

        Ok(Self {
            cf_handles,
            db,
            read_options_key,
            read_options_mode,
        })
    }

    fn cache_cf_handle(db: &Arc<DB>, cf: ColumnFamily) -> CachedCfHandle {
        let handle = db
            .cf_handle(cf.as_str())
            .unwrap_or_else(|| panic!("missing RocksDB column family `{cf}` after open"));
        CachedCfHandle(static_cf_handle(handle))
    }

    #[inline]
    fn cf_handle(&self, cf: ColumnFamily) -> &Arc<BoundColumnFamily<'static>> {
        &self.cf_handles[cf_index(cf)].0
    }

    fn with_read_options<T>(&self, f: impl FnOnce(&ReadOptions) -> T) -> T {
        let readopts = READ_OPTIONS_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let entry = cache.entry(self.read_options_key).or_insert_with(|| {
                Box::new(ThreadReadOptions::new(
                    self.read_options_mode,
                    self.db.clone(),
                ))
            });
            &**entry as *const ThreadReadOptions
        });

        // SAFETY: the pointer targets a Box stored in this thread's TLS map.
        // We release the RefCell borrow before running `f`, so nested reads do
        // not panic. The Box allocation itself is stable even if a nested call
        // inserts more entries into the HashMap.
        unsafe { (*readopts).with_read_options(f) }
    }
}

impl Drop for RocksDbBackend {
    fn drop(&mut self) {
        let _ = READ_OPTIONS_CACHE.try_with(|cache| {
            cache.borrow_mut().remove(&self.read_options_key);
        });
    }
}

impl std::fmt::Debug for CachedCfHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CachedCfHandle")
    }
}

#[allow(unsafe_code)]
// SAFETY: RocksDB/ToplingDB column-family handles are safe to share across
// threads. The upstream binding models the lifetime through a raw pointer plus
// PhantomData, so make the cached-handle assumption explicit at this boundary.
unsafe impl Send for CachedCfHandle {}

#[allow(unsafe_code)]
// SAFETY: see the Send impl above.
unsafe impl Sync for CachedCfHandle {}

impl ThreadReadOptions {
    fn new(mode: ReadOptionsMode, db: Arc<DB>) -> Self {
        let mut readopts = ReadOptions::default();
        match mode {
            ReadOptionsMode::ThreadLocalScopePin => Self {
                readopts,
                mode,
                _db_guard: None,
            },
            ReadOptionsMode::ThreadLocalLongPin => {
                readopts.start_pin();
                Self {
                    readopts,
                    mode,
                    _db_guard: Some(db),
                }
            }
        }
    }

    fn with_read_options<T>(&self, f: impl FnOnce(&ReadOptions) -> T) -> T {
        match self.mode {
            ReadOptionsMode::ThreadLocalScopePin => {
                let _pin = ReadOptionsScopePinIfNotPinned::from(&self.readopts);
                f(&self.readopts)
            }
            ReadOptionsMode::ThreadLocalLongPin => f(&self.readopts),
        }
    }
}

impl ReadOptionsMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadLocalScopePin => "thread-local-scope-pin",
            Self::ThreadLocalLongPin => "thread-local-long-pin",
        }
    }
}

impl Drop for ThreadReadOptions {
    fn drop(&mut self) {
        if self.mode == ReadOptionsMode::ThreadLocalLongPin {
            self.readopts.finish_pin();
        }
    }
}

#[inline]
fn cf_index(cf: ColumnFamily) -> usize {
    match cf {
        ColumnFamily::Meta => 0,
        ColumnFamily::Checkpoint => 1,
        ColumnFamily::TxByDigest => 2,
        ColumnFamily::ObjectVersion => 3,
        ColumnFamily::ObjectLastSeen => 4,
        ColumnFamily::EventByType => 5,
        ColumnFamily::OwnerTouchedObjects => 6,
    }
}

fn configured_read_options_mode() -> ReadOptionsMode {
    match std::env::var(READ_OPTIONS_MODE_ENV).as_deref() {
        Ok("thread-local-long-pin" | "long-pin") => ReadOptionsMode::ThreadLocalLongPin,
        _ => ReadOptionsMode::ThreadLocalScopePin,
    }
}

#[allow(unsafe_code)]
fn static_cf_handle(handle: Arc<BoundColumnFamily<'_>>) -> Arc<BoundColumnFamily<'static>> {
    // SAFETY: `BoundColumnFamily<'a>` stores only a raw CF handle plus a
    // PhantomData lifetime tying it to the DB. We cache the Arc to avoid the
    // binding's per-read RwLock/map lookup, and `RocksDbBackend` declares
    // `cf_handles` before `db` so these cached handles drop before the DB.
    unsafe {
        std::mem::transmute::<Arc<BoundColumnFamily<'_>>, Arc<BoundColumnFamily<'static>>>(handle)
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

    fn multi_get_impl(&self) -> &'static str {
        "native_batched_multi_get_cf"
    }

    fn cf_handle_mode(&self) -> &'static str {
        "cached-at-open"
    }

    fn read_options_mode(&self) -> &'static str {
        self.read_options_mode.as_str()
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
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<ScanOutcome> {
        let handle = self.cf_handle(cf);
        let iter = self
            .db
            .iterator_cf(handle, IteratorMode::From(prefix, Direction::Forward));

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
}
