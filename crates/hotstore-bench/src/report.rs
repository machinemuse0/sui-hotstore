use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkSuiteReport {
    pub backend: String,
    pub db_path: String,
    pub workload: String,
    pub dataset: Option<String>,
    pub requests_per_run: usize,
    pub warmup_requests_per_run: usize,
    pub batch_size: usize,
    pub scan_mode: String,
    pub access_pattern: String,
    pub seed: u64,
    pub mixed_profile: Option<String>,
    pub cache_state: String,
    pub compact_before_run: bool,
    pub metadata: BenchmarkMetadata,
    pub runs: Vec<BenchmarkRunReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkMetadata {
    pub started_at_unix: u64,
    pub finished_at_unix: u64,
    pub build_profile: String,
    pub git_sha: Option<String>,
    pub git_dirty: Option<String>,
    pub rustc_version: Option<String>,
    pub os: String,
    pub arch: String,
    pub hostname: Option<String>,
    pub cpu_model: Option<String>,
    pub total_memory_bytes: Option<u64>,
    pub multi_get_impl: String,
    pub cf_handle_mode: String,
    pub toplingdb_config: Option<FileFingerprint>,
    pub checksum_report: Option<FileFingerprint>,
    pub key_files: Vec<FileFingerprint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileFingerprint {
    pub label: String,
    pub path: String,
    pub sha256: String,
    pub usable_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchmarkRunReport {
    pub concurrency: usize,
    pub requests: usize,
    pub elapsed_ms: f64,
    pub throughput_rps: f64,
    pub hits: u64,
    pub misses: u64,
    pub errors: u64,
    pub error_kinds: Vec<ErrorKindReport>,
    pub first_error: Option<String>,
    pub hit_rate: f64,
    pub records_returned_total: u64,
    pub avg_records_per_request: f64,
    pub records_per_second: f64,
    pub bytes_returned_total: u64,
    pub bytes_per_second: f64,
    pub latency_ms: LatencyReport,
    pub latency_ns: LatencyReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorKindReport {
    pub kind: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyReport {
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    pub max: f64,
}

pub fn write_json_output<T: Serialize>(value: &T, output: Option<&Path>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize benchmark JSON")?;
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create benchmark output directory {}",
                    parent.display()
                )
            })?;
        }
        fs::write(path, &bytes)
            .with_context(|| format!("failed to write benchmark report to {}", path.display()))?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&bytes)
            .context("failed to write benchmark report to stdout")?;
        stdout
            .write_all(b"\n")
            .context("failed to write trailing newline")?;
    }
    Ok(())
}
