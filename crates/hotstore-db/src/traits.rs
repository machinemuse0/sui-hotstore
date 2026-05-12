use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use hotstore_core::ColumnFamily;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOp {
    pub cf: ColumnFamily,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotWriteBatch {
    ops: Vec<WriteOp>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOutcome {
    pub rows: usize,
    pub key_bytes: usize,
    pub value_bytes: usize,
}

impl HotWriteBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, cf: ColumnFamily, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.ops.push(WriteOp {
            cf,
            key: key.into(),
            value: value.into(),
        });
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &WriteOp> {
        self.ops.iter()
    }

    pub fn append(&mut self, mut other: Self) {
        self.ops.append(&mut other.ops);
    }
}

pub trait StorageEngine: Send + Sync + 'static {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>>;

    fn multi_get(&self, cf: ColumnFamily, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>>;

    fn multi_get_impl(&self) -> &'static str {
        "unknown"
    }

    fn cf_handle_mode(&self) -> &'static str {
        "backend-default"
    }

    fn read_options_mode(&self) -> &'static str {
        "backend-default"
    }

    fn put(&self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()>;

    fn write_batch(&self, batch: HotWriteBatch) -> Result<()>;

    fn scan_prefix(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;

    fn scan_prefix_count(
        &self,
        cf: ColumnFamily,
        prefix: &[u8],
        limit: usize,
    ) -> Result<ScanOutcome> {
        let rows = self.scan_prefix(cf, prefix, limit)?;
        Ok(ScanOutcome {
            rows: rows.len(),
            key_bytes: rows.iter().map(|(key, _)| key.len()).sum(),
            value_bytes: rows.iter().map(|(_, value)| value.len()).sum(),
        })
    }

    fn compact_all(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    RocksDb,
    ToplingDb,
}

impl BackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RocksDb => "rocksdb",
            Self::ToplingDb => "toplingdb",
        }
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "rocksdb" => Ok(Self::RocksDb),
            "toplingdb" => Ok(Self::ToplingDb),
            other => Err(anyhow!("unsupported backend `{other}`")),
        }
    }
}
