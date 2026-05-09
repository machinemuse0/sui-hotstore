use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct BenchKeyBatch {
    pub tx_digests: Vec<String>,
    pub object_versions: Vec<String>,
    pub object_ids: Vec<String>,
    pub event_types: Vec<String>,
}

impl BenchKeyBatch {
    pub fn append(&mut self, mut other: Self) {
        self.tx_digests.append(&mut other.tx_digests);
        self.object_versions.append(&mut other.object_versions);
        self.object_ids.append(&mut other.object_ids);
        self.event_types.append(&mut other.event_types);
    }

    pub fn clear(&mut self) {
        self.tx_digests.clear();
        self.object_versions.clear();
        self.object_ids.clear();
        self.event_types.clear();
    }
}

#[derive(Debug, Clone)]
pub struct BenchKeySink {
    dir: PathBuf,
    network: String,
    first_checkpoint: u64,
    last_checkpoint: u64,
}

impl BenchKeySink {
    pub fn open(
        dir: impl Into<PathBuf>,
        network: impl Into<String>,
        first_checkpoint: u64,
        last_checkpoint: u64,
    ) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create bench keys dir {}", dir.display()))?;

        let sink = Self {
            dir,
            network: network.into(),
            first_checkpoint,
            last_checkpoint,
        };

        sink.touch_raw_files()?;
        Ok(sink)
    }

    pub fn read_progress(dir: &Path) -> Result<Option<BenchKeyProgress>> {
        let path = dir.join("progress.json");
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read bench key progress {}", path.display()))?;
        let progress = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to decode bench key progress {}", path.display()))?;
        Ok(Some(progress))
    }

    pub fn append_batch(&self, batch: &BenchKeyBatch, last_flushed_checkpoint: u64) -> Result<()> {
        append_lines(&self.tx_raw_path(), &batch.tx_digests)?;
        append_lines(&self.object_versions_raw_path(), &batch.object_versions)?;
        append_lines(&self.object_ids_raw_path(), &batch.object_ids)?;
        append_lines(&self.event_types_raw_path(), &batch.event_types)?;

        self.write_progress(BenchKeyProgress {
            network: self.network.clone(),
            first_checkpoint: self.first_checkpoint,
            last_checkpoint: self.last_checkpoint,
            last_flushed_checkpoint,
            completed: false,
        })
    }

    pub fn finalize(&self) -> Result<BenchKeyManifest> {
        let tx_count = sort_unique_lines(&self.tx_raw_path(), &self.tx_out_path())?;
        let object_version_count =
            sort_unique_lines(&self.object_versions_raw_path(), &self.object_versions_out_path())?;
        let object_id_count =
            sort_unique_lines(&self.object_ids_raw_path(), &self.object_ids_out_path())?;
        let event_type_count =
            sort_unique_lines(&self.event_types_raw_path(), &self.event_types_out_path())?;

        let last_flushed_checkpoint = Self::read_progress(&self.dir)?
            .map(|progress| progress.last_flushed_checkpoint)
            .unwrap_or(self.first_checkpoint.saturating_sub(1));

        self.write_progress(BenchKeyProgress {
            network: self.network.clone(),
            first_checkpoint: self.first_checkpoint,
            last_checkpoint: self.last_checkpoint,
            last_flushed_checkpoint,
            completed: true,
        })?;

        let manifest = BenchKeyManifest {
            network: self.network.clone(),
            first_checkpoint: self.first_checkpoint,
            last_checkpoint: self.last_checkpoint,
            generated_files: GeneratedFiles {
                tx_digests: self.tx_out_path().display().to_string(),
                object_versions: self.object_versions_out_path().display().to_string(),
                object_ids: self.object_ids_out_path().display().to_string(),
                event_types: self.event_types_out_path().display().to_string(),
            },
            counts: BenchKeyCounts {
                tx_digests: tx_count,
                object_versions: object_version_count,
                object_ids: object_id_count,
                event_types: event_type_count,
            },
        };

        write_json_atomic(&self.manifest_path(), &manifest)?;
        Ok(manifest)
    }

    fn touch_raw_files(&self) -> Result<()> {
        for path in [
            self.tx_raw_path(),
            self.object_versions_raw_path(),
            self.object_ids_raw_path(),
            self.event_types_raw_path(),
        ] {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| format!("failed to open raw bench key file {}", path.display()))?;
        }
        Ok(())
    }

    fn write_progress(&self, progress: BenchKeyProgress) -> Result<()> {
        write_json_atomic(&self.progress_path(), &progress)
    }

    fn tx_raw_path(&self) -> PathBuf {
        self.dir.join("tx_digests.raw")
    }

    fn object_versions_raw_path(&self) -> PathBuf {
        self.dir.join("object_versions.raw")
    }

    fn object_ids_raw_path(&self) -> PathBuf {
        self.dir.join("object_ids.raw")
    }

    fn event_types_raw_path(&self) -> PathBuf {
        self.dir.join("event_types.raw")
    }

    fn tx_out_path(&self) -> PathBuf {
        self.dir.join("tx_digests.txt")
    }

    fn object_versions_out_path(&self) -> PathBuf {
        self.dir.join("object_versions.txt")
    }

    fn object_ids_out_path(&self) -> PathBuf {
        self.dir.join("object_ids.txt")
    }

    fn event_types_out_path(&self) -> PathBuf {
        self.dir.join("event_types.txt")
    }

    fn progress_path(&self) -> PathBuf {
        self.dir.join("progress.json")
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchKeyProgress {
    pub network: String,
    pub first_checkpoint: u64,
    pub last_checkpoint: u64,
    pub last_flushed_checkpoint: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchKeyManifest {
    pub network: String,
    pub first_checkpoint: u64,
    pub last_checkpoint: u64,
    pub generated_files: GeneratedFiles,
    pub counts: BenchKeyCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedFiles {
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

fn append_lines(path: &Path, lines: &[String]) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;

    for line in lines {
        writeln!(file, "{line}")
            .with_context(|| format!("failed to append to {}", path.display()))?;
    }

    file.flush()
        .with_context(|| format!("failed to flush {}", path.display()))?;
    Ok(())
}

fn sort_unique_lines(raw_path: &Path, out_path: &Path) -> Result<usize> {
    let file = File::open(raw_path)
        .with_context(|| format!("failed to open raw bench key file {}", raw_path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = BTreeSet::new();

    for line in reader.lines() {
        let line = line.with_context(|| format!("failed to read {}", raw_path.display()))?;
        if !line.is_empty() {
            lines.insert(line);
        }
    }

    let temp_path = out_path.with_extension("txt.tmp");
    let mut out = File::create(&temp_path)
        .with_context(|| format!("failed to create {}", temp_path.display()))?;
    for line in &lines {
        writeln!(out, "{line}")
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
    }
    out.flush()
        .with_context(|| format!("failed to flush {}", temp_path.display()))?;

    fs::rename(&temp_path, out_path)
        .with_context(|| format!("failed to rename {} to {}", temp_path.display(), out_path.display()))?;
    Ok(lines.len())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    fs::write(&temp_path, bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to rename {} to {}", temp_path.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{BenchKeyBatch, BenchKeySink};

    #[test]
    fn finalize_deduplicates_and_persists_progress() {
        let dir = tempdir().unwrap();
        let sink = BenchKeySink::open(dir.path(), "testnet", 10, 20).unwrap();

        sink.append_batch(
            &BenchKeyBatch {
                tx_digests: vec!["tx1".into(), "tx1".into(), "tx2".into()],
                object_versions: vec!["obj1,1".into(), "obj1,1".into()],
                object_ids: vec!["obj1".into(), "obj1".into(), "obj2".into()],
                event_types: vec!["pkg::m::E".into(), "pkg::m::E".into()],
            },
            12,
        )
        .unwrap();

        let manifest = sink.finalize().unwrap();
        assert_eq!(manifest.counts.tx_digests, 2);
        assert_eq!(manifest.counts.object_versions, 1);
        assert_eq!(manifest.counts.object_ids, 2);
        assert_eq!(manifest.counts.event_types, 1);

        let progress = BenchKeySink::read_progress(dir.path()).unwrap().unwrap();
        assert!(progress.completed);
        assert_eq!(progress.last_flushed_checkpoint, 12);
    }
}
