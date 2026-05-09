use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use hotstore_core::{deserialize_record, ColumnFamily, EventRecord, ObjectRecord, TxRecord};
use hotstore_db::BackendKind;
use rocksdb::{IteratorMode, Options, DB};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct BenchKeyExportConfig {
    pub tx_limit: usize,
    pub object_version_limit: usize,
    pub object_id_limit: usize,
    pub event_type_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchKeyManifest {
    pub backend: String,
    pub db_path: String,
    pub generated_files: BenchKeyFiles,
    pub counts: BenchKeyCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchKeyFiles {
    pub tx_digests: String,
    pub object_versions: String,
    pub object_ids: String,
    pub event_types: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchKeyCounts {
    pub tx_digests: usize,
    pub object_versions: usize,
    pub object_ids: usize,
    pub event_types: usize,
}

pub fn export_bench_keys(
    backend: BackendKind,
    db_path: &Path,
    out_dir: &Path,
    config: &BenchKeyExportConfig,
) -> Result<BenchKeyManifest> {
    match backend {
        BackendKind::RocksDb | BackendKind::ToplingDb => {
            export_rocksdb_bench_keys(backend, db_path, out_dir, config)
        }
    }
}

fn export_rocksdb_bench_keys(
    backend: BackendKind,
    db_path: &Path,
    out_dir: &Path,
    config: &BenchKeyExportConfig,
) -> Result<BenchKeyManifest> {
    let db = open_read_only_db(db_path)?;
    fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create output dir {}", out_dir.display()))?;

    let tx_digests = collect_tx_digests(&db, config.tx_limit)?;
    let object_versions = collect_object_versions(&db, config.object_version_limit)?;
    let object_ids = collect_object_ids(&db, config.object_id_limit)?;
    let event_types = collect_event_types(&db, config.event_type_limit)?;

    let tx_digests_path = out_dir.join("tx_digests.txt");
    let object_versions_path = out_dir.join("object_versions.txt");
    let object_ids_path = out_dir.join("object_ids.txt");
    let event_types_path = out_dir.join("event_types.txt");
    let manifest_path = out_dir.join("manifest.json");

    write_lines(&tx_digests_path, &tx_digests)?;
    write_lines(&object_versions_path, &object_versions)?;
    write_lines(&object_ids_path, &object_ids)?;
    write_lines(&event_types_path, &event_types)?;

    let manifest = BenchKeyManifest {
        backend: backend.to_string(),
        db_path: db_path.display().to_string(),
        generated_files: BenchKeyFiles {
            tx_digests: tx_digests_path.display().to_string(),
            object_versions: object_versions_path.display().to_string(),
            object_ids: object_ids_path.display().to_string(),
            event_types: event_types_path.display().to_string(),
        },
        counts: BenchKeyCounts {
            tx_digests: tx_digests.len(),
            object_versions: object_versions.len(),
            object_ids: object_ids.len(),
            event_types: event_types.len(),
        },
    };

    let bytes = serde_json::to_vec_pretty(&manifest).context("failed to serialize key manifest")?;
    fs::write(&manifest_path, bytes)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    Ok(manifest)
}

fn open_read_only_db(db_path: &Path) -> Result<DB> {
    let mut opts = Options::default();
    opts.create_if_missing(false);
    let cf_names = DB::list_cf(&opts, db_path)
        .with_context(|| format!("failed to list column families in {}", db_path.display()))?;
    DB::open_cf_for_read_only(&opts, db_path, cf_names, false)
        .with_context(|| format!("failed to open RocksDB at {}", db_path.display()))
}

fn collect_tx_digests(db: &DB, limit: usize) -> Result<Vec<String>> {
    collect_records(db, ColumnFamily::TxByDigest, limit, |record: TxRecord| {
        decode_utf8(record.digest, "tx digest")
    })
}

fn collect_object_versions(db: &DB, limit: usize) -> Result<Vec<String>> {
    collect_records(
        db,
        ColumnFamily::ObjectVersion,
        limit,
        |record: ObjectRecord| {
            Ok(format!(
                "{},{}",
                decode_utf8(record.object_id, "object id")?,
                record.version
            ))
        },
    )
}

fn collect_object_ids(db: &DB, limit: usize) -> Result<Vec<String>> {
    collect_records(
        db,
        ColumnFamily::ObjectLastSeen,
        limit,
        |record: ObjectRecord| decode_utf8(record.object_id, "object id"),
    )
}

fn collect_event_types(db: &DB, limit: usize) -> Result<Vec<String>> {
    let handle = db
        .cf_handle(ColumnFamily::EventByType.as_str())
        .with_context(|| format!("missing column family `{}`", ColumnFamily::EventByType))?;
    let iter = db.iterator_cf(&handle, IteratorMode::Start);
    let mut values = BTreeSet::new();

    for row in iter {
        let (_, value) = row.with_context(|| {
            format!(
                "failed to iterate column family `{}`",
                ColumnFamily::EventByType
            )
        })?;
        let record: EventRecord =
            deserialize_record(&value).context("failed to decode event record")?;
        values.insert(record.event_type);
        if values.len() >= limit {
            break;
        }
    }

    Ok(values.into_iter().collect())
}

fn collect_records<T, F>(db: &DB, cf: ColumnFamily, limit: usize, mut map: F) -> Result<Vec<String>>
where
    T: serde::de::DeserializeOwned,
    F: FnMut(T) -> Result<String>,
{
    let handle = db
        .cf_handle(cf.as_str())
        .with_context(|| format!("missing column family `{cf}`"))?;
    let iter = db.iterator_cf(&handle, IteratorMode::Start);
    let mut out = Vec::with_capacity(limit);

    for row in iter {
        let (_, value) = row.with_context(|| format!("failed to iterate column family `{cf}`"))?;
        let record: T = deserialize_record(&value)
            .with_context(|| format!("failed to decode record in `{cf}`"))?;
        out.push(map(record)?);
        if out.len() >= limit {
            break;
        }
    }

    Ok(out)
}

fn write_lines(path: &Path, lines: &[String]) -> Result<()> {
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

fn decode_utf8(bytes: Vec<u8>, label: &str) -> Result<String> {
    String::from_utf8(bytes).with_context(|| format!("{label} is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hotstore_core::{serialize_record, EventRecord, ObjectRecord, TxRecord};
    use hotstore_db::{RocksDbBackend, StorageEngine};
    use tempfile::TempDir;

    use super::*;

    fn seed_db(path: &Path) -> Result<()> {
        let db = RocksDbBackend::open(path)?;

        db.put(
            ColumnFamily::TxByDigest,
            b"tx-1",
            &serialize_record(&TxRecord {
                digest: b"tx-1".to_vec(),
                checkpoint: 1,
                tx_index: 0,
                sender: None,
                status: "success".to_owned(),
                gas_used: Some(7),
                event_count: 1,
                changed_object_count: 1,
                raw_effects_bytes: None,
            })?,
        )?;
        db.put(
            ColumnFamily::TxByDigest,
            b"tx-2",
            &serialize_record(&TxRecord {
                digest: b"tx-2".to_vec(),
                checkpoint: 1,
                tx_index: 1,
                sender: None,
                status: "success".to_owned(),
                gas_used: Some(8),
                event_count: 1,
                changed_object_count: 1,
                raw_effects_bytes: None,
            })?,
        )?;

        db.put(
            ColumnFamily::ObjectVersion,
            b"obj-1-key",
            &serialize_record(&ObjectRecord {
                object_id: b"0xobj1".to_vec(),
                version: 11,
                checkpoint: 1,
                owner: None,
                type_tag: None,
                raw_object_bytes: None,
            })?,
        )?;
        db.put(
            ColumnFamily::ObjectVersion,
            b"obj-2-key",
            &serialize_record(&ObjectRecord {
                object_id: b"0xobj2".to_vec(),
                version: 12,
                checkpoint: 1,
                owner: None,
                type_tag: None,
                raw_object_bytes: None,
            })?,
        )?;

        db.put(
            ColumnFamily::ObjectLastSeen,
            b"obj-last-1",
            &serialize_record(&ObjectRecord {
                object_id: b"0xobj1".to_vec(),
                version: 11,
                checkpoint: 1,
                owner: None,
                type_tag: None,
                raw_object_bytes: None,
            })?,
        )?;

        db.put(
            ColumnFamily::EventByType,
            b"event-1",
            &serialize_record(&EventRecord {
                event_type: "pkg::module::A".to_owned(),
                checkpoint: 1,
                tx_digest: b"tx-1".to_vec(),
                sender: None,
                package_id: None,
                module: None,
                event_name: Some("A".to_owned()),
                payload: Vec::new(),
            })?,
        )?;
        db.put(
            ColumnFamily::EventByType,
            b"event-2",
            &serialize_record(&EventRecord {
                event_type: "pkg::module::B".to_owned(),
                checkpoint: 1,
                tx_digest: b"tx-2".to_vec(),
                sender: None,
                package_id: None,
                module: None,
                event_name: Some("B".to_owned()),
                payload: Vec::new(),
            })?,
        )?;

        Ok(())
    }

    #[test]
    fn export_bench_keys_writes_expected_files() {
        let db_dir = TempDir::new().expect("db temp dir");
        let out_dir = TempDir::new().expect("out temp dir");
        seed_db(db_dir.path()).expect("seed db");

        let manifest = export_bench_keys(
            BackendKind::RocksDb,
            db_dir.path(),
            out_dir.path(),
            &BenchKeyExportConfig {
                tx_limit: 10,
                object_version_limit: 10,
                object_id_limit: 10,
                event_type_limit: 10,
            },
        )
        .expect("export bench keys");

        assert_eq!(manifest.counts.tx_digests, 2);
        assert_eq!(manifest.counts.object_versions, 2);
        assert_eq!(manifest.counts.object_ids, 1);
        assert_eq!(manifest.counts.event_types, 2);

        let tx_digests =
            fs::read_to_string(out_dir.path().join("tx_digests.txt")).expect("read tx digests");
        let object_versions = fs::read_to_string(out_dir.path().join("object_versions.txt"))
            .expect("read object versions");
        let object_ids =
            fs::read_to_string(out_dir.path().join("object_ids.txt")).expect("read object ids");
        let event_types =
            fs::read_to_string(out_dir.path().join("event_types.txt")).expect("read event types");

        assert!(tx_digests.contains("tx-1"));
        assert!(tx_digests.contains("tx-2"));
        assert!(object_versions.contains("0xobj1,11"));
        assert!(object_versions.contains("0xobj2,12"));
        assert_eq!(object_ids.trim(), "0xobj1");
        assert!(event_types.contains("pkg::module::A"));
        assert!(event_types.contains("pkg::module::B"));
        assert!(out_dir.path().join("manifest.json").is_file());
    }
}
