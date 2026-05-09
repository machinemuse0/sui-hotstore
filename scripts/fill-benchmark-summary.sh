#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/fill-benchmark-summary.sh \
    [--rocksdb-report-dir <path>] \
    [--toplingdb-report-dir <path>] \
    [--keys-manifest <path>] \
    [--pick-concurrency <best-throughput|max-concurrency|max|N>] \
    [--output <path>]

Purpose:
  Read benchmark JSON outputs and regenerate reports/summary.md with the most
  useful DB-level fields already filled in.

Defaults:
  - output: ./reports/summary.md
  - pick-concurrency: best-throughput

Expected report layout under each report dir:
  stats.json
  checksum/checksum.json
  checksum/compare-checksum.json    (optional)
  db/get-tx.json
  db/get-object-version.json
  db/get-object-last-seen.json
  db/multi-get-tx.json
  db/multi-get-object-version.json
  db/scan-events.json
  db/mixed-rpc.json

Examples:
  scripts/fill-benchmark-summary.sh \
    --rocksdb-report-dir /tmp/hotstore-reports-rocksdb \
    --output reports/summary.md

  scripts/fill-benchmark-summary.sh \
    --rocksdb-report-dir /tmp/hotstore-reports-rocksdb \
    --toplingdb-report-dir /tmp/hotstore-reports-toplingdb \
    --keys-manifest /tmp/hotstore-bench-keys-testnet/manifest.json \
    --pick-concurrency 4 \
    --output reports/summary.md
EOF
}

ROCKSDB_REPORT_DIR=""
TOPLINGDB_REPORT_DIR=""
KEYS_MANIFEST=""
PICK_CONCURRENCY="best-throughput"
OUTPUT_PATH="$(pwd)/reports/summary.md"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --rocksdb-report-dir)
      ROCKSDB_REPORT_DIR="${2:?missing value for --rocksdb-report-dir}"
      shift 2
      ;;
    --toplingdb-report-dir)
      TOPLINGDB_REPORT_DIR="${2:?missing value for --toplingdb-report-dir}"
      shift 2
      ;;
    --keys-manifest)
      KEYS_MANIFEST="${2:?missing value for --keys-manifest}"
      shift 2
      ;;
    --pick-concurrency)
      PICK_CONCURRENCY="${2:?missing value for --pick-concurrency}"
      shift 2
      ;;
    --output)
      OUTPUT_PATH="${2:?missing value for --output}"
      shift 2
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

require_cmd jq
require_cmd mktemp

if [[ -z "$ROCKSDB_REPORT_DIR" && -z "$TOPLINGDB_REPORT_DIR" ]]; then
  echo "At least one of --rocksdb-report-dir or --toplingdb-report-dir is required" >&2
  exit 1
fi

if [[ "$PICK_CONCURRENCY" != "best-throughput" && "$PICK_CONCURRENCY" != "max-concurrency" && "$PICK_CONCURRENCY" != "max" && ! "$PICK_CONCURRENCY" =~ ^[0-9]+$ ]]; then
  echo "--pick-concurrency must be best-throughput, max-concurrency, max, or an integer" >&2
  exit 1
fi

if [[ -n "$KEYS_MANIFEST" && ! -f "$KEYS_MANIFEST" ]]; then
  echo "keys manifest does not exist: $KEYS_MANIFEST" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hotstore-summary.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

md_escape() {
  printf '%s' "$1" | sed 's/|/\\|/g'
}

format_float() {
  local value="${1:-}"
  if [[ -z "$value" || "$value" == "null" ]]; then
    printf -- ""
  else
    printf '%.2f' "$value"
  fi
}

format_int() {
  local value="${1:-}"
  if [[ -z "$value" || "$value" == "null" ]]; then
    printf -- ""
  else
    printf '%s' "$value"
  fi
}

json_string_or_empty() {
  local file="$1"
  local expr="$2"
  jq -r "$expr // \"\"" "$file"
}

load_dataset_meta() {
  local report_dir="$1"
  local prefix="$2"
  local stats_file="$report_dir/stats.json"
  local checksum_file="$report_dir/checksum/checksum.json"

  if [[ -f "$stats_file" ]]; then
    eval "${prefix}_db_path=\"\$(json_string_or_empty \"$stats_file\" '.db_path')\""
    eval "${prefix}_disk_usage=\"\$(json_string_or_empty \"$stats_file\" '.disk_usage_bytes')\""
    eval "${prefix}_tx_count=\"\$(json_string_or_empty \"$stats_file\" '.column_families.cf_tx_by_digest.entries')\""
    eval "${prefix}_event_count=\"\$(json_string_or_empty \"$stats_file\" '.column_families.cf_event_by_type.entries')\""
    eval "${prefix}_object_version_count=\"\$(json_string_or_empty \"$stats_file\" '.column_families.cf_object_version.entries')\""
    eval "${prefix}_owner_touched_count=\"\$(json_string_or_empty \"$stats_file\" '.column_families.cf_owner_touched_objects.entries')\""
  else
    eval "${prefix}_db_path=''"
    eval "${prefix}_disk_usage=''"
    eval "${prefix}_tx_count=''"
    eval "${prefix}_event_count=''"
    eval "${prefix}_object_version_count=''"
    eval "${prefix}_owner_touched_count=''"
  fi

  if [[ -f "$checksum_file" ]]; then
    eval "${prefix}_checksum=\"\$(json_string_or_empty \"$checksum_file\" '.totals.sha256')\""
  else
    eval "${prefix}_checksum=''"
  fi
}

load_compare_meta() {
  local report_dir="$1"
  local prefix="$2"
  local compare_file="$report_dir/checksum/compare-checksum.json"

  if [[ -f "$compare_file" ]]; then
    eval "${prefix}_compare_file=\"$compare_file\""
    eval "${prefix}_compare_match=\"\$(json_string_or_empty \"$compare_file\" '.matches')\""
  else
    eval "${prefix}_compare_file=''"
    eval "${prefix}_compare_match=''"
  fi
}

pick_run_json() {
  local file="$1"
  local output_file="$2"
  local jq_filter

  if [[ ! -f "$file" ]]; then
    printf '{}' > "$output_file"
    return
  fi

  if [[ "$PICK_CONCURRENCY" == "best-throughput" ]]; then
    jq_filter='.runs | max_by(.throughput_rps) // {}'
  elif [[ "$PICK_CONCURRENCY" == "max-concurrency" || "$PICK_CONCURRENCY" == "max" ]]; then
    jq_filter='.runs | sort_by(.concurrency) | last // {}'
  else
    jq_filter=".runs | map(select(.concurrency == ${PICK_CONCURRENCY})) | first // {}"
  fi

  jq "$jq_filter" "$file" > "$output_file"
}

emit_workload_row() {
  local workload="$1"
  local backend_label="$2"
  local report_dir="$3"
  local backend_key="$4"
  local json_file="$report_dir/db/${workload}.json"
  local run_file="$TMP_DIR/${backend_key}-${workload}.json"

  pick_run_json "$json_file" "$run_file"

  local concurrency requests throughput p50 p95 p99 p999 errors
  concurrency="$(json_string_or_empty "$run_file" '.concurrency')"
  requests="$(json_string_or_empty "$run_file" '.requests')"
  throughput="$(format_float "$(json_string_or_empty "$run_file" '.throughput_rps')")"
  p50="$(format_float "$(json_string_or_empty "$run_file" '.latency_ms.p50')")"
  p95="$(format_float "$(json_string_or_empty "$run_file" '.latency_ms.p95')")"
  p99="$(format_float "$(json_string_or_empty "$run_file" '.latency_ms.p99')")"
  p999="$(format_float "$(json_string_or_empty "$run_file" '.latency_ms.p999')")"
  errors="$(format_int "$(json_string_or_empty "$run_file" '.errors')")"

  printf '| %s | %s | %s | %s | %s | %s | %s | %s | %s | %s |\n' \
    "$workload" \
    "$backend_label" \
    "$concurrency" \
    "$requests" \
    "$throughput" \
    "$p50" \
    "$p95" \
    "$p99" \
    "$p999" \
    "$errors"
}

ROCKSDB_LABEL="RocksDB"
TOPLINGDB_LABEL="ToplingDB"

if [[ -n "$ROCKSDB_REPORT_DIR" && ! -d "$ROCKSDB_REPORT_DIR" ]]; then
  echo "rocksdb report dir does not exist: $ROCKSDB_REPORT_DIR" >&2
  exit 1
fi

if [[ -n "$TOPLINGDB_REPORT_DIR" && ! -d "$TOPLINGDB_REPORT_DIR" ]]; then
  echo "toplingdb report dir does not exist: $TOPLINGDB_REPORT_DIR" >&2
  exit 1
fi

if [[ -n "$ROCKSDB_REPORT_DIR" ]]; then
  load_dataset_meta "$ROCKSDB_REPORT_DIR" rocksdb
  load_compare_meta "$ROCKSDB_REPORT_DIR" rocksdb
else
  rocksdb_db_path=""
  rocksdb_disk_usage=""
  rocksdb_tx_count=""
  rocksdb_event_count=""
  rocksdb_object_version_count=""
  rocksdb_owner_touched_count=""
  rocksdb_checksum=""
  rocksdb_compare_file=""
  rocksdb_compare_match=""
fi

if [[ -n "$TOPLINGDB_REPORT_DIR" ]]; then
  load_dataset_meta "$TOPLINGDB_REPORT_DIR" toplingdb
  load_compare_meta "$TOPLINGDB_REPORT_DIR" toplingdb
else
  toplingdb_db_path=""
  toplingdb_disk_usage=""
  toplingdb_tx_count=""
  toplingdb_event_count=""
  toplingdb_object_version_count=""
  toplingdb_owner_touched_count=""
  toplingdb_checksum=""
  toplingdb_compare_file=""
  toplingdb_compare_match=""
fi

DATASET_SOURCE=""
DATASET_NETWORK=""
DATASET_CHECKPOINT_RANGE=""
KEY_TX_PATH=""
KEY_OBJECT_VERSIONS_PATH=""
KEY_OBJECT_IDS_PATH=""
KEY_EVENT_TYPES_PATH=""
KEY_TX_COUNT=""
KEY_OBJECT_VERSION_COUNT=""
KEY_OBJECT_ID_COUNT=""
KEY_EVENT_TYPE_COUNT=""

if [[ -n "$KEYS_MANIFEST" ]]; then
  DATASET_SOURCE="Generated from checkpoint RPC via scripts/gen-bench-keys-from-checkpoints.sh"
  DATASET_NETWORK="$(json_string_or_empty "$KEYS_MANIFEST" '.network')"
  local_first="$(json_string_or_empty "$KEYS_MANIFEST" '.first_checkpoint')"
  local_last="$(json_string_or_empty "$KEYS_MANIFEST" '.last_checkpoint')"
  if [[ -n "$local_first" && -n "$local_last" ]]; then
    DATASET_CHECKPOINT_RANGE="${local_first}..${local_last}"
  fi
  KEY_TX_PATH="$(json_string_or_empty "$KEYS_MANIFEST" '.generated_files.tx_digests')"
  KEY_OBJECT_VERSIONS_PATH="$(json_string_or_empty "$KEYS_MANIFEST" '.generated_files.object_versions')"
  KEY_OBJECT_IDS_PATH="$(json_string_or_empty "$KEYS_MANIFEST" '.generated_files.object_ids')"
  KEY_EVENT_TYPES_PATH="$(json_string_or_empty "$KEYS_MANIFEST" '.generated_files.event_types')"
  KEY_TX_COUNT="$(json_string_or_empty "$KEYS_MANIFEST" '.counts.tx_digests')"
  KEY_OBJECT_VERSION_COUNT="$(json_string_or_empty "$KEYS_MANIFEST" '.counts.object_versions')"
  KEY_OBJECT_ID_COUNT="$(json_string_or_empty "$KEYS_MANIFEST" '.counts.object_ids')"
  KEY_EVENT_TYPE_COUNT="$(json_string_or_empty "$KEYS_MANIFEST" '.counts.event_types')"
fi

if [[ -z "$DATASET_NETWORK" ]]; then
  DATASET_NETWORK="unknown"
fi

if [[ -z "$DATASET_SOURCE" ]]; then
  DATASET_SOURCE="Benchmark reports"
fi

TRANSACTIONS_COUNT="${rocksdb_tx_count:-$toplingdb_tx_count}"
EVENTS_COUNT="${rocksdb_event_count:-$toplingdb_event_count}"
OBJECT_VERSIONS_COUNT="${rocksdb_object_version_count:-$toplingdb_object_version_count}"
OWNER_ADDRESSES_COUNT="${rocksdb_owner_touched_count:-$toplingdb_owner_touched_count}"

BACKEND_MATCH=""
if [[ -n "$rocksdb_compare_match" ]]; then
  BACKEND_MATCH="$rocksdb_compare_match"
elif [[ -n "$toplingdb_compare_match" ]]; then
  BACKEND_MATCH="$toplingdb_compare_match"
fi

if [[ -z "$BACKEND_MATCH" && -n "$rocksdb_checksum" && -n "$toplingdb_checksum" ]]; then
  if [[ "$rocksdb_checksum" == "$toplingdb_checksum" ]]; then
    BACKEND_MATCH="true"
  else
    BACKEND_MATCH="false"
  fi
fi

mkdir -p "$(dirname "$OUTPUT_PATH")"

{
  cat <<EOF
# Sui HotStore Benchmark Summary

## Scope

- Benchmark target: Sui HotStore DB-level workloads on a bounded dataset
- Backends:
  - RocksDB baseline
  - ToplingDB
- Status:
  - DB-level benchmark: in scope
  - API-level benchmark: fill after DB benchmark is stable
- Summary concurrency selection: ${PICK_CONCURRENCY}

## Dataset

- Source: $(md_escape "$DATASET_SOURCE")
- Network: $(md_escape "$DATASET_NETWORK")
- Checkpoint range: $(md_escape "$DATASET_CHECKPOINT_RANGE")
- Transactions: $(md_escape "$TRANSACTIONS_COUNT")
- Events: $(md_escape "$EVENTS_COUNT")
- Object versions: $(md_escape "$OBJECT_VERSIONS_COUNT")
- Owner addresses: $(md_escape "$OWNER_ADDRESSES_COUNT")
- Key files:
  - \`$(md_escape "$KEY_TX_PATH")\`$( [[ -n "$KEY_TX_COUNT" ]] && printf ' (%s)' "$KEY_TX_COUNT" )
  - \`$(md_escape "$KEY_OBJECT_VERSIONS_PATH")\`$( [[ -n "$KEY_OBJECT_VERSION_COUNT" ]] && printf ' (%s)' "$KEY_OBJECT_VERSION_COUNT" )
  - \`$(md_escape "$KEY_OBJECT_IDS_PATH")\`$( [[ -n "$KEY_OBJECT_ID_COUNT" ]] && printf ' (%s)' "$KEY_OBJECT_ID_COUNT" )
  - \`$(md_escape "$KEY_EVENT_TYPES_PATH")\`$( [[ -n "$KEY_EVENT_TYPE_COUNT" ]] && printf ' (%s)' "$KEY_EVENT_TYPE_COUNT" )
- Backend data equality:
  - RocksDB checksum: \`$(md_escape "$rocksdb_checksum")\`
  - ToplingDB checksum: \`$(md_escape "$toplingdb_checksum")\`
  - Match: $(md_escape "$BACKEND_MATCH")

## Hardware

- CPU:
- Memory:
- Disk:
- OS:
- Test date:
- Cache state:
- Notes:

## DB-Level Results

| Workload | Backend | Concurrency | Requests | Throughput RPS | p50 ms | p95 ms | p99 ms | p999 ms | Errors |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
EOF

  workloads=(
    "get-tx"
    "get-object-version"
    "get-object-last-seen"
    "multi-get-tx"
    "multi-get-object-version"
    "scan-events"
    "mixed-rpc"
  )

  for workload in "${workloads[@]}"; do
    if [[ -n "$ROCKSDB_REPORT_DIR" ]]; then
      emit_workload_row "$workload" "$ROCKSDB_LABEL" "$ROCKSDB_REPORT_DIR" rocksdb
    else
      printf '| %s | %s |  |  |  |  |  |  |  |  |\n' "$workload" "$ROCKSDB_LABEL"
    fi

    if [[ -n "$TOPLINGDB_REPORT_DIR" ]]; then
      emit_workload_row "$workload" "$TOPLINGDB_LABEL" "$TOPLINGDB_REPORT_DIR" toplingdb
    else
      printf '| %s | %s |  |  |  |  |  |  |  |  |\n' "$workload" "$TOPLINGDB_LABEL"
    fi
  done

  cat <<EOF

## Disk Usage

| Backend | Disk usage bytes |
|---|---:|
| RocksDB | $(md_escape "$rocksdb_disk_usage") |
| ToplingDB | $(md_escape "$toplingdb_disk_usage") |

## Source Reports

- RocksDB report dir: $(md_escape "$ROCKSDB_REPORT_DIR")
- ToplingDB report dir: $(md_escape "$TOPLINGDB_REPORT_DIR")
- Keys manifest: $(md_escape "$KEYS_MANIFEST")
- RocksDB DB path: $(md_escape "$rocksdb_db_path")
- ToplingDB DB path: $(md_escape "$toplingdb_db_path")

## Observations

- Data equality:
- Point lookup:
- Multi-get:
- Prefix scan:
- Mixed RPC:
- Tail latency:
- Disk footprint:
- Bottlenecks:

## Caveats

- This benchmark uses a bounded Sui dataset, not a full-history archive.
- \`object_last_seen\` means latest observed within the imported range.
- API benchmark results are intentionally omitted until DB-level results are stable and repeatable.
- Backend comparisons are hardware-specific and preliminary unless noted otherwise.
EOF
} > "$OUTPUT_PATH"

echo "wrote summary: $OUTPUT_PATH"
