use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};
use hotstore_db::BackendKind;

use crate::extract::RecordMode;

#[derive(Debug, Clone, Parser)]
#[command(name = "sui-hotstore-ingest-real")]
#[command(about = "Ingest a bounded range of real Sui data into HotStore")]
pub struct Cli {
    #[arg(long)]
    pub network: String,

    #[arg(long)]
    pub remote_store_url: String,

    #[arg(long)]
    pub first_checkpoint: u64,

    #[arg(long)]
    pub last_checkpoint: u64,

    #[arg(long, value_enum)]
    pub backend: BackendArg,

    #[arg(long)]
    pub db_path: PathBuf,

    #[arg(long)]
    pub bench_keys_dir: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = RecordModeArg::Lite)]
    pub record_mode: RecordModeArg,

    #[arg(long)]
    pub rpc_url: Option<String>,

    #[arg(long, default_value_t = 50)]
    pub tx_batch_size: usize,

    #[arg(long, default_value_t = 100)]
    pub checkpoint_batch_size: usize,

    #[arg(long, default_value_t = 3)]
    pub max_retries: usize,

    #[arg(long, default_value_t = 750)]
    pub retry_backoff_ms: u64,

    #[arg(long, default_value_t = false)]
    pub resume: bool,
}

impl Cli {
    pub fn backend_kind(&self) -> Result<BackendKind> {
        Ok(match self.backend {
            BackendArg::Rocksdb => BackendKind::RocksDb,
            BackendArg::Toplingdb => BackendKind::ToplingDb,
        })
    }

    pub fn record_mode_kind(&self) -> Result<RecordMode> {
        Ok(match self.record_mode {
            RecordModeArg::Lite => RecordMode::Lite,
            RecordModeArg::Raw => RecordMode::Raw,
            RecordModeArg::Full => RecordMode::Full,
        })
    }

    pub fn source_label(&self) -> String {
        format!(
            "rpc:{} (checkpoint_store_hint:{})",
            self.rpc_url(),
            self.remote_store_url
        )
    }

    pub fn rpc_url(&self) -> String {
        self.rpc_url
            .clone()
            .unwrap_or_else(|| default_rpc_url(&self.network).to_owned())
    }

    pub fn validate(&self) -> Result<()> {
        if self.last_checkpoint < self.first_checkpoint {
            bail!(
                "--last-checkpoint ({}) must be >= --first-checkpoint ({})",
                self.last_checkpoint,
                self.first_checkpoint
            );
        }

        if self.tx_batch_size == 0 {
            bail!("--tx-batch-size must be >= 1");
        }

        if self.checkpoint_batch_size == 0 {
            bail!("--checkpoint-batch-size must be >= 1");
        }

        if let Some(dir) = &self.bench_keys_dir {
            if dir.as_os_str().is_empty() {
                bail!("--bench-keys-dir must not be empty");
            }
        }

        Ok(())
    }
}

fn default_rpc_url(network: &str) -> &'static str {
    match network {
        "mainnet" => "https://fullnode.mainnet.sui.io:443",
        "testnet" => "https://fullnode.testnet.sui.io:443",
        "devnet" => "https://fullnode.devnet.sui.io:443",
        _ => "https://fullnode.testnet.sui.io:443",
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BackendArg {
    Rocksdb,
    Toplingdb,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RecordModeArg {
    Lite,
    Raw,
    Full,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    #[test]
    fn cli_defaults_rpc_url_from_network() {
        let cli = Cli::parse_from([
            "sui-hotstore-ingest-real",
            "--network",
            "testnet",
            "--remote-store-url",
            "https://checkpoints.testnet.sui.io",
            "--first-checkpoint",
            "100",
            "--last-checkpoint",
            "101",
            "--backend",
            "rocksdb",
            "--db-path",
            "/tmp/demo-hotstore",
        ]);

        assert_eq!(cli.rpc_url(), "https://fullnode.testnet.sui.io:443");
        assert_eq!(cli.tx_batch_size, 50);
        assert_eq!(cli.checkpoint_batch_size, 100);
        assert!(cli.bench_keys_dir.is_none());
        assert!(!cli.resume);
    }

    #[test]
    fn cli_rejects_inverted_checkpoint_range() {
        let cli = Cli::parse_from([
            "sui-hotstore-ingest-real",
            "--network",
            "testnet",
            "--remote-store-url",
            "https://checkpoints.testnet.sui.io",
            "--first-checkpoint",
            "101",
            "--last-checkpoint",
            "100",
            "--backend",
            "rocksdb",
            "--db-path",
            "/tmp/demo-hotstore",
        ]);

        assert!(cli.validate().is_err());
    }
}
