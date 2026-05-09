#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/run-benchmark-suite.sh \
    --backend <rocksdb|toplingdb> \
    --db-path <path> \
    [--report-dir <path>] \
    [--keys-dir <path>] \
    [--dataset <name>] \
    [--requests <count>] \
    [--warmup-requests <count>] \
    [--concurrency <csv>] \
    [--access-pattern <sequential|uniform>] \
    [--seed <u64>] \
    [--scan-mode <materialized|count>] \
    [--mix <profile>] \
    [--cache-state <unknown|hot|cold>] \
    [--compact-before-run] \
    [--min-hit-rate <0.0-1.0>] \
    [--min-requests-per-worker <count>] \
    [--batch-size <count>] \
    [--cargo-profile <dev|release>] \
    [--compare-checksum-with <path>]

This DB-only suite runs the currently implemented benchmark workloads:
  - hotstore-admin stats
  - hotstore-admin checksum
  - hotstore-bench get-tx
  - hotstore-bench get-object-version
  - hotstore-bench get-object-last-seen
  - hotstore-bench multi-get-tx
  - hotstore-bench multi-get-object-version
  - hotstore-bench scan-events
  - hotstore-bench mixed-rpc

Expected key files under --keys-dir:
  tx_digests.txt
  object_versions.txt
  object_ids.txt
  event_types.txt
EOF
}

BACKEND=""
DB_PATH=""
REPORT_DIR=""
KEYS_DIR="./bench/keys"
DATASET=""
REQUESTS="100000"
WARMUP_REQUESTS=""
CONCURRENCY="1,4,8,16,32,64"
ACCESS_PATTERN="sequential"
SEED="6840346605343600653"
SCAN_MODE="materialized"
MIX=""
CACHE_STATE="unknown"
COMPACT_BEFORE_RUN="false"
MIN_HIT_RATE=""
MIN_REQUESTS_PER_WORKER="1"
BATCH_SIZE="50"
CARGO_PROFILE="release"
COMPARE_CHECKSUM_WITH=""
TARGET_NOFILE="1048576"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --backend)
      BACKEND="${2:-}"
      shift 2
      ;;
    --db-path)
      DB_PATH="${2:-}"
      shift 2
      ;;
    --report-dir)
      REPORT_DIR="${2:-}"
      shift 2
      ;;
    --keys-dir)
      KEYS_DIR="${2:-}"
      shift 2
      ;;
    --dataset)
      DATASET="${2:-}"
      shift 2
      ;;
    --requests)
      REQUESTS="${2:-}"
      shift 2
      ;;
    --warmup-requests)
      WARMUP_REQUESTS="${2:-}"
      shift 2
      ;;
    --concurrency)
      CONCURRENCY="${2:-}"
      shift 2
      ;;
    --access-pattern)
      ACCESS_PATTERN="${2:-}"
      shift 2
      ;;
    --seed)
      SEED="${2:-}"
      shift 2
      ;;
    --scan-mode)
      SCAN_MODE="${2:-}"
      shift 2
      ;;
    --mix)
      MIX="${2:-}"
      shift 2
      ;;
    --cache-state)
      CACHE_STATE="${2:-}"
      shift 2
      ;;
    --compact-before-run)
      COMPACT_BEFORE_RUN="true"
      shift
      ;;
    --min-hit-rate)
      MIN_HIT_RATE="${2:-}"
      shift 2
      ;;
    --min-requests-per-worker)
      MIN_REQUESTS_PER_WORKER="${2:-}"
      shift 2
      ;;
    --batch-size)
      BATCH_SIZE="${2:-}"
      shift 2
      ;;
    --cargo-profile)
      CARGO_PROFILE="${2:-}"
      shift 2
      ;;
    --compare-checksum-with)
      COMPARE_CHECKSUM_WITH="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "$BACKEND" || -z "$DB_PATH" ]]; then
  echo "--backend and --db-path are required" >&2
  usage >&2
  exit 1
fi

if [[ "$BACKEND" != "rocksdb" && "$BACKEND" != "toplingdb" ]]; then
  echo "--backend must be rocksdb or toplingdb" >&2
  exit 1
fi

if [[ -z "$REPORT_DIR" ]]; then
  REPORT_DIR="./reports/${BACKEND}"
fi

if [[ "$CARGO_PROFILE" != "release" && "$CARGO_PROFILE" != "dev" ]]; then
  echo "--cargo-profile must be release or dev" >&2
  exit 1
fi

if [[ "$ACCESS_PATTERN" != "sequential" && "$ACCESS_PATTERN" != "uniform" ]]; then
  echo "--access-pattern must be sequential or uniform" >&2
  exit 1
fi

if [[ "$SCAN_MODE" != "materialized" && "$SCAN_MODE" != "count" ]]; then
  echo "--scan-mode must be materialized or count" >&2
  exit 1
fi

if [[ "$CACHE_STATE" != "unknown" && "$CACHE_STATE" != "hot" && "$CACHE_STATE" != "cold" ]]; then
  echo "--cache-state must be unknown, hot, or cold" >&2
  exit 1
fi

if [[ ! -d "$DB_PATH" ]]; then
  echo "db path does not exist: $DB_PATH" >&2
  exit 1
fi

for required_file in tx_digests.txt object_versions.txt object_ids.txt event_types.txt; do
  if [[ ! -f "$KEYS_DIR/$required_file" ]]; then
    echo "missing key file: $KEYS_DIR/$required_file" >&2
    exit 1
  fi
done

mkdir -p "$REPORT_DIR/db" "$REPORT_DIR/checksum"

if [[ "$CARGO_PROFILE" == "release" ]]; then
  CARGO_ARGS=(run --release)
else
  CARGO_ARGS=(run)
fi

CURRENT_NOFILE="$(ulimit -n)"
if [[ "$CURRENT_NOFILE" =~ ^[0-9]+$ ]] && (( CURRENT_NOFILE < TARGET_NOFILE )); then
  if ulimit -n "$TARGET_NOFILE" 2>/dev/null; then
    echo "raised ulimit -n from ${CURRENT_NOFILE} to ${TARGET_NOFILE}"
  else
    echo "warning: unable to raise ulimit -n to ${TARGET_NOFILE}; current value is ${CURRENT_NOFILE}" >&2
  fi
fi

echo "[1/9] stats (${BACKEND})"
cargo "${CARGO_ARGS[@]}" --bin hotstore-admin -- \
  stats \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --output "$REPORT_DIR/stats.json"

echo "[2/9] checksum (${BACKEND})"
cargo "${CARGO_ARGS[@]}" --bin hotstore-admin -- \
  checksum \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --output "$REPORT_DIR/checksum/checksum.json"

if [[ -n "$COMPARE_CHECKSUM_WITH" ]]; then
  echo "[2b/9] compare-checksum (${BACKEND})"
  cargo "${CARGO_ARGS[@]}" --bin hotstore-admin -- \
    compare-checksum \
    --left "$REPORT_DIR/checksum/checksum.json" \
    --right "$COMPARE_CHECKSUM_WITH" \
    --output "$REPORT_DIR/checksum/compare-checksum.json"
fi

run_bench() {
  local args=("$@")
  if [[ -n "$WARMUP_REQUESTS" ]]; then
    args+=(--warmup-requests "$WARMUP_REQUESTS")
  fi
  if [[ -n "$MIN_HIT_RATE" ]]; then
    args+=(--min-hit-rate "$MIN_HIT_RATE")
  fi
  if [[ -n "$MIX" ]]; then
    args+=(--mix "$MIX")
  fi
  args+=(--access-pattern "$ACCESS_PATTERN" --seed "$SEED")
  args+=(--scan-mode "$SCAN_MODE" --cache-state "$CACHE_STATE")
  if [[ "$COMPACT_BEFORE_RUN" == "true" ]]; then
    args+=(--compact-before-run)
  fi
  args+=(--min-requests-per-worker "$MIN_REQUESTS_PER_WORKER")
  args+=(--checksum-report "$REPORT_DIR/checksum/checksum.json")
  if [[ -n "$DATASET" ]]; then
    cargo "${CARGO_ARGS[@]}" --bin hotstore-bench -- "${args[@]}" --dataset "$DATASET"
  else
    cargo "${CARGO_ARGS[@]}" --bin hotstore-bench -- "${args[@]}"
  fi
}

echo "[3/9] get-tx (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload get-tx \
  --keys "$KEYS_DIR/tx_digests.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --output "$REPORT_DIR/db/get-tx.json"

echo "[4/9] get-object-version (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload get-object-version \
  --keys "$KEYS_DIR/object_versions.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --output "$REPORT_DIR/db/get-object-version.json"

echo "[5/9] get-object-last-seen (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload get-object-last-seen \
  --keys "$KEYS_DIR/object_ids.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --output "$REPORT_DIR/db/get-object-last-seen.json"

echo "[6/9] multi-get-tx (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload multi-get-tx \
  --keys "$KEYS_DIR/tx_digests.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --batch-size "$BATCH_SIZE" \
  --output "$REPORT_DIR/db/multi-get-tx.json"

echo "[7/9] multi-get-object-version (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload multi-get-object-version \
  --keys "$KEYS_DIR/object_versions.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --batch-size "$BATCH_SIZE" \
  --output "$REPORT_DIR/db/multi-get-object-version.json"

echo "[8/9] scan-events (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload scan-events \
  --event-types "$KEYS_DIR/event_types.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --scan-limit "$BATCH_SIZE" \
  --output "$REPORT_DIR/db/scan-events.json"

echo "[9/9] mixed-rpc (${BACKEND})"
run_bench \
  --backend "$BACKEND" \
  --db-path "$DB_PATH" \
  --workload mixed-rpc \
  --tx-keys "$KEYS_DIR/tx_digests.txt" \
  --object-version-keys "$KEYS_DIR/object_versions.txt" \
  --object-keys "$KEYS_DIR/object_ids.txt" \
  --event-types "$KEYS_DIR/event_types.txt" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --batch-size "$BATCH_SIZE" \
  --scan-limit "$BATCH_SIZE" \
  --output "$REPORT_DIR/db/mixed-rpc.json"

echo "done: $REPORT_DIR"
