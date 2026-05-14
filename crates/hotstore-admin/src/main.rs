mod checksum;
mod export_keys;
mod raw_sync;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use hotstore_db::BackendKind;

use crate::checksum::{
    compare_checksum_reports, compute_checksum_report, compute_stats_report, write_json_output,
};
use crate::export_keys::{export_bench_keys, BenchKeyExportConfig};
use crate::raw_sync::{export_raw_to_path, import_raw_from_path, RawImportConfig};

#[derive(Debug, Parser)]
#[command(name = "hotstore-admin")]
#[command(about = "Administrative tools for inspecting benchmark data stores")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Stats {
        #[arg(long)]
        backend: BackendKind,
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Checksum {
        #[arg(long)]
        backend: BackendKind,
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    CompareChecksum {
        #[arg(long)]
        left: PathBuf,
        #[arg(long)]
        right: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    ExportBenchKeys {
        #[arg(long)]
        backend: BackendKind,
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long, default_value_t = 100_000)]
        tx_limit: usize,
        #[arg(long, default_value_t = 100_000)]
        object_version_limit: usize,
        #[arg(long, default_value_t = 100_000)]
        object_id_limit: usize,
        #[arg(long, default_value_t = 1_000)]
        event_type_limit: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    ExportRaw {
        #[arg(long)]
        backend: BackendKind,
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        report_output: Option<PathBuf>,
    },
    ImportRaw {
        #[arg(long)]
        backend: BackendKind,
        #[arg(long)]
        db_path: PathBuf,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long, default_value_t = 50_000)]
        batch_rows: usize,
        #[arg(long)]
        allow_existing: bool,
        #[arg(long)]
        compact: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Stats {
            backend,
            db_path,
            output,
        } => {
            let report = compute_stats_report(backend, &db_path)?;
            write_json_output(&report, output.as_deref())?;
        }
        Command::Checksum {
            backend,
            db_path,
            output,
        } => {
            let report = compute_checksum_report(backend, &db_path)?;
            write_json_output(&report, output.as_deref())?;
        }
        Command::CompareChecksum {
            left,
            right,
            output,
        } => {
            let report = compare_checksum_reports(&left, &right)?;
            let matches = report.matches;
            write_json_output(&report, output.as_deref())?;
            if !matches {
                bail!("checksum reports do not match");
            }
        }
        Command::ExportBenchKeys {
            backend,
            db_path,
            out_dir,
            tx_limit,
            object_version_limit,
            object_id_limit,
            event_type_limit,
            output,
        } => {
            let manifest = export_bench_keys(
                backend,
                &db_path,
                &out_dir,
                &BenchKeyExportConfig {
                    tx_limit,
                    object_version_limit,
                    object_id_limit,
                    event_type_limit,
                },
            )?;
            write_json_output(&manifest, output.as_deref())?;
        }
        Command::ExportRaw {
            backend,
            db_path,
            output,
            report_output,
        } => {
            let report = export_raw_to_path(backend, &db_path, output.as_deref())?;
            if output.is_some() || report_output.is_some() {
                write_json_output(&report, report_output.as_deref())?;
            } else {
                eprintln!(
                    "exported {} rows from {} to stdout",
                    report.totals.entries,
                    db_path.display()
                );
            }
        }
        Command::ImportRaw {
            backend,
            db_path,
            input,
            batch_rows,
            allow_existing,
            compact,
            output,
        } => {
            let report = import_raw_from_path(
                backend,
                &db_path,
                input.as_deref(),
                RawImportConfig {
                    batch_rows,
                    allow_existing,
                    compact,
                },
            )?;
            write_json_output(&report, output.as_deref())?;
        }
    }

    Ok(())
}
