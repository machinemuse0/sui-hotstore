use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use hotstore_core::{
    event_type_hash, key_object_last_seen, key_object_version, key_tx_by_digest, ColumnFamily,
};
use hotstore_db::{
    open_backend, toplingdb_backend::TOPLINGDB_EASY_MIGRATE_CONF_ENV, BackendKind, StorageEngine,
};
use sha2::{Digest, Sha256};

use crate::report::{
    BenchmarkMetadata, BenchmarkRunReport, BenchmarkSuiteReport, ErrorKindReport, FileFingerprint,
    LatencyReport,
};

#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub backend: BackendKind,
    pub db_path: PathBuf,
    pub workload: WorkloadKind,
    pub keys_path: Option<PathBuf>,
    pub tx_keys_path: Option<PathBuf>,
    pub object_version_keys_path: Option<PathBuf>,
    pub object_keys_path: Option<PathBuf>,
    pub event_types_path: Option<PathBuf>,
    pub checksum_report_path: Option<PathBuf>,
    pub requests: usize,
    pub warmup_requests: usize,
    pub concurrency: Vec<usize>,
    pub batch_size: usize,
    pub scan_limit: usize,
    pub dataset: Option<String>,
    pub access_pattern: AccessPattern,
    pub scan_mode: ScanMode,
    pub seed: u64,
    pub min_hit_rate: Option<f64>,
    pub min_requests_per_worker: usize,
    pub mixed_weights: MixedWeights,
    pub cache_state: CacheState,
    pub compact_before_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadKind {
    GetTx,
    GetObjectVersion,
    GetObjectLastSeen,
    MultiGetTx,
    MultiGetObjectVersion,
    ScanEvents,
    MixedRpc,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPattern {
    Sequential,
    Uniform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    Materialized,
    Count,
}

impl ScanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Materialized => "materialized",
            Self::Count => "count",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    Unknown,
    Hot,
    Cold,
}

impl CacheState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Hot => "hot",
            Self::Cold => "cold",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixedWeights {
    pub get_tx: u32,
    pub get_object_version: u32,
    pub multi_get_object_version: u32,
    pub scan_events: u32,
    pub get_object_last_seen: u32,
}

impl Default for MixedWeights {
    fn default() -> Self {
        Self {
            get_tx: 50,
            get_object_version: 20,
            multi_get_object_version: 15,
            scan_events: 10,
            get_object_last_seen: 5,
        }
    }
}

impl MixedWeights {
    fn total(self) -> u32 {
        self.get_tx
            + self.get_object_version
            + self.multi_get_object_version
            + self.scan_events
            + self.get_object_last_seen
    }

    pub fn profile_string(self) -> String {
        format!(
            "get-tx={},get-object-version={},multi-get-object-version={},scan-events={},get-object-last-seen={}",
            self.get_tx,
            self.get_object_version,
            self.multi_get_object_version,
            self.scan_events,
            self.get_object_last_seen
        )
    }
}

impl AccessPattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Uniform => "uniform",
        }
    }
}

impl WorkloadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GetTx => "get-tx",
            Self::GetObjectVersion => "get-object-version",
            Self::GetObjectLastSeen => "get-object-last-seen",
            Self::MultiGetTx => "multi-get-tx",
            Self::MultiGetObjectVersion => "multi-get-object-version",
            Self::ScanEvents => "scan-events",
            Self::MixedRpc => "mixed-rpc",
            Self::Noop => "noop",
        }
    }
}

pub fn parse_concurrency_list(input: &str) -> Result<Vec<usize>> {
    let values = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<usize>()
                .with_context(|| format!("invalid concurrency value `{part}`"))
        })
        .collect::<Result<Vec<_>>>()?;

    if values.is_empty() {
        bail!("concurrency list must contain at least one value");
    }
    if values.iter().any(|value| *value == 0) {
        bail!("concurrency values must be >= 1");
    }

    Ok(values)
}

pub fn parse_mixed_weights(input: &str) -> Result<MixedWeights> {
    let mut weights = MixedWeights::default();
    for part in input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (name, value) = part
            .split_once('=')
            .ok_or_else(|| anyhow!("invalid --mix item `{part}`; expected workload=weight"))?;
        let value = value
            .trim()
            .parse::<u32>()
            .with_context(|| format!("invalid --mix weight in `{part}`"))?;
        match name.trim() {
            "get-tx" => weights.get_tx = value,
            "get-object-version" => weights.get_object_version = value,
            "multi-get-object-version" => weights.multi_get_object_version = value,
            "scan-events" => weights.scan_events = value,
            "get-object-last-seen" => weights.get_object_last_seen = value,
            other => bail!("unsupported --mix workload `{other}`"),
        }
    }
    if weights.total() == 0 {
        bail!("--mix weights must sum to at least 1");
    }
    Ok(weights)
}

pub fn run_benchmark_suite(config: BenchmarkConfig) -> Result<BenchmarkSuiteReport> {
    if config.requests == 0 {
        bail!("--requests must be >= 1");
    }
    if config.batch_size == 0 {
        bail!("--batch-size must be >= 1");
    }
    if config.scan_limit == 0 {
        bail!("--scan-limit must be >= 1");
    }
    if config.concurrency.is_empty() {
        bail!("concurrency list must contain at least one value");
    }
    if config.concurrency.iter().any(|value| *value == 0) {
        bail!("concurrency values must be >= 1");
    }
    if let Some(min_hit_rate) = config.min_hit_rate {
        if !(0.0..=1.0).contains(&min_hit_rate) {
            bail!("--min-hit-rate must be between 0.0 and 1.0");
        }
    }
    if config.mixed_weights.total() == 0 {
        bail!("mixed workload weights must sum to at least 1");
    }

    let started_at_unix = unix_timestamp();
    let engine = open_backend(config.backend, &config.db_path)
        .with_context(|| format!("failed to open backend at {}", config.db_path.display()))?;
    if config.compact_before_run {
        engine.compact_all().with_context(|| {
            format!("failed to compact backend at {}", config.db_path.display())
        })?;
    }
    let workload_data = WorkloadData::load(&config)?;
    let key_files = key_file_fingerprints(&config)?;

    let mut runs = Vec::with_capacity(config.concurrency.len());
    for &concurrency in &config.concurrency {
        runs.push(run_single_benchmark(
            engine.clone(),
            workload_data.clone(),
            config.workload,
            config.requests,
            config.warmup_requests,
            concurrency,
            config.batch_size,
            config.scan_limit,
            config.scan_mode,
            config.access_pattern,
            config.seed,
            config.min_hit_rate,
            config.min_requests_per_worker,
        )?);
    }

    let finished_at_unix = unix_timestamp();
    Ok(BenchmarkSuiteReport {
        backend: config.backend.to_string(),
        db_path: config.db_path.display().to_string(),
        workload: config.workload.as_str().to_owned(),
        dataset: config.dataset,
        requests_per_run: config.requests,
        warmup_requests_per_run: config.warmup_requests,
        batch_size: config.batch_size,
        scan_mode: config.scan_mode.as_str().to_owned(),
        access_pattern: config.access_pattern.as_str().to_owned(),
        seed: config.seed,
        mixed_profile: (config.workload == WorkloadKind::MixedRpc)
            .then(|| config.mixed_weights.profile_string()),
        cache_state: config.cache_state.as_str().to_owned(),
        compact_before_run: config.compact_before_run,
        metadata: BenchmarkMetadata {
            started_at_unix,
            finished_at_unix,
            build_profile: build_profile().to_owned(),
            git_sha: option_env!("HOTSTORE_GIT_SHA").map(ToOwned::to_owned),
            git_dirty: option_env!("HOTSTORE_GIT_DIRTY").map(ToOwned::to_owned),
            rustc_version: option_env!("HOTSTORE_RUSTC_VERSION").map(ToOwned::to_owned),
            os: env::consts::OS.to_owned(),
            arch: env::consts::ARCH.to_owned(),
            hostname: hostname(),
            cpu_model: cpu_model(),
            total_memory_bytes: total_memory_bytes(),
            multi_get_impl: engine.multi_get_impl().to_owned(),
            cf_handle_mode: engine.cf_handle_mode().to_owned(),
            read_options_mode: engine.read_options_mode().to_owned(),
            toplingdb_config: toplingdb_config_fingerprint(),
            checksum_report: config
                .checksum_report_path
                .as_ref()
                .and_then(|path| fingerprint_file("checksum-report", path).ok()),
            key_files,
        },
        runs,
    })
}

fn run_single_benchmark(
    engine: Arc<dyn StorageEngine>,
    workload_data: Arc<WorkloadData>,
    workload: WorkloadKind,
    requests: usize,
    warmup_requests: usize,
    concurrency: usize,
    batch_size: usize,
    scan_limit: usize,
    scan_mode: ScanMode,
    access_pattern: AccessPattern,
    seed: u64,
    min_hit_rate: Option<f64>,
    min_requests_per_worker: usize,
) -> Result<BenchmarkRunReport> {
    if min_requests_per_worker > 0 && requests / concurrency < min_requests_per_worker {
        bail!(
            "requests per worker too low: requests={} concurrency={} min_requests_per_worker={}",
            requests,
            concurrency,
            min_requests_per_worker
        );
    }

    let mut handles = Vec::with_capacity(concurrency);
    let start_barrier = Arc::new(Barrier::new(concurrency + 1));
    let (ready_tx, ready_rx) = mpsc::channel();
    let base_requests = requests / concurrency;
    let extra_requests = requests % concurrency;
    let base_warmup_requests = warmup_requests / concurrency;
    let extra_warmup_requests = warmup_requests % concurrency;
    let mut next_start_index = 0usize;
    let mut next_warmup_start_index = requests;

    for worker_idx in 0..concurrency {
        let worker_requests = base_requests + usize::from(worker_idx < extra_requests);
        let worker_start_index = next_start_index;
        next_start_index += worker_requests;
        let worker_warmup_requests =
            base_warmup_requests + usize::from(worker_idx < extra_warmup_requests);
        let worker_warmup_start_index = next_warmup_start_index;
        next_warmup_start_index += worker_warmup_requests;
        let engine = engine.clone();
        let workload_data = workload_data.clone();
        let start_barrier = start_barrier.clone();
        let ready_tx = ready_tx.clone();
        let worker_seed = worker_seed(seed, worker_idx);

        handles.push(thread::spawn(move || {
            run_warmup(
                &engine,
                workload_data.as_ref(),
                workload,
                worker_warmup_start_index,
                worker_warmup_requests,
                batch_size,
                scan_limit,
                scan_mode,
                access_pattern,
                worker_seed,
            );
            let _ = ready_tx.send(());
            start_barrier.wait();
            run_worker(
                engine,
                workload_data,
                workload,
                worker_start_index,
                worker_requests,
                batch_size,
                scan_limit,
                scan_mode,
                access_pattern,
                worker_seed,
            )
        }));
    }
    drop(ready_tx);

    for _ in 0..concurrency {
        ready_rx
            .recv()
            .context("benchmark worker exited before measured run started")?;
    }

    let started_at = Instant::now();
    start_barrier.wait();
    let mut latencies_ns = Vec::with_capacity(requests);
    let mut hits = 0u64;
    let mut misses = 0u64;
    let mut errors = 0u64;
    let mut error_kinds = BTreeMap::new();
    let mut first_error = None;
    let mut records_returned_total = 0u64;
    let mut bytes_returned_total = 0u64;

    for handle in handles {
        let worker = handle
            .join()
            .map_err(|_| anyhow!("benchmark worker thread panicked"))??;
        latencies_ns.extend(worker.latencies_ns);
        hits += worker.hits;
        misses += worker.misses;
        errors += worker.errors;
        for (kind, count) in worker.error_kinds {
            *error_kinds.entry(kind).or_insert(0) += count;
        }
        if first_error.is_none() {
            first_error = worker.first_error;
        }
        records_returned_total += worker.records_returned_total;
        bytes_returned_total += worker.bytes_returned_total;
    }

    if latencies_ns.is_empty() {
        bail!("benchmark produced no latency samples");
    }

    latencies_ns.sort_unstable();
    let elapsed = started_at.elapsed();
    let elapsed_secs = elapsed.as_secs_f64().max(0.000_001);
    let hit_rate = if hits + misses == 0 {
        1.0
    } else {
        hits as f64 / (hits + misses) as f64
    };
    if let Some(min_hit_rate) = min_hit_rate {
        if hit_rate < min_hit_rate {
            bail!(
                "hit rate below threshold: hit_rate={:.6} min_hit_rate={:.6}",
                hit_rate,
                min_hit_rate
            );
        }
    }
    let error_kind_reports = error_kinds
        .into_iter()
        .map(|(kind, count)| ErrorKindReport { kind, count })
        .collect::<Vec<_>>();

    Ok(BenchmarkRunReport {
        concurrency,
        requests,
        elapsed_ms: elapsed.as_secs_f64() * 1_000.0,
        throughput_rps: requests as f64 / elapsed_secs,
        hits,
        misses,
        errors,
        error_kinds: error_kind_reports,
        first_error,
        hit_rate,
        records_returned_total,
        avg_records_per_request: records_returned_total as f64 / requests as f64,
        records_per_second: records_returned_total as f64 / elapsed_secs,
        bytes_returned_total,
        bytes_per_second: bytes_returned_total as f64 / elapsed_secs,
        latency_ms: LatencyReport {
            p50: percentile_ms(&latencies_ns, 0.50),
            p90: percentile_ms(&latencies_ns, 0.90),
            p95: percentile_ms(&latencies_ns, 0.95),
            p99: percentile_ms(&latencies_ns, 0.99),
            p999: percentile_ms(&latencies_ns, 0.999),
            max: *latencies_ns.last().expect("latencies not empty") as f64 / 1_000_000.0,
        },
        latency_ns: LatencyReport {
            p50: percentile_ns(&latencies_ns, 0.50) as f64,
            p90: percentile_ns(&latencies_ns, 0.90) as f64,
            p95: percentile_ns(&latencies_ns, 0.95) as f64,
            p99: percentile_ns(&latencies_ns, 0.99) as f64,
            p999: percentile_ns(&latencies_ns, 0.999) as f64,
            max: *latencies_ns.last().expect("latencies not empty") as f64,
        },
    })
}

fn run_worker(
    engine: Arc<dyn StorageEngine>,
    workload_data: Arc<WorkloadData>,
    workload: WorkloadKind,
    start_index: usize,
    requests: usize,
    batch_size: usize,
    scan_limit: usize,
    scan_mode: ScanMode,
    access_pattern: AccessPattern,
    seed: u64,
) -> Result<WorkerResult> {
    let mut result = WorkerResult {
        latencies_ns: Vec::with_capacity(requests),
        hits: 0,
        misses: 0,
        errors: 0,
        error_kinds: BTreeMap::new(),
        first_error: None,
        records_returned_total: 0,
        bytes_returned_total: 0,
    };
    let mut rng = SimpleRng::new(seed);

    for i in 0..requests {
        let request_index = start_index + i;
        let started_at = Instant::now();
        let op_result = execute_request(
            &*engine,
            workload_data.as_ref(),
            workload,
            request_index,
            batch_size,
            scan_limit,
            scan_mode,
            access_pattern,
            &mut rng,
        );
        let elapsed_ns = started_at.elapsed().as_nanos() as u64;
        result.latencies_ns.push(elapsed_ns);

        match op_result {
            Ok(outcome) => {
                result.hits += outcome.hits;
                result.misses += outcome.misses;
                result.records_returned_total += outcome.records_returned_total;
                result.bytes_returned_total += outcome.bytes_returned_total;
            }
            Err(error) => {
                result.errors += 1;
                let kind = classify_error(&error);
                *result.error_kinds.entry(kind).or_insert(0) += 1;
                if result.first_error.is_none() {
                    result.first_error = Some(format_error(&error));
                }
            }
        }
    }

    Ok(result)
}

fn run_warmup(
    engine: &Arc<dyn StorageEngine>,
    workload_data: &WorkloadData,
    workload: WorkloadKind,
    start_index: usize,
    requests: usize,
    batch_size: usize,
    scan_limit: usize,
    scan_mode: ScanMode,
    access_pattern: AccessPattern,
    seed: u64,
) {
    let mut rng = SimpleRng::new(seed ^ 0x9e37_79b9_7f4a_7c15);
    for i in 0..requests {
        let _ = execute_request(
            &**engine,
            workload_data,
            workload,
            start_index + i,
            batch_size,
            scan_limit,
            scan_mode,
            access_pattern,
            &mut rng,
        );
    }
}

fn execute_request(
    engine: &dyn StorageEngine,
    workload_data: &WorkloadData,
    workload: WorkloadKind,
    request_index: usize,
    batch_size: usize,
    scan_limit: usize,
    scan_mode: ScanMode,
    access_pattern: AccessPattern,
    rng: &mut SimpleRng,
) -> Result<OperationOutcome> {
    match workload {
        WorkloadKind::Noop => Ok(OperationOutcome::default()),
        WorkloadKind::GetTx => {
            let key = workload_data.key_at(request_index, access_pattern, rng)?;
            point_get(engine, ColumnFamily::TxByDigest, key)
        }
        WorkloadKind::GetObjectVersion => {
            let key = workload_data.key_at(request_index, access_pattern, rng)?;
            point_get(engine, ColumnFamily::ObjectVersion, key)
        }
        WorkloadKind::GetObjectLastSeen => {
            let key = workload_data.key_at(request_index, access_pattern, rng)?;
            point_get(engine, ColumnFamily::ObjectLastSeen, key)
        }
        WorkloadKind::MultiGetTx => {
            let keys = workload_data.batch_at(request_index, batch_size, access_pattern, rng)?;
            multi_get(engine, ColumnFamily::TxByDigest, keys.as_slice())
        }
        WorkloadKind::MultiGetObjectVersion => {
            let keys = workload_data.batch_at(request_index, batch_size, access_pattern, rng)?;
            multi_get(engine, ColumnFamily::ObjectVersion, keys.as_slice())
        }
        WorkloadKind::ScanEvents => {
            let prefix = workload_data.prefix_at(request_index, access_pattern, rng)?;
            scan_prefix(
                engine,
                ColumnFamily::EventByType,
                prefix,
                scan_limit,
                scan_mode,
            )
        }
        WorkloadKind::MixedRpc => mixed_rpc(
            engine,
            workload_data.mixed_request(
                request_index,
                batch_size,
                scan_limit,
                access_pattern,
                rng,
            )?,
            scan_mode,
        ),
    }
}

fn point_get(engine: &dyn StorageEngine, cf: ColumnFamily, key: &[u8]) -> Result<OperationOutcome> {
    let mut hit = false;
    let mut bytes_returned_total = 0u64;
    engine.get_pinned_with(cf, key, &mut |value| {
        if let Some(value) = value {
            hit = true;
            bytes_returned_total = value.len() as u64;
        }
    })?;
    let hits = u64::from(hit);
    Ok(OperationOutcome {
        hits,
        misses: 1 - hits,
        records_returned_total: hits,
        bytes_returned_total,
    })
}

fn multi_get(
    engine: &dyn StorageEngine,
    cf: ColumnFamily,
    keys: &[&[u8]],
) -> Result<OperationOutcome> {
    let total = keys.len() as u64;
    let mut hits = 0u64;
    let mut bytes_returned_total = 0u64;
    engine.multi_get_pinned_with(cf, keys, &mut |_idx, value| {
        if let Some(value) = value {
            hits += 1;
            bytes_returned_total += value.len() as u64;
        }
    })?;
    Ok(OperationOutcome {
        hits,
        misses: total - hits,
        records_returned_total: hits,
        bytes_returned_total,
    })
}

fn scan_prefix(
    engine: &dyn StorageEngine,
    cf: ColumnFamily,
    prefix: &[u8],
    limit: usize,
    scan_mode: ScanMode,
) -> Result<OperationOutcome> {
    match scan_mode {
        ScanMode::Materialized => {
            let rows = engine.scan_prefix(cf, prefix, limit)?;
            let bytes_returned_total = rows
                .iter()
                .map(|(key, value)| key.len() as u64 + value.len() as u64)
                .sum();
            Ok(OperationOutcome {
                hits: u64::from(!rows.is_empty()),
                misses: u64::from(rows.is_empty()),
                records_returned_total: rows.len() as u64,
                bytes_returned_total,
            })
        }
        ScanMode::Count => {
            let outcome = engine.scan_prefix_count(cf, prefix, limit)?;
            Ok(OperationOutcome {
                hits: u64::from(outcome.rows > 0),
                misses: u64::from(outcome.rows == 0),
                records_returned_total: outcome.rows as u64,
                bytes_returned_total: (outcome.key_bytes + outcome.value_bytes) as u64,
            })
        }
    }
}

fn mixed_rpc(
    engine: &dyn StorageEngine,
    request: MixedRequest<'_>,
    scan_mode: ScanMode,
) -> Result<OperationOutcome> {
    match request {
        MixedRequest::GetTx(key) => point_get(engine, ColumnFamily::TxByDigest, key),
        MixedRequest::GetObjectVersion(key) => point_get(engine, ColumnFamily::ObjectVersion, key),
        MixedRequest::GetObjectLastSeen(key) => {
            point_get(engine, ColumnFamily::ObjectLastSeen, key)
        }
        MixedRequest::MultiGetObjectVersion(keys) => {
            multi_get(engine, ColumnFamily::ObjectVersion, keys.as_slice())
        }
        MixedRequest::ScanEvents(prefix, limit) => {
            scan_prefix(engine, ColumnFamily::EventByType, prefix, limit, scan_mode)
        }
    }
}

fn percentile_ns(latencies_ns: &[u64], quantile: f64) -> u64 {
    let index = ((latencies_ns.len() - 1) as f64 * quantile).round() as usize;
    latencies_ns[index]
}

fn percentile_ms(latencies_ns: &[u64], quantile: f64) -> f64 {
    percentile_ns(latencies_ns, quantile) as f64 / 1_000_000.0
}

fn classify_error(error: &anyhow::Error) -> String {
    let message = error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());
    message
        .split(':')
        .next()
        .unwrap_or("unknown")
        .trim()
        .chars()
        .take(120)
        .collect()
}

fn format_error(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

fn select_index(
    len: usize,
    sequence_index: usize,
    access_pattern: AccessPattern,
    rng: &mut SimpleRng,
) -> usize {
    match access_pattern {
        AccessPattern::Sequential => sequence_index % len,
        AccessPattern::Uniform => rng.next_usize(len),
    }
}

fn worker_seed(seed: u64, worker_idx: usize) -> u64 {
    seed ^ splitmix64(worker_idx as u64 + 0x517c_c1b7_2722_0a95)
}

#[derive(Debug, Clone)]
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        splitmix64(self.state)
    }

    fn next_usize(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        (self.next_u64() % upper as u64) as usize
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn key_file_fingerprints(config: &BenchmarkConfig) -> Result<Vec<FileFingerprint>> {
    let mut files = Vec::new();
    match config.workload {
        WorkloadKind::GetTx | WorkloadKind::GetObjectLastSeen | WorkloadKind::MultiGetTx => {
            files.push(fingerprint_file(
                "keys",
                required_path(config.keys_path.as_ref(), "--keys")?,
            )?);
        }
        WorkloadKind::GetObjectVersion | WorkloadKind::MultiGetObjectVersion => {
            files.push(fingerprint_file(
                "keys",
                required_path(config.keys_path.as_ref(), "--keys")?,
            )?);
        }
        WorkloadKind::ScanEvents => {
            let path = config
                .event_types_path
                .as_ref()
                .or(config.keys_path.as_ref())
                .ok_or_else(|| anyhow!("scan-events requires --event-types or --keys"))?;
            files.push(fingerprint_file("event-types", path)?);
        }
        WorkloadKind::Noop => {}
        WorkloadKind::MixedRpc => {
            files.push(fingerprint_file(
                "tx-keys",
                required_path(config.tx_keys_path.as_ref(), "--tx-keys")?,
            )?);
            files.push(fingerprint_file(
                "object-version-keys",
                required_path(
                    config.object_version_keys_path.as_ref(),
                    "--object-version-keys",
                )?,
            )?);
            files.push(fingerprint_file(
                "object-keys",
                required_path(config.object_keys_path.as_ref(), "--object-keys")?,
            )?);
            files.push(fingerprint_file(
                "event-types",
                required_path(config.event_types_path.as_ref(), "--event-types")?,
            )?);
        }
    }
    Ok(files)
}

fn toplingdb_config_fingerprint() -> Option<FileFingerprint> {
    env::var_os(TOPLINGDB_EASY_MIGRATE_CONF_ENV)
        .map(PathBuf::from)
        .and_then(|path| fingerprint_file(TOPLINGDB_EASY_MIGRATE_CONF_ENV, &path).ok())
}

fn fingerprint_file(label: &str, path: &Path) -> Result<FileFingerprint> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to fingerprint file {}", path.display()))?;
    let sha256 = Sha256::digest(&bytes);
    let usable_lines = String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    Ok(FileFingerprint {
        label: label.to_owned(),
        path: path.display().to_string(),
        sha256: format!("{sha256:x}"),
        usable_lines,
    })
}

fn hostname() -> Option<String> {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| env::var("HOSTNAME").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn cpu_model() -> Option<String> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("model name").and_then(|rest| {
                rest.split_once(':')
                    .map(|(_, value)| value.trim().to_owned())
            })
        })
        .filter(|value| !value.is_empty())
}

fn total_memory_bytes() -> Option<u64> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    text.lines().find_map(|line| {
        let rest = line.strip_prefix("MemTotal:")?;
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(kb * 1024)
    })
}

#[derive(Debug, Clone)]
enum WorkloadData {
    FlatKeys(Vec<Vec<u8>>),
    EventPrefixes(Vec<Vec<u8>>),
    Mixed(MixedWorkloadData),
    Noop,
}

impl WorkloadData {
    fn load(config: &BenchmarkConfig) -> Result<Arc<Self>> {
        match config.workload {
            WorkloadKind::GetTx | WorkloadKind::GetObjectLastSeen | WorkloadKind::MultiGetTx => {
                let path = required_path(config.keys_path.as_ref(), "--keys")?;
                Ok(Arc::new(Self::FlatKeys(load_flat_keys(
                    path,
                    config.workload,
                )?)))
            }
            WorkloadKind::GetObjectVersion | WorkloadKind::MultiGetObjectVersion => {
                let path = required_path(config.keys_path.as_ref(), "--keys")?;
                Ok(Arc::new(Self::FlatKeys(load_object_version_keys(path)?)))
            }
            WorkloadKind::ScanEvents => {
                let path = config
                    .event_types_path
                    .as_ref()
                    .or(config.keys_path.as_ref())
                    .ok_or_else(|| anyhow!("scan-events requires --event-types or --keys"))?;
                Ok(Arc::new(Self::EventPrefixes(load_event_type_prefixes(
                    path,
                )?)))
            }
            WorkloadKind::Noop => Ok(Arc::new(Self::Noop)),
            WorkloadKind::MixedRpc => Ok(Arc::new(Self::Mixed(MixedWorkloadData {
                weights: config.mixed_weights,
                tx_keys: load_flat_keys(
                    required_path(config.tx_keys_path.as_ref(), "--tx-keys")?,
                    WorkloadKind::GetTx,
                )?,
                object_version_keys: load_object_version_keys(required_path(
                    config.object_version_keys_path.as_ref(),
                    "--object-version-keys",
                )?)?,
                object_last_seen_keys: load_flat_keys(
                    required_path(config.object_keys_path.as_ref(), "--object-keys")?,
                    WorkloadKind::GetObjectLastSeen,
                )?,
                event_type_prefixes: load_event_type_prefixes(required_path(
                    config.event_types_path.as_ref(),
                    "--event-types",
                )?)?,
            }))),
        }
    }

    fn key_at(
        &self,
        index: usize,
        access_pattern: AccessPattern,
        rng: &mut SimpleRng,
    ) -> Result<&[u8]> {
        match self {
            Self::FlatKeys(keys) => {
                Ok(keys[select_index(keys.len(), index, access_pattern, rng)].as_slice())
            }
            Self::EventPrefixes(_) => bail!("requested point key from event-prefix workload data"),
            Self::Mixed(_) => bail!("requested point key from mixed workload data"),
            Self::Noop => bail!("requested point key from noop workload data"),
        }
    }

    fn batch_at(
        &self,
        start_index: usize,
        batch_size: usize,
        access_pattern: AccessPattern,
        rng: &mut SimpleRng,
    ) -> Result<Vec<&[u8]>> {
        match self {
            Self::FlatKeys(keys) => {
                let mut batch = Vec::with_capacity(batch_size);
                for offset in 0..batch_size {
                    let index = match access_pattern {
                        AccessPattern::Sequential => start_index * batch_size + offset,
                        AccessPattern::Uniform => rng.next_usize(keys.len()),
                    };
                    batch.push(keys[index % keys.len()].as_slice());
                }
                Ok(batch)
            }
            Self::EventPrefixes(_) => bail!("requested key batch from event-prefix workload data"),
            Self::Mixed(_) => bail!("requested key batch from mixed workload data"),
            Self::Noop => bail!("requested key batch from noop workload data"),
        }
    }

    fn prefix_at(
        &self,
        index: usize,
        access_pattern: AccessPattern,
        rng: &mut SimpleRng,
    ) -> Result<&[u8]> {
        match self {
            Self::EventPrefixes(prefixes) => {
                Ok(prefixes[select_index(prefixes.len(), index, access_pattern, rng)].as_slice())
            }
            Self::FlatKeys(_) => bail!("requested scan prefix from flat-key workload data"),
            Self::Mixed(_) => bail!("requested scan prefix from mixed workload data"),
            Self::Noop => bail!("requested scan prefix from noop workload data"),
        }
    }

    fn mixed_request(
        &self,
        index: usize,
        batch_size: usize,
        scan_limit: usize,
        access_pattern: AccessPattern,
        rng: &mut SimpleRng,
    ) -> Result<MixedRequest<'_>> {
        match self {
            Self::Mixed(data) => {
                Ok(data.request_at(index, batch_size, scan_limit, access_pattern, rng))
            }
            Self::FlatKeys(_) | Self::EventPrefixes(_) | Self::Noop => {
                bail!("requested mixed-rpc data from non-mixed workload data")
            }
        }
    }
}

fn parse_object_version_line(line: &str) -> Result<Vec<u8>> {
    let (object_id, version) = line.split_once(',').ok_or_else(|| {
        anyhow!("invalid object version key line `{line}`; expected object_id,version")
    })?;
    let version = version
        .trim()
        .parse::<u64>()
        .with_context(|| format!("invalid object version in line `{line}`"))?;
    Ok(key_object_version(object_id.trim().as_bytes(), version))
}

fn required_path<'a>(path: Option<&'a PathBuf>, flag: &str) -> Result<&'a PathBuf> {
    path.ok_or_else(|| anyhow!("workload requires {flag}"))
}

fn load_utf8_lines(path: &PathBuf) -> Result<Vec<String>> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read key file {}", path.display()))?;
    let text = String::from_utf8(bytes)
        .with_context(|| format!("key file is not valid UTF-8: {}", path.display()))?;
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        bail!(
            "key file {} did not contain any usable keys",
            path.display()
        );
    }
    Ok(lines)
}

fn load_flat_keys(path: &PathBuf, workload: WorkloadKind) -> Result<Vec<Vec<u8>>> {
    load_utf8_lines(path)?
        .into_iter()
        .map(|line| match workload {
            WorkloadKind::GetTx | WorkloadKind::MultiGetTx => Ok(key_tx_by_digest(line.as_bytes())),
            WorkloadKind::GetObjectLastSeen => Ok(key_object_last_seen(line.as_bytes())),
            other => bail!("load_flat_keys does not support {}", other.as_str()),
        })
        .collect()
}

fn load_object_version_keys(path: &PathBuf) -> Result<Vec<Vec<u8>>> {
    load_utf8_lines(path)?
        .into_iter()
        .map(|line| parse_object_version_line(&line))
        .collect()
}

fn load_event_type_prefixes(path: &PathBuf) -> Result<Vec<Vec<u8>>> {
    load_utf8_lines(path)?
        .into_iter()
        .map(|event_type| Ok(event_type_hash(&event_type).to_vec()))
        .collect()
}

#[derive(Debug, Clone)]
struct MixedWorkloadData {
    weights: MixedWeights,
    tx_keys: Vec<Vec<u8>>,
    object_version_keys: Vec<Vec<u8>>,
    object_last_seen_keys: Vec<Vec<u8>>,
    event_type_prefixes: Vec<Vec<u8>>,
}

impl MixedWorkloadData {
    fn request_at(
        &self,
        index: usize,
        batch_size: usize,
        scan_limit: usize,
        access_pattern: AccessPattern,
        rng: &mut SimpleRng,
    ) -> MixedRequest<'_> {
        let total = self.weights.total();
        let slot = match access_pattern {
            AccessPattern::Sequential => (index as u32) % total,
            AccessPattern::Uniform => rng.next_usize(total as usize) as u32,
        };
        let get_tx_end = self.weights.get_tx;
        let get_object_version_end = get_tx_end + self.weights.get_object_version;
        let multi_get_object_version_end =
            get_object_version_end + self.weights.multi_get_object_version;
        let scan_events_end = multi_get_object_version_end + self.weights.scan_events;

        if slot < get_tx_end {
            MixedRequest::GetTx(
                self.tx_keys[select_index(self.tx_keys.len(), index, access_pattern, rng)]
                    .as_slice(),
            )
        } else if slot < get_object_version_end {
            MixedRequest::GetObjectVersion(
                self.object_version_keys
                    [select_index(self.object_version_keys.len(), index, access_pattern, rng)]
                .as_slice(),
            )
        } else if slot < multi_get_object_version_end {
            let mut keys = Vec::with_capacity(batch_size);
            for offset in 0..batch_size {
                let key_index = match access_pattern {
                    AccessPattern::Sequential => index * batch_size + offset,
                    AccessPattern::Uniform => rng.next_usize(self.object_version_keys.len()),
                };
                keys.push(
                    self.object_version_keys[key_index % self.object_version_keys.len()].as_slice(),
                );
            }
            MixedRequest::MultiGetObjectVersion(keys)
        } else if slot < scan_events_end {
            MixedRequest::ScanEvents(
                self.event_type_prefixes
                    [select_index(self.event_type_prefixes.len(), index, access_pattern, rng)]
                .as_slice(),
                scan_limit,
            )
        } else {
            MixedRequest::GetObjectLastSeen(
                self.object_last_seen_keys
                    [select_index(self.object_last_seen_keys.len(), index, access_pattern, rng)]
                .as_slice(),
            )
        }
    }
}

#[derive(Debug, Clone)]
enum MixedRequest<'a> {
    GetTx(&'a [u8]),
    GetObjectVersion(&'a [u8]),
    GetObjectLastSeen(&'a [u8]),
    MultiGetObjectVersion(Vec<&'a [u8]>),
    ScanEvents(&'a [u8], usize),
}

#[derive(Debug, Default)]
struct OperationOutcome {
    hits: u64,
    misses: u64,
    records_returned_total: u64,
    bytes_returned_total: u64,
}

#[derive(Debug, Default)]
struct WorkerResult {
    latencies_ns: Vec<u64>,
    hits: u64,
    misses: u64,
    errors: u64,
    error_kinds: BTreeMap<String, u64>,
    first_error: Option<String>,
    records_returned_total: u64,
    bytes_returned_total: u64,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use hotstore_db::{cf::ColumnFamily, RocksDbBackend, StorageEngine};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn parse_concurrency_rejects_zero() {
        assert!(parse_concurrency_list("1,0,4").is_err());
    }

    #[test]
    fn parse_object_version_line_builds_encoded_key() {
        let key = parse_object_version_line("object-1,7").expect("parse object version line");
        assert_eq!(key, key_object_version(b"object-1", 7));
    }

    #[test]
    fn parse_mixed_weights_overrides_selected_weights() {
        let weights = parse_mixed_weights("get-tx=10,scan-events=90").expect("parse mix");
        assert_eq!(weights.get_tx, 10);
        assert_eq!(weights.scan_events, 90);
        assert_eq!(weights.get_object_version, 20);
        assert!(parse_mixed_weights("get-tx=0,get-object-version=0,multi-get-object-version=0,scan-events=0,get-object-last-seen=0").is_err());
    }

    #[test]
    fn benchmark_suite_runs_noop_without_keys() {
        let temp_dir = TempDir::new().expect("temp dir");
        RocksDbBackend::open(temp_dir.path()).expect("open db");

        let report = run_benchmark_suite(BenchmarkConfig {
            backend: BackendKind::RocksDb,
            db_path: temp_dir.path().to_path_buf(),
            workload: WorkloadKind::Noop,
            keys_path: None,
            tx_keys_path: None,
            object_version_keys_path: None,
            object_keys_path: None,
            event_types_path: None,
            checksum_report_path: None,
            requests: 10,
            warmup_requests: 1,
            concurrency: vec![1],
            batch_size: 2,
            scan_limit: 10,
            scan_mode: ScanMode::Materialized,
            dataset: None,
            access_pattern: AccessPattern::Sequential,
            seed: 7,
            min_hit_rate: Some(1.0),
            min_requests_per_worker: 1,
            mixed_weights: MixedWeights::default(),
            cache_state: CacheState::Unknown,
            compact_before_run: false,
        })
        .expect("run noop benchmark suite");

        assert_eq!(report.workload, "noop");
        assert_eq!(report.runs[0].errors, 0);
        assert_eq!(report.runs[0].hits, 0);
        assert_eq!(report.runs[0].misses, 0);
        assert_eq!(report.runs[0].hit_rate, 1.0);
    }

    #[test]
    fn benchmark_suite_runs_get_tx_on_seeded_db() {
        let temp_dir = TempDir::new().expect("temp dir");
        {
            let db = RocksDbBackend::open(temp_dir.path()).expect("open db");
            db.put(ColumnFamily::TxByDigest, b"tx-1", b"value-1")
                .expect("seed tx-1");
            db.put(ColumnFamily::TxByDigest, b"tx-2", b"value-2")
                .expect("seed tx-2");
        }

        let keys_path = temp_dir.path().join("tx_keys.txt");
        fs::write(&keys_path, "tx-1\ntx-2\n").expect("write keys file");

        let report = run_benchmark_suite(BenchmarkConfig {
            backend: BackendKind::RocksDb,
            db_path: temp_dir.path().to_path_buf(),
            workload: WorkloadKind::GetTx,
            keys_path: Some(keys_path),
            tx_keys_path: None,
            object_version_keys_path: None,
            object_keys_path: None,
            event_types_path: None,
            checksum_report_path: None,
            requests: 20,
            warmup_requests: 2,
            concurrency: vec![1, 2],
            batch_size: 2,
            scan_limit: 10,
            scan_mode: ScanMode::Materialized,
            dataset: Some("test".to_owned()),
            access_pattern: AccessPattern::Sequential,
            seed: 7,
            min_hit_rate: Some(1.0),
            min_requests_per_worker: 1,
            mixed_weights: MixedWeights::default(),
            cache_state: CacheState::Unknown,
            compact_before_run: false,
        })
        .expect("run benchmark suite");

        assert_eq!(report.runs.len(), 2);
        assert_eq!(report.runs[0].requests, 20);
        assert_eq!(report.runs[0].errors, 0);
        assert_eq!(report.runs[0].error_kinds.len(), 0);
        assert_eq!(report.runs[0].hit_rate, 1.0);
        assert!(report.runs[0].hits > 0);
        assert!(report.runs[0].latency_ns.max >= report.runs[0].latency_ns.p50);
    }

    #[test]
    fn benchmark_suite_runs_scan_events_on_seeded_db() {
        let temp_dir = TempDir::new().expect("temp dir");
        {
            let db = RocksDbBackend::open(temp_dir.path()).expect("open db");
            db.put(
                ColumnFamily::EventByType,
                &hotstore_core::key_event_by_type("type::A", 1, 0, 0),
                b"event-1",
            )
            .expect("seed event-1");
            db.put(
                ColumnFamily::EventByType,
                &hotstore_core::key_event_by_type("type::A", 2, 0, 0),
                b"event-2",
            )
            .expect("seed event-2");
        }

        let event_types = temp_dir.path().join("event_types.txt");
        fs::write(&event_types, "type::A\n").expect("write event types");

        let report = run_benchmark_suite(BenchmarkConfig {
            backend: BackendKind::RocksDb,
            db_path: temp_dir.path().to_path_buf(),
            workload: WorkloadKind::ScanEvents,
            keys_path: None,
            tx_keys_path: None,
            object_version_keys_path: None,
            object_keys_path: None,
            event_types_path: Some(event_types),
            checksum_report_path: None,
            requests: 6,
            warmup_requests: 1,
            concurrency: vec![1],
            batch_size: 2,
            scan_limit: 10,
            scan_mode: ScanMode::Count,
            dataset: None,
            access_pattern: AccessPattern::Sequential,
            seed: 7,
            min_hit_rate: Some(1.0),
            min_requests_per_worker: 1,
            mixed_weights: MixedWeights::default(),
            cache_state: CacheState::Unknown,
            compact_before_run: false,
        })
        .expect("run scan benchmark suite");

        assert_eq!(report.runs[0].errors, 0);
        assert_eq!(report.runs[0].hit_rate, 1.0);
        assert_eq!(report.runs[0].records_returned_total, 12);
        assert_eq!(report.scan_mode, "count");
        assert!(report.runs[0].bytes_returned_total > 0);
    }

    #[test]
    fn benchmark_suite_runs_mixed_rpc_on_seeded_db() {
        let temp_dir = TempDir::new().expect("temp dir");
        {
            let db = RocksDbBackend::open(temp_dir.path()).expect("open db");
            db.put(ColumnFamily::TxByDigest, b"tx-1", b"tx-value")
                .expect("seed tx");
            db.put(
                ColumnFamily::ObjectVersion,
                &key_object_version(b"object-1", 7),
                b"object-version",
            )
            .expect("seed object version");
            db.put(
                ColumnFamily::ObjectLastSeen,
                &key_object_last_seen(b"object-1"),
                b"object-last-seen",
            )
            .expect("seed object last seen");
            db.put(
                ColumnFamily::EventByType,
                &hotstore_core::key_event_by_type("type::B", 3, 0, 0),
                b"event-value",
            )
            .expect("seed event");
        }

        let tx_keys = temp_dir.path().join("tx_keys.txt");
        let object_version_keys = temp_dir.path().join("object_versions.txt");
        let object_keys = temp_dir.path().join("object_keys.txt");
        let event_types = temp_dir.path().join("event_types.txt");
        fs::write(&tx_keys, "tx-1\n").expect("write tx keys");
        fs::write(&object_version_keys, "object-1,7\n").expect("write object version keys");
        fs::write(&object_keys, "object-1\n").expect("write object keys");
        fs::write(&event_types, "type::B\n").expect("write event types");

        let report = run_benchmark_suite(BenchmarkConfig {
            backend: BackendKind::RocksDb,
            db_path: temp_dir.path().to_path_buf(),
            workload: WorkloadKind::MixedRpc,
            keys_path: None,
            tx_keys_path: Some(tx_keys),
            object_version_keys_path: Some(object_version_keys),
            object_keys_path: Some(object_keys),
            event_types_path: Some(event_types),
            checksum_report_path: None,
            requests: 25,
            warmup_requests: 2,
            concurrency: vec![1],
            batch_size: 3,
            scan_limit: 5,
            scan_mode: ScanMode::Materialized,
            dataset: Some("mixed-test".to_owned()),
            access_pattern: AccessPattern::Sequential,
            seed: 7,
            min_hit_rate: Some(1.0),
            min_requests_per_worker: 1,
            mixed_weights: MixedWeights::default(),
            cache_state: CacheState::Unknown,
            compact_before_run: false,
        })
        .expect("run mixed benchmark suite");

        assert_eq!(report.runs[0].errors, 0);
        assert!(report.runs[0].hits > 0);
        assert!(report.runs[0].records_returned_total > 0);
    }
}
