use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use hotstore_core::ColumnFamily;

use crate::rocksdb_backend::RocksDbBackend;
use crate::traits::{HotWriteBatch, StorageEngine};

pub const TOPLINGDB_EASY_MIGRATE_CONF_ENV: &str = "TOPLINGDB_EASY_MIGRATE_CONF";

#[derive(Debug)]
pub struct ToplingDbBackend {
    _path: PathBuf,
    _config_path: PathBuf,
    inner: RocksDbBackend,
}

impl ToplingDbBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let config_path = env::var_os(TOPLINGDB_EASY_MIGRATE_CONF_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ToplingDB backend requires {} to point at a ToplingDB YAML config",
                    TOPLINGDB_EASY_MIGRATE_CONF_ENV
                )
            })?;
        if !config_path.is_file() {
            bail!(
                "{} does not point to a readable file: {}",
                TOPLINGDB_EASY_MIGRATE_CONF_ENV,
                config_path.display()
            );
        }

        let path = path.as_ref().to_path_buf();
        let inner = RocksDbBackend::open(&path).with_context(|| {
            format!(
                "failed to open ToplingDB-compatible store at {} using config {}",
                path.display(),
                config_path.display()
            )
        })?;

        Ok(Self {
            _path: path,
            _config_path: config_path,
            inner,
        })
    }
}

impl StorageEngine for ToplingDbBackend {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.inner.get(cf, key)
    }

    fn multi_get(&self, cf: ColumnFamily, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>> {
        self.inner.multi_get(cf, keys)
    }

    fn get_pinned_with(
        &self,
        cf: ColumnFamily,
        key: &[u8],
        f: &mut dyn FnMut(Option<&[u8]>),
    ) -> Result<()> {
        self.inner.get_pinned_with(cf, key, f)
    }

    fn multi_get_pinned_with(
        &self,
        cf: ColumnFamily,
        keys: &[&[u8]],
        f: &mut dyn FnMut(usize, Option<&[u8]>),
    ) -> Result<()> {
        self.inner.multi_get_pinned_with(cf, keys, f)
    }

    fn multi_get_impl(&self) -> &'static str {
        self.inner.multi_get_impl()
    }

    fn cf_handle_mode(&self) -> &'static str {
        self.inner.cf_handle_mode()
    }

    fn read_options_mode(&self) -> &'static str {
        self.inner.read_options_mode()
    }

    fn put(&self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        self.inner.put(cf, key, value)
    }

    fn write_batch(&self, batch: HotWriteBatch) -> Result<()> {
        self.inner.write_batch(batch)
    }

    fn scan_prefix(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.scan_prefix(cf, prefix, limit)
    }

    fn scan_prefix_count(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<crate::traits::ScanOutcome> {
        self.inner.scan_prefix_count(cf, prefix, limit)
    }

    fn compact_all(&self) -> Result<()> {
        self.inner.compact_all()
    }
}
