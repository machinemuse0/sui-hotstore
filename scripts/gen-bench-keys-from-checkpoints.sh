#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/gen-bench-keys-from-checkpoints.sh \
    --network <mainnet|testnet|devnet> \
    [--first-checkpoint <N> --last-checkpoint <N> | --latest-count <N>] \
    [--rpc-url <url>] \
    [--out-dir <path>] \
    [--tx-batch-size <N>] \
    [--rpc-timeout-secs <N>]

Purpose:
  Fetch a bounded Sui checkpoint range over JSON-RPC and generate the UTF-8 key
  files expected by hotstore-bench:
    - tx_digests.txt
    - object_versions.txt
    - object_ids.txt
    - event_types.txt

Defaults:
  - network: testnet
  - rpc-url: derived from --network
  - out-dir:
      - ./bench/keys/<network>-<first>-<last> when using explicit range
      - ./bench/keys/<network>-latest-<count> when using --latest-count
  - tx-batch-size: 50
  - rpc-timeout-secs: 30

Examples:
  scripts/gen-bench-keys-from-checkpoints.sh \
    --network testnet \
    --first-checkpoint 331445801 \
    --last-checkpoint 331445803

  scripts/gen-bench-keys-from-checkpoints.sh \
    --network mainnet \
    --latest-count 10000 \
    --out-dir /private/tmp/hotstore-bench-keys-mainnet
EOF
}

NETWORK="testnet"
RPC_URL=""
FIRST_CHECKPOINT=""
LAST_CHECKPOINT=""
LATEST_COUNT=""
OUT_DIR=""
TX_BATCH_SIZE="50"
RPC_TIMEOUT_SECS="30"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      NETWORK="${2:?missing value for --network}"
      shift 2
      ;;
    --rpc-url)
      RPC_URL="${2:?missing value for --rpc-url}"
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
    --latest-count)
      LATEST_COUNT="${2:?missing value for --latest-count}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:?missing value for --out-dir}"
      shift 2
      ;;
    --tx-batch-size)
      TX_BATCH_SIZE="${2:?missing value for --tx-batch-size}"
      shift 2
      ;;
    --rpc-timeout-secs)
      RPC_TIMEOUT_SECS="${2:?missing value for --rpc-timeout-secs}"
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

if [[ -n "$LATEST_COUNT" && ( -n "$FIRST_CHECKPOINT" || -n "$LAST_CHECKPOINT" ) ]]; then
  echo "Use either --latest-count or --first-checkpoint/--last-checkpoint, not both" >&2
  usage >&2
  exit 1
fi

if [[ -z "$LATEST_COUNT" && ( -z "$FIRST_CHECKPOINT" || -z "$LAST_CHECKPOINT" ) ]]; then
  echo "Either --latest-count or both --first-checkpoint and --last-checkpoint are required" >&2
  usage >&2
  exit 1
fi

case "$NETWORK" in
  mainnet|testnet|devnet)
    ;;
  *)
    echo "Unsupported network: $NETWORK (expected mainnet, testnet, or devnet)" >&2
    exit 1
    ;;
esac

if [[ -n "$LATEST_COUNT" && ( ! "$LATEST_COUNT" =~ ^[0-9]+$ || "$LATEST_COUNT" -lt 1 ) ]]; then
  echo "--latest-count must be an integer >= 1" >&2
  exit 1
fi

if [[ -n "$FIRST_CHECKPOINT" && ! "$FIRST_CHECKPOINT" =~ ^[0-9]+$ ]] || [[ -n "$LAST_CHECKPOINT" && ! "$LAST_CHECKPOINT" =~ ^[0-9]+$ ]]; then
  echo "--first-checkpoint and --last-checkpoint must be integers" >&2
  exit 1
fi

if [[ -n "$FIRST_CHECKPOINT" && -n "$LAST_CHECKPOINT" ]] && (( LAST_CHECKPOINT < FIRST_CHECKPOINT )); then
  echo "--last-checkpoint must be >= --first-checkpoint" >&2
  exit 1
fi

if [[ ! "$TX_BATCH_SIZE" =~ ^[0-9]+$ || "$TX_BATCH_SIZE" -lt 1 || "$TX_BATCH_SIZE" -gt 50 ]]; then
  echo "--tx-batch-size must be an integer in [1, 50] because public Sui RPC limits sui_multiGetTransactionBlocks to 50 digests" >&2
  exit 1
fi

if [[ ! "$RPC_TIMEOUT_SECS" =~ ^[0-9]+$ || "$RPC_TIMEOUT_SECS" -lt 1 ]]; then
  echo "--rpc-timeout-secs must be an integer >= 1" >&2
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

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd jq
require_cmd mktemp
require_cmd sort
require_cmd wc
require_cmd sed

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/hotstore-bench-keys.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

TX_RAW="$TMP_DIR/tx_digests.raw"
OBJECT_VERSIONS_RAW="$TMP_DIR/object_versions.raw"
OBJECT_IDS_RAW="$TMP_DIR/object_ids.raw"
EVENT_TYPES_RAW="$TMP_DIR/event_types.raw"

: > "$TX_RAW"
: > "$OBJECT_VERSIONS_RAW"
: > "$OBJECT_IDS_RAW"
: > "$EVENT_TYPES_RAW"

rpc_post() {
  local payload="$1"
  local output_path="$2"
  curl --max-time "$RPC_TIMEOUT_SECS" -fsS "$RPC_URL" \
    -H 'Content-Type: application/json' \
    -d "$payload" \
    -o "$output_path"
}

resolve_latest_checkpoint() {
  local output_path payload latest
  output_path="$TMP_DIR/latest-checkpoint.json"
  payload='{"jsonrpc":"2.0","id":1,"method":"sui_getLatestCheckpointSequenceNumber","params":[]}'
  if ! rpc_post "$payload" "$output_path"; then
    echo "failed to fetch latest checkpoint from ${RPC_URL} within ${RPC_TIMEOUT_SECS}s" >&2
    return 1
  fi
  jq -e '.error == null and .result != null' "$output_path" >/dev/null || {
    echo "RPC returned no latest checkpoint result" >&2
    jq . "$output_path" >&2 || true
    return 1
  }
  latest="$(jq -r '.result' "$output_path")"
  if [[ ! "$latest" =~ ^[0-9]+$ ]]; then
    echo "latest checkpoint result is not an integer: $latest" >&2
    return 1
  fi
  LAST_CHECKPOINT="$latest"
  FIRST_CHECKPOINT=$(( LAST_CHECKPOINT - LATEST_COUNT + 1 ))
  if (( FIRST_CHECKPOINT < 0 )); then
    FIRST_CHECKPOINT=0
  fi
}

fetch_checkpoint() {
  local checkpoint_seq="$1"
  local output_path="$2"
  local payload
  payload="$(jq -nc --arg seq "$checkpoint_seq" \
    '{jsonrpc:"2.0", id:1, method:"sui_getCheckpoint", params:[$seq]}')"
  if ! rpc_post "$payload" "$output_path"; then
    echo "failed to fetch checkpoint ${checkpoint_seq} from ${RPC_URL} within ${RPC_TIMEOUT_SECS}s" >&2
    return 1
  fi
  jq -e '.error == null and .result != null' "$output_path" >/dev/null || {
    echo "RPC returned no checkpoint result for ${checkpoint_seq}" >&2
    jq . "$output_path" >&2 || true
    return 1
  }
}

fetch_tx_batch() {
  local batch_file="$1"
  local output_path="$2"
  local digests_json payload
  digests_json="$(jq -Rsc 'split("\n") | map(select(length > 0))' "$batch_file")"
  payload="$(jq -nc \
    --argjson digests "$digests_json" \
    '{
      jsonrpc:"2.0",
      id:1,
      method:"sui_multiGetTransactionBlocks",
      params:[
        $digests,
        {
          showInput:true,
          showEffects:true,
          showEvents:true,
          showObjectChanges:true
        }
      ]
    }')"
  if ! rpc_post "$payload" "$output_path"; then
    echo "failed to fetch transaction detail batch from ${RPC_URL} within ${RPC_TIMEOUT_SECS}s" >&2
    return 1
  fi
  jq -e '.error == null and .result != null' "$output_path" >/dev/null || {
    echo "RPC returned no transaction batch result" >&2
    jq . "$output_path" >&2 || true
    return 1
  }
}

if [[ -n "$LATEST_COUNT" ]]; then
  echo "Resolving latest checkpoint from ${RPC_URL}"
  resolve_latest_checkpoint
fi

if [[ -z "$OUT_DIR" ]]; then
  if [[ -n "$LATEST_COUNT" ]]; then
    OUT_DIR="$(pwd)/bench/keys/${NETWORK}-latest-${LATEST_COUNT}"
  else
    OUT_DIR="$(pwd)/bench/keys/${NETWORK}-${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
  fi
fi

mkdir -p "$OUT_DIR"

TX_OUT="$OUT_DIR/tx_digests.txt"
OBJECT_VERSIONS_OUT="$OUT_DIR/object_versions.txt"
OBJECT_IDS_OUT="$OUT_DIR/object_ids.txt"
EVENT_TYPES_OUT="$OUT_DIR/event_types.txt"
MANIFEST_OUT="$OUT_DIR/manifest.json"

echo "Generating benchmark keys from ${NETWORK} checkpoints ${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}"
echo "RPC URL: ${RPC_URL}"
echo "Output dir: ${OUT_DIR}"
echo "RPC timeout secs: ${RPC_TIMEOUT_SECS}"

for (( checkpoint = FIRST_CHECKPOINT; checkpoint <= LAST_CHECKPOINT; checkpoint++ )); do
  checkpoint_json="$TMP_DIR/checkpoint-${checkpoint}.json"
  echo "Fetching checkpoint ${checkpoint}"
  fetch_checkpoint "$checkpoint" "$checkpoint_json"
  jq -r '.result.transactions[]?' "$checkpoint_json" >> "$TX_RAW"
done

sort -u "$TX_RAW" > "$TX_OUT"

TX_COUNT="$(wc -l < "$TX_OUT" | tr -d '[:space:]')"
if [[ "$TX_COUNT" == "0" ]]; then
  echo "No transaction digests were found in the requested checkpoint range" >&2
  exit 1
fi

echo "Fetched ${TX_COUNT} unique transaction digests"

tx_total_lines="$TX_COUNT"
batch_start=1
batch_index=0
while (( batch_start <= tx_total_lines )); do
  batch_end=$(( batch_start + TX_BATCH_SIZE - 1 ))
  batch_file="$TMP_DIR/tx-batch-${batch_index}.txt"
  batch_json="$TMP_DIR/tx-batch-${batch_index}.json"

  sed -n "${batch_start},${batch_end}p" "$TX_OUT" > "$batch_file"
  echo "Fetching transaction details batch $((batch_index + 1)) (lines ${batch_start}-${batch_end})"
  fetch_tx_batch "$batch_file" "$batch_json"

  jq -r '
    .result[]?
    | .objectChanges[]?
    | select(.objectId != null and .version != null)
    | "\(.objectId),\(.version)"
  ' "$batch_json" >> "$OBJECT_VERSIONS_RAW"

  jq -r '
    .result[]?
    | .objectChanges[]?
    | select(.objectId != null)
    | .objectId
  ' "$batch_json" >> "$OBJECT_IDS_RAW"

  jq -r '
    .result[]?
    | .events[]?
    | select(.type != null)
    | .type
  ' "$batch_json" >> "$EVENT_TYPES_RAW"

  batch_start=$(( batch_end + 1 ))
  batch_index=$(( batch_index + 1 ))
done

sort -u "$OBJECT_VERSIONS_RAW" > "$OBJECT_VERSIONS_OUT"
sort -u "$OBJECT_IDS_RAW" > "$OBJECT_IDS_OUT"
sort -u "$EVENT_TYPES_RAW" > "$EVENT_TYPES_OUT"

OBJECT_VERSION_COUNT="$(wc -l < "$OBJECT_VERSIONS_OUT" | tr -d '[:space:]')"
OBJECT_ID_COUNT="$(wc -l < "$OBJECT_IDS_OUT" | tr -d '[:space:]')"
EVENT_TYPE_COUNT="$(wc -l < "$EVENT_TYPES_OUT" | tr -d '[:space:]')"

if [[ "$OBJECT_VERSION_COUNT" == "0" ]]; then
  echo "No object version keys were produced; widen the checkpoint range" >&2
  exit 1
fi

if [[ "$OBJECT_ID_COUNT" == "0" ]]; then
  echo "No object id keys were produced; widen the checkpoint range" >&2
  exit 1
fi

if [[ "$EVENT_TYPE_COUNT" == "0" ]]; then
  echo "No event type keys were produced; widen the checkpoint range so scan-events can run" >&2
  exit 1
fi

jq -nc \
  --arg network "$NETWORK" \
  --arg rpc_url "$RPC_URL" \
  --argjson first_checkpoint "$FIRST_CHECKPOINT" \
  --argjson last_checkpoint "$LAST_CHECKPOINT" \
  --argjson tx_count "$TX_COUNT" \
  --argjson object_version_count "$OBJECT_VERSION_COUNT" \
  --argjson object_id_count "$OBJECT_ID_COUNT" \
  --argjson event_type_count "$EVENT_TYPE_COUNT" \
  --arg out_dir "$OUT_DIR" \
  '{
    network: $network,
    rpc_url: $rpc_url,
    first_checkpoint: $first_checkpoint,
    last_checkpoint: $last_checkpoint,
    generated_files: {
      tx_digests: ($out_dir + "/tx_digests.txt"),
      object_versions: ($out_dir + "/object_versions.txt"),
      object_ids: ($out_dir + "/object_ids.txt"),
      event_types: ($out_dir + "/event_types.txt")
    },
    counts: {
      tx_digests: $tx_count,
      object_versions: $object_version_count,
      object_ids: $object_id_count,
      event_types: $event_type_count
    }
  }' > "$MANIFEST_OUT"

echo "Generated benchmark key files:"
echo "  ${TX_OUT}"
echo "  ${OBJECT_VERSIONS_OUT}"
echo "  ${OBJECT_IDS_OUT}"
echo "  ${EVENT_TYPES_OUT}"
echo "Manifest:"
echo "  ${MANIFEST_OUT}"
