use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use hotstore_db::BackendKind;
use rocksdb::{IteratorMode, Options, DB};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsReport {
    pub backend: String,
    pub db_path: String,
    pub disk_usage_bytes: u64,
    pub column_families: BTreeMap<String, ColumnFamilyStats>,
    pub totals: StatsTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnFamilyStats {
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsTotals {
    pub column_family_count: usize,
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumReport {
    pub backend: String,
    pub db_path: String,
    pub disk_usage_bytes: u64,
    pub column_families: BTreeMap<String, ColumnFamilyChecksum>,
    pub totals: ChecksumTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnFamilyChecksum {
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumTotals {
    pub column_family_count: usize,
    pub entries: u64,
    pub key_bytes: u64,
    pub value_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareChecksumReport {
    pub matches: bool,
    pub left_path: String,
    pub right_path: String,
    pub only_in_left: Vec<String>,
    pub only_in_right: Vec<String>,
    pub mismatches: Vec<ChecksumMismatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecksumMismatch {
    pub column_family: String,
    pub reason: String,
    pub left_entries: u64,
    pub right_entries: u64,
    pub left_sha256: String,
    pub right_sha256: String,
}

pub fn compute_stats_report(backend: BackendKind, db_path: &Path) -> Result<StatsReport> {
    let snapshot = inspect_backend(backend, db_path, false)?;
    Ok(StatsReport {
        backend: backend.to_string(),
        db_path: db_path.display().to_string(),
        disk_usage_bytes: dir_size_bytes(db_path)?,
        totals: StatsTotals {
            column_family_count: snapshot.column_families.len(),
            entries: snapshot.totals.entries,
            key_bytes: snapshot.totals.key_bytes,
            value_bytes: snapshot.totals.value_bytes,
        },
        column_families: snapshot
            .column_families
            .into_iter()
            .map(|(name, metrics)| {
                (
                    name,
                    ColumnFamilyStats {
                        entries: metrics.entries,
                        key_bytes: metrics.key_bytes,
                        value_bytes: metrics.value_bytes,
                    },
                )
            })
            .collect(),
    })
}

pub fn compute_checksum_report(backend: BackendKind, db_path: &Path) -> Result<ChecksumReport> {
    let snapshot = inspect_backend(backend, db_path, true)?;
    Ok(ChecksumReport {
        backend: backend.to_string(),
        db_path: db_path.display().to_string(),
        disk_usage_bytes: dir_size_bytes(db_path)?,
        totals: ChecksumTotals {
            column_family_count: snapshot.column_families.len(),
            entries: snapshot.totals.entries,
            key_bytes: snapshot.totals.key_bytes,
            value_bytes: snapshot.totals.value_bytes,
            sha256: snapshot
                .totals
                .sha256
                .expect("checksum mode always populates digest"),
        },
        column_families: snapshot
            .column_families
            .into_iter()
            .map(|(name, metrics)| {
                (
                    name,
                    ColumnFamilyChecksum {
                        entries: metrics.entries,
                        key_bytes: metrics.key_bytes,
                        value_bytes: metrics.value_bytes,
                        sha256: metrics
                            .sha256
                            .expect("checksum mode always populates column family digest"),
                    },
                )
            })
            .collect(),
    })
}

pub fn compare_checksum_reports(
    left_path: &Path,
    right_path: &Path,
) -> Result<CompareChecksumReport> {
    let left_bytes =
        fs::read(left_path).with_context(|| format!("failed to read {}", left_path.display()))?;
    let right_bytes =
        fs::read(right_path).with_context(|| format!("failed to read {}", right_path.display()))?;

    let left: ChecksumReport = serde_json::from_slice(&left_bytes)
        .with_context(|| format!("failed to parse {}", left_path.display()))?;
    let right: ChecksumReport = serde_json::from_slice(&right_bytes)
        .with_context(|| format!("failed to parse {}", right_path.display()))?;

    let left_names = left
        .column_families
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let right_names = right
        .column_families
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();

    let only_in_left = left_names
        .difference(&right_names)
        .cloned()
        .collect::<Vec<_>>();
    let only_in_right = right_names
        .difference(&left_names)
        .cloned()
        .collect::<Vec<_>>();

    let mut mismatches = Vec::new();

    for name in left_names.intersection(&right_names) {
        let left_cf = left
            .column_families
            .get(name)
            .expect("column family exists in left report");
        let right_cf = right
            .column_families
            .get(name)
            .expect("column family exists in right report");

        if left_cf.entries != right_cf.entries {
            mismatches.push(ChecksumMismatch {
                column_family: name.clone(),
                reason: "entry-count".to_owned(),
                left_entries: left_cf.entries,
                right_entries: right_cf.entries,
                left_sha256: left_cf.sha256.clone(),
                right_sha256: right_cf.sha256.clone(),
            });
        } else if left_cf.sha256 != right_cf.sha256 {
            mismatches.push(ChecksumMismatch {
                column_family: name.clone(),
                reason: "sha256".to_owned(),
                left_entries: left_cf.entries,
                right_entries: right_cf.entries,
                left_sha256: left_cf.sha256.clone(),
                right_sha256: right_cf.sha256.clone(),
            });
        }
    }

    Ok(CompareChecksumReport {
        matches: only_in_left.is_empty() && only_in_right.is_empty() && mismatches.is_empty(),
        left_path: left_path.display().to_string(),
        right_path: right_path.display().to_string(),
        only_in_left,
        only_in_right,
        mismatches,
    })
}

pub fn write_json_output<T: Serialize>(value: &T, output: Option<&Path>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize JSON output")?;

    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directories for {}", path.display())
            })?;
        }
        fs::write(path, &bytes).with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&bytes)
            .context("failed to write JSON to stdout")?;
        stdout.write_all(b"\n").context("failed to write newline")?;
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct InspectionSnapshot {
    column_families: BTreeMap<String, RawMetrics>,
    totals: RawTotals,
}

#[derive(Debug, Clone, Default)]
struct RawMetrics {
    entries: u64,
    key_bytes: u64,
    value_bytes: u64,
    sha256: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RawTotals {
    entries: u64,
    key_bytes: u64,
    value_bytes: u64,
    sha256: Option<String>,
}

fn inspect_backend(
    backend: BackendKind,
    db_path: &Path,
    include_checksum: bool,
) -> Result<InspectionSnapshot> {
    match backend {
        BackendKind::RocksDb | BackendKind::ToplingDb => inspect_rocksdb(db_path, include_checksum),
    }
}

fn inspect_rocksdb(db_path: &Path, include_checksum: bool) -> Result<InspectionSnapshot> {
    let mut opts = Options::default();
    opts.create_if_missing(false);

    let cf_names = DB::list_cf(&opts, db_path)
        .with_context(|| format!("failed to list column families in {}", db_path.display()))?;
    let db = DB::open_cf_for_read_only(&opts, db_path, cf_names.clone(), false)
        .with_context(|| format!("failed to open RocksDB at {}", db_path.display()))?;

    let mut column_families = BTreeMap::new();
    let mut overall_hasher = include_checksum.then(Sha256::new);
    let mut totals = RawTotals::default();

    for cf_name in cf_names {
        let handle = db
            .cf_handle(&cf_name)
            .with_context(|| format!("missing column family handle `{cf_name}`"))?;
        let iter = db.iterator_cf(&handle, IteratorMode::Start);

        let mut metrics = RawMetrics::default();
        let mut cf_hasher = include_checksum.then(Sha256::new);

        for row in iter {
            let (key, value) =
                row.with_context(|| format!("failed to iterate column family `{cf_name}`"))?;
            metrics.entries += 1;
            metrics.key_bytes += key.len() as u64;
            metrics.value_bytes += value.len() as u64;

            if let Some(hasher) = cf_hasher.as_mut() {
                update_hasher(hasher, &key, &value);
            }
        }

        if let Some(hasher) = cf_hasher {
            let digest = hex::encode(hasher.finalize());
            if let Some(overall) = overall_hasher.as_mut() {
                update_overall_digest(overall, &cf_name, &digest, metrics.entries);
            }
            metrics.sha256 = Some(digest);
        }

        totals.entries += metrics.entries;
        totals.key_bytes += metrics.key_bytes;
        totals.value_bytes += metrics.value_bytes;
        column_families.insert(cf_name, metrics);
    }

    if let Some(hasher) = overall_hasher {
        totals.sha256 = Some(hex::encode(hasher.finalize()));
    }

    Ok(InspectionSnapshot {
        column_families,
        totals,
    })
}

fn update_hasher(hasher: &mut Sha256, key: &[u8], value: &[u8]) {
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn update_overall_digest(hasher: &mut Sha256, cf_name: &str, cf_sha256: &str, entries: u64) {
    hasher.update((cf_name.len() as u64).to_be_bytes());
    hasher.update(cf_name.as_bytes());
    hasher.update(entries.to_be_bytes());
    hasher.update(cf_sha256.as_bytes());
}

fn dir_size_bytes(path: &Path) -> Result<u64> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }

    let mut total = 0u64;
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", path.display()))?;
        let child_path = entry.path();
        let child_metadata = entry
            .metadata()
            .with_context(|| format!("failed to stat {}", child_path.display()))?;
        if child_metadata.is_dir() {
            total += dir_size_bytes(&child_path)?;
        } else {
            total += child_metadata.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use hotstore_db::{cf::ColumnFamily, RocksDbBackend, StorageEngine};
    use tempfile::TempDir;

    use super::*;

    fn seed_db(path: &Path) -> Result<()> {
        let db = RocksDbBackend::open(path)?;
        db.put(ColumnFamily::Meta, b"dataset:name", b"sui-demo")?;
        db.put(ColumnFamily::Checkpoint, b"0001", b"checkpoint")?;
        db.put(ColumnFamily::TxByDigest, b"tx1", b"ok")?;
        Ok(())
    }

    #[test]
    fn stats_report_counts_entries() {
        let temp_dir = TempDir::new().expect("temp dir");
        seed_db(temp_dir.path()).expect("seed db");

        let report = compute_stats_report(BackendKind::RocksDb, temp_dir.path())
            .expect("compute stats report");

        assert!(report.disk_usage_bytes > 0);
        assert!(report.totals.entries >= 3);
        assert_eq!(
            report
                .column_families
                .get("cf_meta")
                .expect("meta cf exists")
                .entries,
            1
        );
    }

    #[test]
    fn checksum_compare_matches_identical_reports() {
        let left_dir = TempDir::new().expect("left temp dir");
        let right_dir = TempDir::new().expect("right temp dir");
        seed_db(left_dir.path()).expect("seed left db");
        seed_db(right_dir.path()).expect("seed right db");

        let left_report = compute_checksum_report(BackendKind::RocksDb, left_dir.path())
            .expect("left checksum report");
        let right_report = compute_checksum_report(BackendKind::RocksDb, right_dir.path())
            .expect("right checksum report");

        let left_path = left_dir.path().join("left.json");
        let right_path = right_dir.path().join("right.json");
        write_json_output(&left_report, Some(&left_path)).expect("write left report");
        write_json_output(&right_report, Some(&right_path)).expect("write right report");

        let diff = compare_checksum_reports(&left_path, &right_path).expect("compare checksums");
        assert!(diff.matches);
        assert!(diff.only_in_left.is_empty());
        assert!(diff.only_in_right.is_empty());
        assert!(diff.mismatches.is_empty());
    }
}
