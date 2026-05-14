use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use hotstore_core::ColumnFamily;
use hotstore_db::{open_backend, BackendKind, HotWriteBatch};
use rocksdb::{IteratorMode, Options, DB};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"HSRAW001";
const ROW_TAG: u8 = 1;
const TOPLINGDB_EASY_MIGRATE_CONF_ENV: &str = "TOPLINGDB_EASY_MIGRATE_CONF";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSyncReport {
    pub backend: String,
    pub db_path: String,
    pub column_families: BTreeMap<String, RawColumnFamilyReport>,
    pub totals: RawSyncTotals,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawColumnFamilyReport {
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawSyncTotals {
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct RawImportConfig {
    pub batch_rows: usize,
    pub allow_existing: bool,
    pub compact: bool,
}

pub fn export_raw_to_path(
    backend: BackendKind,
    db_path: &Path,
    output: Option<&Path>,
) -> Result<RawSyncReport> {
    match output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create parent directory for {}", path.display())
                })?;
            }
            let file = File::create(path)
                .with_context(|| format!("failed to create {}", path.display()))?;
            let mut writer = BufWriter::new(file);
            let report = export_raw(backend, db_path, &mut writer)?;
            writer
                .flush()
                .with_context(|| format!("failed to flush {}", path.display()))?;
            Ok(report)
        }
        None => {
            let stdout = io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            let report = export_raw(backend, db_path, &mut writer)?;
            writer
                .flush()
                .context("failed to flush raw export stdout")?;
            Ok(report)
        }
    }
}

pub fn import_raw_from_path(
    backend: BackendKind,
    db_path: &Path,
    input: Option<&Path>,
    config: RawImportConfig,
) -> Result<RawSyncReport> {
    match input {
        Some(path) => {
            let file =
                File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
            let mut reader = BufReader::new(file);
            import_raw(backend, db_path, &mut reader, config)
        }
        None => {
            let stdin = io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            import_raw(backend, db_path, &mut reader, config)
        }
    }
}

pub fn export_raw(
    backend: BackendKind,
    db_path: &Path,
    writer: &mut impl Write,
) -> Result<RawSyncReport> {
    validate_backend_environment(backend)?;
    let db = open_read_only_db(db_path)?;
    write_header(writer)?;

    let mut builders = RawReportBuilders::default();

    for (cf_index, cf) in ColumnFamily::ALL.into_iter().enumerate() {
        let cf_name = cf.as_str();
        let handle = db
            .cf_handle(cf_name)
            .with_context(|| format!("missing column family handle `{cf_name}`"))?;
        let iter = db.iterator_cf(&handle, IteratorMode::Start);

        for row in iter {
            let (key, value) =
                row.with_context(|| format!("failed to iterate column family `{cf_name}`"))?;
            write_row(writer, cf_index, &key, &value)?;
            builders.record(cf, &key, &value);
        }
    }

    Ok(builders.finish(backend, db_path))
}

pub fn import_raw(
    backend: BackendKind,
    db_path: &Path,
    reader: &mut impl Read,
    config: RawImportConfig,
) -> Result<RawSyncReport> {
    if config.batch_rows == 0 {
        bail!("--batch-rows must be greater than zero");
    }
    ensure_import_target(db_path, config.allow_existing)?;
    read_and_validate_header(reader)?;

    let db = open_backend(backend, db_path)
        .with_context(|| format!("failed to open import target {}", db_path.display()))?;
    let mut builders = RawReportBuilders::default();
    let mut batch = HotWriteBatch::new();

    loop {
        let Some((cf, key, value)) = read_row(reader)? else {
            break;
        };

        builders.record(cf, &key, &value);
        batch.put(cf, key, value);

        if batch.len() >= config.batch_rows {
            let flushed_rows = batch.len();
            db.write_batch(batch)?;
            eprintln!("imported {flushed_rows} rows in latest batch");
            batch = HotWriteBatch::new();
        }
    }

    if !batch.is_empty() {
        let flushed_rows = batch.len();
        db.write_batch(batch)?;
        eprintln!("imported {flushed_rows} rows in final batch");
    }

    if config.compact {
        eprintln!("compacting imported database");
        db.compact_all()?;
    }

    Ok(builders.finish(backend, db_path))
}

fn validate_backend_environment(backend: BackendKind) -> Result<()> {
    if backend != BackendKind::ToplingDb {
        return Ok(());
    }

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

    Ok(())
}

fn open_read_only_db(db_path: &Path) -> Result<DB> {
    let mut opts = Options::default();
    opts.create_if_missing(false);

    let cf_names = DB::list_cf(&opts, db_path)
        .with_context(|| format!("failed to list column families in {}", db_path.display()))?;

    DB::open_cf_for_read_only(&opts, db_path, cf_names, false).with_context(|| {
        format!(
            "failed to open RocksDB-compatible store at {}",
            db_path.display()
        )
    })
}

fn ensure_import_target(db_path: &Path, allow_existing: bool) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent directory {}", parent.display()))?;
    }

    if !db_path.exists() || allow_existing {
        return Ok(());
    }

    let mut entries = fs::read_dir(db_path)
        .with_context(|| format!("failed to read import target {}", db_path.display()))?;
    if entries.next().is_some() {
        bail!(
            "import target {} already exists and is not empty; use a new directory or --allow-existing",
            db_path.display()
        );
    }

    Ok(())
}

fn write_header(writer: &mut impl Write) -> Result<()> {
    writer
        .write_all(MAGIC)
        .context("failed to write raw header")?;
    write_u32(writer, ColumnFamily::ALL.len() as u32)?;

    for (index, cf) in ColumnFamily::ALL.into_iter().enumerate() {
        let name = cf.as_str().as_bytes();
        writer
            .write_all(&[index as u8])
            .context("failed to write column family index")?;
        write_u16(writer, name.len() as u16)?;
        writer
            .write_all(name)
            .context("failed to write column family name")?;
    }

    Ok(())
}

fn read_and_validate_header(reader: &mut impl Read) -> Result<()> {
    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .context("failed to read raw header")?;
    if &magic != MAGIC {
        bail!("invalid raw stream magic");
    }

    let cf_count = read_u32(reader)? as usize;
    if cf_count != ColumnFamily::ALL.len() {
        bail!(
            "raw stream has {cf_count} column families, expected {}",
            ColumnFamily::ALL.len()
        );
    }

    for expected_index in 0..cf_count {
        let mut index = [0u8; 1];
        reader
            .read_exact(&mut index)
            .context("failed to read column family index")?;
        if index[0] as usize != expected_index {
            bail!(
                "raw stream column family index mismatch: got {}, expected {expected_index}",
                index[0]
            );
        }

        let name_len = read_u16(reader)? as usize;
        let mut name = vec![0u8; name_len];
        reader
            .read_exact(&mut name)
            .context("failed to read column family name")?;
        let name = std::str::from_utf8(&name).context("column family name is not UTF-8")?;
        let expected_name = ColumnFamily::ALL[expected_index].as_str();
        if name != expected_name {
            bail!("raw stream column family mismatch: got `{name}`, expected `{expected_name}`");
        }
    }

    Ok(())
}

fn write_row(writer: &mut impl Write, cf_index: usize, key: &[u8], value: &[u8]) -> Result<()> {
    if cf_index > u8::MAX as usize {
        bail!("column family index {cf_index} does not fit in raw stream");
    }
    if key.len() > u32::MAX as usize {
        bail!("key is too large for raw stream");
    }
    if value.len() > u32::MAX as usize {
        bail!("value is too large for raw stream");
    }

    writer
        .write_all(&[ROW_TAG])
        .context("failed to write row tag")?;
    writer
        .write_all(&[cf_index as u8])
        .context("failed to write row column family")?;
    write_u32(writer, key.len() as u32)?;
    write_u32(writer, value.len() as u32)?;
    writer.write_all(key).context("failed to write row key")?;
    writer
        .write_all(value)
        .context("failed to write row value")?;

    Ok(())
}

fn read_row(reader: &mut impl Read) -> Result<Option<(ColumnFamily, Vec<u8>, Vec<u8>)>> {
    let mut tag = [0u8; 1];
    match reader.read(&mut tag).context("failed to read row tag")? {
        0 => return Ok(None),
        1 => {}
        _ => bail!("short read while reading row tag"),
    }

    if tag[0] != ROW_TAG {
        bail!("invalid row tag {}", tag[0]);
    }

    let mut cf_index = [0u8; 1];
    reader
        .read_exact(&mut cf_index)
        .context("failed to read row column family")?;
    let cf = ColumnFamily::ALL
        .get(cf_index[0] as usize)
        .copied()
        .with_context(|| format!("invalid column family index {}", cf_index[0]))?;
    let key_len = read_u32(reader)? as usize;
    let value_len = read_u32(reader)? as usize;
    let mut key = vec![0u8; key_len];
    let mut value = vec![0u8; value_len];
    reader
        .read_exact(&mut key)
        .context("failed to read row key")?;
    reader
        .read_exact(&mut value)
        .context("failed to read row value")?;

    Ok(Some((cf, key, value)))
}

fn write_u16(writer: &mut impl Write, value: u16) -> Result<()> {
    writer
        .write_all(&value.to_be_bytes())
        .context("failed to write u16")
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<()> {
    writer
        .write_all(&value.to_be_bytes())
        .context("failed to write u32")
}

fn read_u16(reader: &mut impl Read) -> Result<u16> {
    let mut bytes = [0u8; 2];
    reader
        .read_exact(&mut bytes)
        .context("failed to read u16")?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .context("failed to read u32")?;
    Ok(u32::from_be_bytes(bytes))
}

#[derive(Default)]
struct RawReportBuilders {
    column_families: BTreeMap<String, RawColumnFamilyBuilder>,
    totals: RawTotalsBuilder,
}

impl RawReportBuilders {
    fn record(&mut self, cf: ColumnFamily, key: &[u8], value: &[u8]) {
        let cf_name = cf.as_str().to_owned();
        self.column_families
            .entry(cf_name)
            .or_default()
            .record(key, value);
        self.totals.record(cf, key, value);
    }

    fn finish(mut self, backend: BackendKind, db_path: &Path) -> RawSyncReport {
        let mut column_families = BTreeMap::new();
        for cf in ColumnFamily::ALL {
            let name = cf.as_str().to_owned();
            let report = self
                .column_families
                .remove(&name)
                .unwrap_or_default()
                .finish();
            column_families.insert(name, report);
        }

        RawSyncReport {
            backend: backend.to_string(),
            db_path: db_path.display().to_string(),
            column_families,
            totals: self.totals.finish(),
        }
    }
}

struct RawColumnFamilyBuilder {
    entries: u64,
    key_bytes: u64,
    value_bytes: u64,
    hasher: Sha256,
}

impl Default for RawColumnFamilyBuilder {
    fn default() -> Self {
        Self {
            entries: 0,
            key_bytes: 0,
            value_bytes: 0,
            hasher: Sha256::new(),
        }
    }
}

impl RawColumnFamilyBuilder {
    fn record(&mut self, key: &[u8], value: &[u8]) {
        self.entries += 1;
        self.key_bytes += key.len() as u64;
        self.value_bytes += value.len() as u64;
        update_row_digest(&mut self.hasher, key, value);
    }

    fn finish(self) -> RawColumnFamilyReport {
        RawColumnFamilyReport {
            entries: self.entries,
            key_bytes: self.key_bytes,
            value_bytes: self.value_bytes,
            sha256: hex::encode(self.hasher.finalize()),
        }
    }
}

struct RawTotalsBuilder {
    entries: u64,
    key_bytes: u64,
    value_bytes: u64,
    hasher: Sha256,
}

impl Default for RawTotalsBuilder {
    fn default() -> Self {
        Self {
            entries: 0,
            key_bytes: 0,
            value_bytes: 0,
            hasher: Sha256::new(),
        }
    }
}

impl RawTotalsBuilder {
    fn record(&mut self, cf: ColumnFamily, key: &[u8], value: &[u8]) {
        self.entries += 1;
        self.key_bytes += key.len() as u64;
        self.value_bytes += value.len() as u64;
        let cf_name = cf.as_str();
        self.hasher.update((cf_name.len() as u64).to_be_bytes());
        self.hasher.update(cf_name.as_bytes());
        update_row_digest(&mut self.hasher, key, value);
    }

    fn finish(self) -> RawSyncTotals {
        RawSyncTotals {
            entries: self.entries,
            key_bytes: self.key_bytes,
            value_bytes: self.value_bytes,
            sha256: hex::encode(self.hasher.finalize()),
        }
    }
}

fn update_row_digest(hasher: &mut Sha256, key: &[u8], value: &[u8]) {
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use hotstore_db::{RocksDbBackend, StorageEngine};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn raw_export_import_round_trips_hotstore_cfs() {
        let source_dir = TempDir::new().expect("source temp dir");
        let target_dir = TempDir::new().expect("target temp dir");
        let target_path = target_dir.path().join("db");

        let source = RocksDbBackend::open(source_dir.path()).expect("open source");
        source
            .put(ColumnFamily::Meta, b"dataset:name", b"demo")
            .expect("put meta");
        source
            .put(ColumnFamily::Checkpoint, b"ckpt-1", b"payload-1")
            .expect("put checkpoint");
        source
            .put(ColumnFamily::TxByDigest, b"tx-1", b"tx-payload")
            .expect("put tx");
        drop(source);

        let mut raw = Vec::new();
        let export_report =
            export_raw(BackendKind::RocksDb, source_dir.path(), &mut raw).expect("export raw");
        assert_eq!(export_report.totals.entries, 3);

        let mut raw_reader = raw.as_slice();
        let import_report = import_raw(
            BackendKind::RocksDb,
            &target_path,
            &mut raw_reader,
            RawImportConfig {
                batch_rows: 2,
                allow_existing: false,
                compact: false,
            },
        )
        .expect("import raw");

        assert_eq!(import_report.totals.entries, 3);
        assert_eq!(import_report.totals.sha256, export_report.totals.sha256);

        let target = RocksDbBackend::open(&target_path).expect("open target");
        assert_eq!(
            target
                .get(ColumnFamily::TxByDigest, b"tx-1")
                .expect("get tx")
                .as_deref(),
            Some(b"tx-payload".as_slice())
        );
    }
}
