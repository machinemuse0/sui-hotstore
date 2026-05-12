#![deny(unsafe_code)]

pub mod cf;
pub mod rocksdb_backend;
pub mod toplingdb_backend;
pub mod traits;

pub use rocksdb_backend::RocksDbBackend;
pub use toplingdb_backend::ToplingDbBackend;
pub use traits::{BackendKind, HotWriteBatch, StorageEngine, WriteOp};

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

pub fn open_backend(kind: BackendKind, path: impl AsRef<Path>) -> Result<Arc<dyn StorageEngine>> {
    match kind {
        BackendKind::RocksDb => Ok(Arc::new(RocksDbBackend::open(path)?)),
        BackendKind::ToplingDb => Ok(Arc::new(ToplingDbBackend::open(path)?)),
    }
}
