#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/run-toplingdb-benchmark-demo.sh [options]

Purpose:
  Run a small end-to-end ToplingDB benchmark demo on Linux:
    1. validate the ToplingDB dependency switch and config env
    2. ingest the same checkpoint range into two ToplingDB directories
    3. generate benchmark key files from the same checkpoint range
    4. compute checksum for the second database
    5. run the DB benchmark suite on the first database and compare checksums

Defaults:
  - network: testnet
  - checkpoint range: 331445801..331445803
  - base dir: ./data/toplingdb-benchmark-demo-<network>-<first>-<last>
  - cargo profile: dev
  - requests: 2000
  - concurrency: 1,2,4
  - batch size: 10
  - tx batch size: 25
  - checkpoint batch size: 2

Required environment:
  TOPLINGDB_EASY_MIGRATE_CONF=/path/to/sui/crates/typed-store/config/topling_sui.yaml

Expected Cargo.toml change:
  [patch.crates-io]
  rocksdb = { git = "https://github.com/topling/rust-toplingdb" }

Examples:
  scripts/run-toplingdb-benchmark-demo.sh

  scripts/run-toplingdb-benchmark-demo.sh \
    --network mainnet \
    --first-checkpoint 123 \
    --last-checkpoint 140 \
    --base-dir /tmp/sui-hotstore-topling-mainnet-demo \
    --cargo-profile release \
    --requests 10000 \
    --concurrency 1,4,8
EOF
}

NETWORK="testnet"
FIRST_CHECKPOINT="331445801"
LAST_CHECKPOINT="331445803"
BASE_DIR=""
RPC_URL=""
REMOTE_STORE_URL=""
CARGO_PROFILE="dev"
REQUESTS="2000"
CONCURRENCY="1,2,4"
BATCH_SIZE="10"
TX_BATCH_SIZE="25"
CHECKPOINT_BATCH_SIZE="2"
SKIP_CARGO_CHECK="0"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      NETWORK="${2:?missing value for --network}"
      shift 2
      ;;
    --first-checkpoint)
      FIRST_CHECKPOINT="${2:?missing value for --first-checkpoint}"
      shift 2
      ;;
    --last-checkpoint)
      LAST_CHECKPOINT="${2:?missing value for --last-checkpoint}"
      shift 2
      ;;
    --base-dir)
      BASE_DIR="${2:?missing value for --base-dir}"
      shift 2
      ;;
    --rpc-url)
      RPC_URL="${2:?missing value for --rpc-url}"
      shift 2
      ;;
    --remote-store-url)
      REMOTE_STORE_URL="${2:?missing value for --remote-store-url}"
      shift 2
      ;;
    --cargo-profile)
      CARGO_PROFILE="${2:?missing value for --cargo-profile}"
      shift 2
      ;;
    --requests)
      REQUESTS="${2:?missing value for --requests}"
      shift 2
      ;;
    --concurrency)
      CONCURRENCY="${2:?missing value for --concurrency}"
      shift 2
      ;;
    --batch-size)
      BATCH_SIZE="${2:?missing value for --batch-size}"
      shift 2
      ;;
    --tx-batch-size)
      TX_BATCH_SIZE="${2:?missing value for --tx-batch-size}"
      shift 2
      ;;
    --checkpoint-batch-size)
      CHECKPOINT_BATCH_SIZE="${2:?missing value for --checkpoint-batch-size}"
      shift 2
      ;;
    --skip-cargo-check)
      SKIP_CARGO_CHECK="1"
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

file_contains_topling_patch() {
  local path="$1"
  local pattern='rocksdb\s*=\s*\{\s*git\s*=\s*"https://github.com/topling/rust-toplingdb"'

  if command -v rg >/dev/null 2>&1; then
    rg -q "$pattern" "$path"
  else
    grep -Eq "$pattern" "$path"
  fi
}

case "$NETWORK" in
  mainnet|testnet|devnet)
    ;;
  *)
    echo "Unsupported network: $NETWORK (expected mainnet, testnet, or devnet)" >&2
    exit 1
    ;;
esac

if [[ ! "$FIRST_CHECKPOINT" =~ ^[0-9]+$ || ! "$LAST_CHECKPOINT" =~ ^[0-9]+$ ]]; then
  echo "--first-checkpoint and --last-checkpoint must be integers" >&2
  exit 1
fi

if (( LAST_CHECKPOINT < FIRST_CHECKPOINT )); then
  echo "--last-checkpoint must be >= --first-checkpoint" >&2
  exit 1
fi

if [[ "$CARGO_PROFILE" != "dev" && "$CARGO_PROFILE" != "release" ]]; then
  echo "--cargo-profile must be dev or release" >&2
  exit 1
fi

if [[ -z "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]]; then
  echo "TOPLINGDB_EASY_MIGRATE_CONF is required" >&2
  exit 1
fi

if [[ ! -f "$TOPLINGDB_EASY_MIGRATE_CONF" ]]; then
  echo "TOPLINGDB_EASY_MIGRATE_CONF does not point to a file: $TOPLINGDB_EASY_MIGRATE_CONF" >&2
  exit 1
fi

require_cmd cargo

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script is intended for Linux. Current OS: $(uname -s)" >&2
  exit 1
fi

if ! file_contains_topling_patch Cargo.toml; then
  echo "Cargo.toml does not appear to patch rocksdb to rust-toplingdb yet." >&2
  echo "Please add this before running the demo:" >&2
  echo "[patch.crates-io]" >&2
  echo 'rocksdb = { git = "https://github.com/topling/rust-toplingdb" }' >&2
  exit 1
fi

if [[ -z "$RPC_URL" ]]; then
  case "$NETWORK" in
    mainnet)
      RPC_URL="https://fullnode.mainnet.sui.io:443"
      ;;
    testnet)
      RPC_URL="https://fullnode.testnet.sui.io:443"
      ;;
    devnet)
      RPC_URL="https://fullnode.devnet.sui.io:443"
      ;;
  esac
fi

if [[ -z "$REMOTE_STORE_URL" ]]; then
  case "$NETWORK" in
    mainnet)
      REMOTE_STORE_URL="https://checkpoints.mainnet.sui.io"
      ;;
    testnet)
      REMOTE_STORE_URL="https://checkpoints.testnet.sui.io"
      ;;
    devnet)
      REMOTE_STORE_URL="https://checkpoints.devnet.sui.io"
      ;;
  esac
fi

if [[ -z "$BASE_DIR" ]]; then
  BASE_DIR="$(pwd)/data/toplingdb-benchmark-demo-${NETWORK}-${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
fi

DB_A_DIR="${BASE_DIR}/db-a"
DB_B_DIR="${BASE_DIR}/db-b"
KEYS_DIR="${BASE_DIR}/keys"
REPORT_DIR="${BASE_DIR}/reports"
CHECKSUM_B_JSON="${BASE_DIR}/checksum-b.json"

mkdir -p "$BASE_DIR"
rm -rf "$DB_A_DIR" "$DB_B_DIR" "$KEYS_DIR" "$REPORT_DIR"
mkdir -p "$DB_A_DIR" "$DB_B_DIR"

echo "ToplingDB benchmark demo"
echo "  network: ${NETWORK}"
echo "  checkpoint range: ${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
echo "  base dir: ${BASE_DIR}"
echo "  rpc url: ${RPC_URL}"
echo "  remote store url: ${REMOTE_STORE_URL}"
echo "  config: ${TOPLINGDB_EASY_MIGRATE_CONF}"

if [[ "$SKIP_CARGO_CHECK" != "1" ]]; then
  echo "[0/5] cargo check --workspace"
  cargo check --workspace
fi

run_ingest() {
  local db_path="$1"
  cargo run --bin sui-hotstore-ingest-real -- \
    --network "$NETWORK" \
    --remote-store-url "$REMOTE_STORE_URL" \
    --first-checkpoint "$FIRST_CHECKPOINT" \
    --last-checkpoint "$LAST_CHECKPOINT" \
    --backend toplingdb \
    --db-path "$db_path" \
    --record-mode lite \
    --rpc-url "$RPC_URL" \
    --tx-batch-size "$TX_BATCH_SIZE" \
    --checkpoint-batch-size "$CHECKPOINT_BATCH_SIZE"
}

echo "[1/5] ingest db-a"
run_ingest "$DB_A_DIR"

echo "[2/5] ingest db-b"
run_ingest "$DB_B_DIR"

echo "[3/5] generate benchmark keys"
scripts/gen-bench-keys-from-checkpoints.sh \
  --network "$NETWORK" \
  --rpc-url "$RPC_URL" \
  --first-checkpoint "$FIRST_CHECKPOINT" \
  --last-checkpoint "$LAST_CHECKPOINT" \
  --out-dir "$KEYS_DIR" \
  --tx-batch-size "$TX_BATCH_SIZE"

echo "[4/5] checksum db-b"
cargo run --bin hotstore-admin -- \
  checksum \
  --backend toplingdb \
  --db-path "$DB_B_DIR" \
  --output "$CHECKSUM_B_JSON"

echo "[5/5] benchmark suite on db-a"
scripts/run-benchmark-suite.sh \
  --backend toplingdb \
  --db-path "$DB_A_DIR" \
  --keys-dir "$KEYS_DIR" \
  --report-dir "$REPORT_DIR" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --batch-size "$BATCH_SIZE" \
  --cargo-profile "$CARGO_PROFILE" \
  --compare-checksum-with "$CHECKSUM_B_JSON"

echo
echo "ToplingDB demo finished."
echo "  db-a: ${DB_A_DIR}"
echo "  db-b: ${DB_B_DIR}"
echo "  keys: ${KEYS_DIR}"
echo "  checksum-b: ${CHECKSUM_B_JSON}"
echo "  reports: ${REPORT_DIR}"
