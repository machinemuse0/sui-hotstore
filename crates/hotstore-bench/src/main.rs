mod report;
mod workloads;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use hotstore_db::BackendKind;

use crate::report::write_json_output;
use crate::workloads::{
    parse_concurrency_list, parse_mixed_weights, run_benchmark_suite, AccessPattern,
    BenchmarkConfig, CacheState, MixedWeights, ScanMode, WorkloadKind,
};

#[derive(Debug, Parser)]
#[command(name = "hotstore-bench")]
#[command(about = "DB-level benchmark runner for HotStore workloads")]
struct Cli {
    #[arg(long)]
    backend: BackendKind,

    #[arg(long)]
    db_path: PathBuf,

    #[arg(long, value_enum)]
    workload: WorkloadArg,

    #[arg(long)]
    keys: Option<PathBuf>,

    #[arg(long)]
    tx_keys: Option<PathBuf>,

    #[arg(long)]
    object_version_keys: Option<PathBuf>,

    #[arg(long)]
    object_keys: Option<PathBuf>,

    #[arg(long)]
    event_types: Option<PathBuf>,

    #[arg(long)]
    checksum_report: Option<PathBuf>,

    #[arg(long, default_value_t = 10_000)]
    requests: usize,

    #[arg(long)]
    warmup_requests: Option<usize>,

    #[arg(long, default_value = "1")]
    concurrency: String,

    #[arg(long, default_value_t = 10)]
    batch_size: usize,

    #[arg(long, default_value_t = 100)]
    scan_limit: usize,

    #[arg(long, value_enum, default_value = "materialized")]
    scan_mode: ScanModeArg,

    #[arg(long)]
    dataset: Option<String>,

    #[arg(long, value_enum, default_value = "sequential")]
    access_pattern: AccessPatternArg,

    #[arg(long, default_value_t = 0x5eed_cafe_f00d_f00d)]
    seed: u64,

    #[arg(long)]
    mix: Option<String>,

    #[arg(long, value_enum, default_value = "unknown")]
    cache_state: CacheStateArg,

    #[arg(long)]
    compact_before_run: bool,

    #[arg(long)]
    min_hit_rate: Option<f64>,

    #[arg(long, default_value_t = 1)]
    min_requests_per_worker: usize,

    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum WorkloadArg {
    GetTx,
    GetObjectVersion,
    GetObjectLastSeen,
    MultiGetTx,
    MultiGetObjectVersion,
    ScanEvents,
    MixedRpc,
    Noop,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AccessPatternArg {
    Sequential,
    Uniform,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ScanModeArg {
    Materialized,
    Count,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CacheStateArg {
    Unknown,
    Hot,
    Cold,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = BenchmarkConfig {
        backend: cli.backend,
        db_path: cli.db_path,
        workload: match cli.workload {
            WorkloadArg::GetTx => WorkloadKind::GetTx,
            WorkloadArg::GetObjectVersion => WorkloadKind::GetObjectVersion,
            WorkloadArg::GetObjectLastSeen => WorkloadKind::GetObjectLastSeen,
            WorkloadArg::MultiGetTx => WorkloadKind::MultiGetTx,
            WorkloadArg::MultiGetObjectVersion => WorkloadKind::MultiGetObjectVersion,
            WorkloadArg::ScanEvents => WorkloadKind::ScanEvents,
            WorkloadArg::MixedRpc => WorkloadKind::MixedRpc,
            WorkloadArg::Noop => WorkloadKind::Noop,
        },
        keys_path: cli.keys,
        tx_keys_path: cli.tx_keys,
        object_version_keys_path: cli.object_version_keys,
        object_keys_path: cli.object_keys,
        event_types_path: cli.event_types,
        checksum_report_path: cli.checksum_report,
        requests: cli.requests,
        warmup_requests: cli.warmup_requests.unwrap_or(cli.requests / 10),
        concurrency: parse_concurrency_list(&cli.concurrency)?,
        batch_size: cli.batch_size,
        scan_limit: cli.scan_limit,
        scan_mode: match cli.scan_mode {
            ScanModeArg::Materialized => ScanMode::Materialized,
            ScanModeArg::Count => ScanMode::Count,
        },
        dataset: cli.dataset,
        access_pattern: match cli.access_pattern {
            AccessPatternArg::Sequential => AccessPattern::Sequential,
            AccessPatternArg::Uniform => AccessPattern::Uniform,
        },
        seed: cli.seed,
        min_hit_rate: cli.min_hit_rate,
        min_requests_per_worker: cli.min_requests_per_worker,
        mixed_weights: cli
            .mix
            .as_deref()
            .map(parse_mixed_weights)
            .transpose()?
            .unwrap_or_else(MixedWeights::default),
        cache_state: match cli.cache_state {
            CacheStateArg::Unknown => CacheState::Unknown,
            CacheStateArg::Hot => CacheState::Hot,
            CacheStateArg::Cold => CacheState::Cold,
        },
        compact_before_run: cli.compact_before_run,
    };

    let report = run_benchmark_suite(config)?;
    write_json_output(&report, cli.output.as_deref())?;
    Ok(())
}
