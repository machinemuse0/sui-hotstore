#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/run-route1-benchmark-server.sh [options]

Purpose:
  Server-oriented end-to-end benchmark runner for route 1:
    1. resolve or reuse a fixed latest checkpoint range from a public RPC
    2. ingest that bounded range into a HotStore DB
    3. emit benchmark key files during ingest
    4. compute stats and checksum
    5. run the DB-level benchmark suite

Defaults:
  - network: mainnet
  - latest-count: 10000 (only when explicit first/last is not provided)
  - backend: rocksdb
  - cargo-profile: release
  - requests: 100000
  - concurrency: 1,4,8,16,32,64
  - batch size: 50
  - tx batch size: 50
  - checkpoint batch size: 100
  - rpc timeout secs: 30
  - rpc max retries inside ingest: 6
  - rpc retry backoff ms inside ingest: 1000
  - step max attempts: 20
  - step retry sleep secs: 30
  - base dir:
      - /data4/sui-hotstore-route1-<network>-latest-<count> when /data4 exists
      - ./data/sui-hotstore-route1-<network>-latest-<count> otherwise

Optional compare mode:
  Pass --write-compare-db to ingest the same range into a second DB and produce
  compare-checksum output as part of the suite. This roughly doubles ingest time.

Resume behavior:
  - the script stores the resolved checkpoint range under <base-dir>/run-config.env
  - rerunning the same base dir reuses that exact range instead of resolving a new one
  - ingest uses --resume and continues from the shared DB/key watermark
  - pass --reset-state to discard prior state and start fresh
  - pass explicit --first-checkpoint and --last-checkpoint when you want RocksDB and ToplingDB to use an identical fixed range

Examples:
  bash scripts/run-route1-benchmark-server.sh

  bash scripts/run-route1-benchmark-server.sh \
    --network mainnet \
    --first-checkpoint 270700000 \
    --last-checkpoint 270709999 \
    --base-dir /data4/sui-hotstore-mainnet-latest-10000 \
    --requests 200000 \
    --concurrency 1,8,16,32,64 \
    --write-compare-db
EOF
}

NETWORK="mainnet"
LATEST_COUNT="10000"
FIRST_CHECKPOINT=""
LAST_CHECKPOINT=""
BACKEND="rocksdb"
BASE_DIR=""
RPC_URL=""
REMOTE_STORE_URL=""
CARGO_PROFILE="release"
REQUESTS="100000"
CONCURRENCY="1,4,8,16,32,64"
BATCH_SIZE="50"
TX_BATCH_SIZE="50"
CHECKPOINT_BATCH_SIZE="100"
RPC_TIMEOUT_SECS="30"
RPC_MAX_RETRIES="6"
RPC_RETRY_BACKOFF_MS="1000"
STEP_MAX_ATTEMPTS="20"
STEP_RETRY_SLEEP_SECS="30"
WRITE_COMPARE_DB="0"
SKIP_CARGO_CHECK="0"
RESET_STATE="0"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      NETWORK="${2:?missing value for --network}"
      shift 2
      ;;
    --latest-count)
      LATEST_COUNT="${2:?missing value for --latest-count}"
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
    --backend)
      BACKEND="${2:?missing value for --backend}"
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
    --rpc-timeout-secs)
      RPC_TIMEOUT_SECS="${2:?missing value for --rpc-timeout-secs}"
      shift 2
      ;;
    --rpc-max-retries)
      RPC_MAX_RETRIES="${2:?missing value for --rpc-max-retries}"
      shift 2
      ;;
    --rpc-retry-backoff-ms)
      RPC_RETRY_BACKOFF_MS="${2:?missing value for --rpc-retry-backoff-ms}"
      shift 2
      ;;
    --step-max-attempts)
      STEP_MAX_ATTEMPTS="${2:?missing value for --step-max-attempts}"
      shift 2
      ;;
    --step-retry-sleep-secs)
      STEP_RETRY_SLEEP_SECS="${2:?missing value for --step-retry-sleep-secs}"
      shift 2
      ;;
    --write-compare-db)
      WRITE_COMPARE_DB="1"
      shift
      ;;
    --skip-cargo-check)
      SKIP_CARGO_CHECK="1"
      shift
      ;;
    --reset-state)
      RESET_STATE="1"
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

run_with_retries() {
  local label="$1"
  shift

  local attempt=1
  while true; do
    echo "${label}: attempt ${attempt}/${STEP_MAX_ATTEMPTS}" >&2
    if "$@"; then
      return 0
    fi

    if (( attempt >= STEP_MAX_ATTEMPTS )); then
      echo "${label}: exhausted ${STEP_MAX_ATTEMPTS} attempts" >&2
      return 1
    fi

    echo "${label}: failed, sleeping ${STEP_RETRY_SLEEP_SECS}s before retry" >&2
    sleep "$STEP_RETRY_SLEEP_SECS"
    attempt=$(( attempt + 1 ))
  done
}

case "$NETWORK" in
  mainnet|testnet|devnet)
    ;;
  *)
    echo "Unsupported network: $NETWORK (expected mainnet, testnet, or devnet)" >&2
    exit 1
    ;;
esac

if [[ "$BACKEND" != "rocksdb" && "$BACKEND" != "toplingdb" ]]; then
  echo "--backend must be rocksdb or toplingdb" >&2
  exit 1
fi

if [[ -n "$FIRST_CHECKPOINT" || -n "$LAST_CHECKPOINT" ]]; then
  if [[ -z "$FIRST_CHECKPOINT" || -z "$LAST_CHECKPOINT" ]]; then
    echo "--first-checkpoint and --last-checkpoint must be provided together" >&2
    exit 1
  fi
  if [[ ! "$FIRST_CHECKPOINT" =~ ^[0-9]+$ || ! "$LAST_CHECKPOINT" =~ ^[0-9]+$ ]]; then
    echo "--first-checkpoint and --last-checkpoint must be integers" >&2
    exit 1
  fi
  if (( LAST_CHECKPOINT < FIRST_CHECKPOINT )); then
    echo "--last-checkpoint must be >= --first-checkpoint" >&2
    exit 1
  fi
else
  if [[ ! "$LATEST_COUNT" =~ ^[0-9]+$ || "$LATEST_COUNT" -lt 1 ]]; then
    echo "--latest-count must be an integer >= 1" >&2
    exit 1
  fi
fi

if [[ ! "$RPC_MAX_RETRIES" =~ ^[0-9]+$ ]]; then
  echo "--rpc-max-retries must be an integer >= 0" >&2
  exit 1
fi

if [[ ! "$RPC_RETRY_BACKOFF_MS" =~ ^[0-9]+$ || "$RPC_RETRY_BACKOFF_MS" -lt 1 ]]; then
  echo "--rpc-retry-backoff-ms must be an integer >= 1" >&2
  exit 1
fi

if [[ ! "$STEP_MAX_ATTEMPTS" =~ ^[0-9]+$ || "$STEP_MAX_ATTEMPTS" -lt 1 ]]; then
  echo "--step-max-attempts must be an integer >= 1" >&2
  exit 1
fi

if [[ ! "$STEP_RETRY_SLEEP_SECS" =~ ^[0-9]+$ || "$STEP_RETRY_SLEEP_SECS" -lt 1 ]]; then
  echo "--step-retry-sleep-secs must be an integer >= 1" >&2
  exit 1
fi

if [[ "$CARGO_PROFILE" != "dev" && "$CARGO_PROFILE" != "release" ]]; then
  echo "--cargo-profile must be dev or release" >&2
  exit 1
fi

if [[ ! "$TX_BATCH_SIZE" =~ ^[0-9]+$ || "$TX_BATCH_SIZE" -lt 1 || "$TX_BATCH_SIZE" -gt 50 ]]; then
  echo "--tx-batch-size must be an integer in [1, 50] because public Sui RPC limits sui_multiGetTransactionBlocks to 50 digests" >&2
  exit 1
fi

if [[ ! "$CHECKPOINT_BATCH_SIZE" =~ ^[0-9]+$ || "$CHECKPOINT_BATCH_SIZE" -lt 1 ]]; then
  echo "--checkpoint-batch-size must be an integer >= 1" >&2
  exit 1
fi

if [[ ! "$RPC_TIMEOUT_SECS" =~ ^[0-9]+$ || "$RPC_TIMEOUT_SECS" -lt 1 ]]; then
  echo "--rpc-timeout-secs must be an integer >= 1" >&2
  exit 1
fi

REQUESTED_FIRST_CHECKPOINT="$FIRST_CHECKPOINT"
REQUESTED_LAST_CHECKPOINT="$LAST_CHECKPOINT"

require_cmd cargo
require_cmd curl
require_cmd jq

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
  if [[ -n "$FIRST_CHECKPOINT" && -n "$LAST_CHECKPOINT" ]]; then
    if [[ -d "/data4" ]]; then
      BASE_DIR="/data4/sui-hotstore-route1-${NETWORK}-${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
    else
      BASE_DIR="$(pwd)/data/sui-hotstore-route1-${NETWORK}-${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
    fi
  else
    if [[ -d "/data4" ]]; then
      BASE_DIR="/data4/sui-hotstore-route1-${NETWORK}-latest-${LATEST_COUNT}"
    else
      BASE_DIR="$(pwd)/data/sui-hotstore-route1-${NETWORK}-latest-${LATEST_COUNT}"
    fi
  fi
fi

RUN_CONFIG_ENV="${BASE_DIR}/run-config.env"

resolve_latest_checkpoint() {
  local response_file latest
  response_file="$(mktemp "${TMPDIR:-/tmp}/hotstore-latest.XXXXXX.json")"
  trap 'rm -f "$response_file"' RETURN
  curl --max-time "$RPC_TIMEOUT_SECS" -fsS "$RPC_URL" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"sui_getLatestCheckpointSequenceNumber","params":[]}' \
    -o "$response_file"
  jq -e '.error == null and .result != null' "$response_file" >/dev/null || {
    echo "RPC returned no latest checkpoint result" >&2
    jq . "$response_file" >&2 || true
    return 1
  }
  latest="$(jq -r '.result' "$response_file")"
  if [[ ! "$latest" =~ ^[0-9]+$ ]]; then
    echo "latest checkpoint result is not an integer: $latest" >&2
    return 1
  fi
  printf '%s\n' "$latest"
}

if [[ "$BACKEND" == "toplingdb" ]]; then
  if [[ -z "${TOPLINGDB_EASY_MIGRATE_CONF:-}" ]]; then
    echo "TOPLINGDB_EASY_MIGRATE_CONF is required for --backend toplingdb" >&2
    exit 1
  fi
  if [[ ! -f "$TOPLINGDB_EASY_MIGRATE_CONF" ]]; then
    echo "TOPLINGDB_EASY_MIGRATE_CONF does not point to a file: $TOPLINGDB_EASY_MIGRATE_CONF" >&2
    exit 1
  fi
fi

DB_A_DIR="${BASE_DIR}/db-a"
DB_B_DIR="${BASE_DIR}/db-b"
KEYS_DIR="${BASE_DIR}/keys"
REPORT_DIR="${BASE_DIR}/reports"
CHECKSUM_B_JSON="${BASE_DIR}/checksum-b.json"

mkdir -p "$BASE_DIR"

if [[ "$RESET_STATE" == "1" ]]; then
  echo "resetting prior route 1 state under ${BASE_DIR}"
  rm -rf "$DB_A_DIR" "$DB_B_DIR" "$KEYS_DIR" "$REPORT_DIR" "$CHECKSUM_B_JSON" "$RUN_CONFIG_ENV"
fi

mkdir -p "$DB_A_DIR" "$KEYS_DIR" "$REPORT_DIR"

if [[ "$WRITE_COMPARE_DB" == "1" ]]; then
  mkdir -p "$DB_B_DIR"
fi

if [[ -f "$RUN_CONFIG_ENV" ]]; then
  # shellcheck disable=SC1090
  source "$RUN_CONFIG_ENV"
  if [[ "${RUN_NETWORK:-}" != "$NETWORK" ]]; then
    echo "saved run config network (${RUN_NETWORK:-}) does not match requested network (${NETWORK}); use --reset-state or a different --base-dir" >&2
    exit 1
  fi
  if [[ "${RUN_BACKEND:-}" != "$BACKEND" ]]; then
    echo "saved run config backend (${RUN_BACKEND:-}) does not match requested backend (${BACKEND}); use a different --base-dir for each backend or pass --reset-state" >&2
    exit 1
  fi
  if [[ -n "$REQUESTED_FIRST_CHECKPOINT" && -n "$REQUESTED_LAST_CHECKPOINT" ]]; then
    if [[ "${RUN_FIRST_CHECKPOINT:-}" != "$REQUESTED_FIRST_CHECKPOINT" || "${RUN_LAST_CHECKPOINT:-}" != "$REQUESTED_LAST_CHECKPOINT" ]]; then
      echo "saved checkpoint range (${RUN_FIRST_CHECKPOINT:-}-${RUN_LAST_CHECKPOINT:-}) does not match requested range (${REQUESTED_FIRST_CHECKPOINT}-${REQUESTED_LAST_CHECKPOINT}); use --reset-state or a different --base-dir" >&2
      exit 1
    fi
  fi
  FIRST_CHECKPOINT="${RUN_FIRST_CHECKPOINT}"
  LAST_CHECKPOINT="${RUN_LAST_CHECKPOINT}"
  LATEST_CHECKPOINT="${RUN_LATEST_CHECKPOINT}"
  echo "reusing saved checkpoint range from ${RUN_CONFIG_ENV}"
else
  if [[ -n "$FIRST_CHECKPOINT" && -n "$LAST_CHECKPOINT" ]]; then
    LATEST_CHECKPOINT="$LAST_CHECKPOINT"
  else
    LATEST_CHECKPOINT="$(run_with_retries "resolve latest checkpoint" resolve_latest_checkpoint)"
    FIRST_CHECKPOINT=$(( LATEST_CHECKPOINT - LATEST_COUNT + 1 ))
    if (( FIRST_CHECKPOINT < 0 )); then
      FIRST_CHECKPOINT=0
    fi
    LAST_CHECKPOINT="$LATEST_CHECKPOINT"
  fi

  cat >"$RUN_CONFIG_ENV" <<EOF
RUN_NETWORK='${NETWORK}'
RUN_BACKEND='${BACKEND}'
RUN_RPC_URL='${RPC_URL}'
RUN_REMOTE_STORE_URL='${REMOTE_STORE_URL}'
RUN_LATEST_COUNT='${LATEST_COUNT}'
RUN_LATEST_CHECKPOINT='${LATEST_CHECKPOINT}'
RUN_FIRST_CHECKPOINT='${FIRST_CHECKPOINT}'
RUN_LAST_CHECKPOINT='${LAST_CHECKPOINT}'
EOF
  echo "saved checkpoint range to ${RUN_CONFIG_ENV}"
fi

echo "Route 1 benchmark server run"
echo "  network: ${NETWORK}"
echo "  backend: ${BACKEND}"
echo "  latest checkpoint: ${LATEST_CHECKPOINT}"
echo "  checkpoint range: ${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
echo "  latest count: ${LATEST_COUNT}"
echo "  base dir: ${BASE_DIR}"
echo "  rpc url: ${RPC_URL}"
echo "  remote store url: ${REMOTE_STORE_URL}"
echo "  cargo profile: ${CARGO_PROFILE}"
echo "  requests: ${REQUESTS}"
echo "  concurrency: ${CONCURRENCY}"
echo "  batch size: ${BATCH_SIZE}"
echo "  rpc max retries: ${RPC_MAX_RETRIES}"
echo "  rpc retry backoff ms: ${RPC_RETRY_BACKOFF_MS}"
echo "  step max attempts: ${STEP_MAX_ATTEMPTS}"
echo "  step retry sleep secs: ${STEP_RETRY_SLEEP_SECS}"

if [[ "$SKIP_CARGO_CHECK" != "1" ]]; then
  echo "[0/5] cargo check --workspace"
  cargo check --workspace
fi

run_cargo_ingest() {
  local db_path="$1"
  local emit_keys="$2"
  local extra_args=()
  if [[ -n "$emit_keys" ]]; then
    extra_args+=(--bench-keys-dir "$KEYS_DIR")
  fi
  if [[ "$CARGO_PROFILE" == "release" ]]; then
    cargo run --release --bin sui-hotstore-ingest-real -- \
      --network "$NETWORK" \
      --remote-store-url "$REMOTE_STORE_URL" \
      --first-checkpoint "$FIRST_CHECKPOINT" \
      --last-checkpoint "$LAST_CHECKPOINT" \
      --backend "$BACKEND" \
      --db-path "$db_path" \
      --record-mode lite \
      --checkpoint-batch-size "$CHECKPOINT_BATCH_SIZE" \
      --tx-batch-size "$TX_BATCH_SIZE" \
      --max-retries "$RPC_MAX_RETRIES" \
      --retry-backoff-ms "$RPC_RETRY_BACKOFF_MS" \
      --resume \
      "${extra_args[@]}"
  else
    cargo run --bin sui-hotstore-ingest-real -- \
      --network "$NETWORK" \
      --remote-store-url "$REMOTE_STORE_URL" \
      --first-checkpoint "$FIRST_CHECKPOINT" \
      --last-checkpoint "$LAST_CHECKPOINT" \
      --backend "$BACKEND" \
      --db-path "$db_path" \
      --record-mode lite \
      --checkpoint-batch-size "$CHECKPOINT_BATCH_SIZE" \
      --tx-batch-size "$TX_BATCH_SIZE" \
      --max-retries "$RPC_MAX_RETRIES" \
      --retry-backoff-ms "$RPC_RETRY_BACKOFF_MS" \
      --resume \
      "${extra_args[@]}"
  fi
}

echo "[1/5] ingest primary DB"
run_with_retries "ingest primary DB" run_cargo_ingest "$DB_A_DIR" "1"

if [[ "$WRITE_COMPARE_DB" == "1" ]]; then
  echo "[1b/5] ingest compare DB"
  run_with_retries "ingest compare DB" run_cargo_ingest "$DB_B_DIR" ""
fi

if [[ "$WRITE_COMPARE_DB" == "1" ]]; then
  echo "[2/5] checksum compare DB"
  if [[ "$CARGO_PROFILE" == "release" ]]; then
    cargo run --release --bin hotstore-admin -- \
      checksum \
      --backend "$BACKEND" \
      --db-path "$DB_B_DIR" \
      --output "$CHECKSUM_B_JSON"
  else
    cargo run --bin hotstore-admin -- \
      checksum \
      --backend "$BACKEND" \
      --db-path "$DB_B_DIR" \
      --output "$CHECKSUM_B_JSON"
  fi
fi

if [[ "$WRITE_COMPARE_DB" == "1" ]]; then
  echo "[3/5] run benchmark suite with checksum compare"
  bash scripts/run-benchmark-suite.sh \
    --backend "$BACKEND" \
    --db-path "$DB_A_DIR" \
    --keys-dir "$KEYS_DIR" \
    --report-dir "$REPORT_DIR" \
    --requests "$REQUESTS" \
    --concurrency "$CONCURRENCY" \
    --batch-size "$BATCH_SIZE" \
    --cargo-profile "$CARGO_PROFILE" \
    --compare-checksum-with "$CHECKSUM_B_JSON"
else
  echo "[2/5] run benchmark suite"
  bash scripts/run-benchmark-suite.sh \
    --backend "$BACKEND" \
    --db-path "$DB_A_DIR" \
    --keys-dir "$KEYS_DIR" \
    --report-dir "$REPORT_DIR" \
    --requests "$REQUESTS" \
    --concurrency "$CONCURRENCY" \
    --batch-size "$BATCH_SIZE" \
    --cargo-profile "$CARGO_PROFILE"
fi

echo "[4/5] done"
echo "Primary DB: ${DB_A_DIR}"
if [[ "$WRITE_COMPARE_DB" == "1" ]]; then
  echo "Compare DB: ${DB_B_DIR}"
  echo "Compare checksum: ${CHECKSUM_B_JSON}"
fi
echo "Keys: ${KEYS_DIR}"
echo "Reports: ${REPORT_DIR}"
