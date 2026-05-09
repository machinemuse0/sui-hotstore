#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/gen-bench-keys-from-sui-db.sh \
    --db-root <path> \
    --network <mainnet|testnet|devnet> \
    --first-checkpoint <N> \
    --last-checkpoint <N> \
    [--out-dir <path>] \
    [--rpc-url <url>] \
    [--key-rpc-url <url>] \
    [--config-path <path>] \
    [--sui-node-bin <path>] \
    [--start-node] \
    [--rpc-timeout-secs <N>]

Purpose:
  Generate benchmark key files from an existing Sui fullnode / formal snapshot
  DB by querying a local JSON-RPC endpoint backed by that DB.

Modes:
  1. Reuse an already-running local fullnode:
     pass --rpc-url only
  2. Start a local fullnode from the supplied DB root:
     pass --start-node and optionally --config-path / --sui-node-bin
  3. Keep DB context local, but fetch keys from another RPC:
     pass --key-rpc-url for checkpoint/transaction queries

Defaults:
  - rpc-url: http://127.0.0.1:9000
  - key-rpc-url: same as --rpc-url
  - config-path: <db-root>/config/fullnode.yaml
  - out-dir: ./bench/keys/<network>-<first>-<last>-from-sui-db
  - rpc-timeout-secs: 30

Generated files:
  tx_digests.txt
  object_versions.txt
  object_ids.txt
  event_types.txt
  manifest.json
EOF
}

DB_ROOT=""
NETWORK=""
FIRST_CHECKPOINT=""
LAST_CHECKPOINT=""
OUT_DIR=""
RPC_URL="http://127.0.0.1:9000"
KEY_RPC_URL=""
CONFIG_PATH=""
SUI_NODE_BIN="${SUI_NODE_BIN:-sui-node}"
START_NODE="0"
RPC_TIMEOUT_SECS="30"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --db-root)
      DB_ROOT="${2:?missing value for --db-root}"
      shift 2
      ;;
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
    --out-dir)
      OUT_DIR="${2:?missing value for --out-dir}"
      shift 2
      ;;
    --rpc-url)
      RPC_URL="${2:?missing value for --rpc-url}"
      shift 2
      ;;
    --key-rpc-url)
      KEY_RPC_URL="${2:?missing value for --key-rpc-url}"
      shift 2
      ;;
    --config-path)
      CONFIG_PATH="${2:?missing value for --config-path}"
      shift 2
      ;;
    --sui-node-bin)
      SUI_NODE_BIN="${2:?missing value for --sui-node-bin}"
      shift 2
      ;;
    --start-node)
      START_NODE="1"
      shift
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

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd jq

if [[ -z "$DB_ROOT" || -z "$NETWORK" || -z "$FIRST_CHECKPOINT" || -z "$LAST_CHECKPOINT" ]]; then
  echo "--db-root, --network, --first-checkpoint, and --last-checkpoint are required" >&2
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

if [[ ! -d "$DB_ROOT" ]]; then
  echo "db root does not exist: $DB_ROOT" >&2
  exit 1
fi

if [[ -z "$CONFIG_PATH" ]]; then
  CONFIG_PATH="${DB_ROOT}/config/fullnode.yaml"
fi

if [[ ! "$FIRST_CHECKPOINT" =~ ^[0-9]+$ || ! "$LAST_CHECKPOINT" =~ ^[0-9]+$ ]]; then
  echo "--first-checkpoint and --last-checkpoint must be integers" >&2
  exit 1
fi

if (( LAST_CHECKPOINT < FIRST_CHECKPOINT )); then
  echo "--last-checkpoint must be >= --first-checkpoint" >&2
  exit 1
fi

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/bench/keys/${NETWORK}-${FIRST_CHECKPOINT}-${LAST_CHECKPOINT}-from-sui-db"
fi

if [[ -z "$KEY_RPC_URL" ]]; then
  KEY_RPC_URL="$RPC_URL"
fi

if [[ ! "$RPC_TIMEOUT_SECS" =~ ^[0-9]+$ || "$RPC_TIMEOUT_SECS" -lt 1 ]]; then
  echo "--rpc-timeout-secs must be an integer >= 1" >&2
  exit 1
fi

NODE_PID=""
NODE_LOG=""

cleanup() {
  if [[ -n "$NODE_PID" ]]; then
    kill "$NODE_PID" >/dev/null 2>&1 || true
    wait "$NODE_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

rpc_ready() {
  local response_file
  response_file="$(mktemp "${TMPDIR:-/tmp}/sui-rpc-ready.XXXXXX")"
  if curl -fsS "$RPC_URL" \
    --max-time "$RPC_TIMEOUT_SECS" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"sui_getLatestCheckpointSequenceNumber","params":[]}' \
    -o "$response_file" >/dev/null 2>&1; then
    if jq -e '.result != null and .error == null' "$response_file" >/dev/null 2>&1; then
      rm -f "$response_file"
      return 0
    fi
  fi
  rm -f "$response_file"
  return 1
}

if [[ "$START_NODE" == "1" ]]; then
  if [[ ! -f "$CONFIG_PATH" ]]; then
    echo "config path does not exist: $CONFIG_PATH" >&2
    exit 1
  fi

  require_cmd "$SUI_NODE_BIN"
  NODE_LOG="$(mktemp "${TMPDIR:-/tmp}/sui-node-bench.XXXXXX.log")"
  DB_PARENT="$(dirname "$DB_ROOT")"

  echo "Starting local sui-node from existing DB"
  echo "  config: $CONFIG_PATH"
  echo "  rpc url: $RPC_URL"
  echo "  log: $NODE_LOG"

  (
    cd "$DB_PARENT"
    "$SUI_NODE_BIN" --config-path "$CONFIG_PATH"
  ) >"$NODE_LOG" 2>&1 &
  NODE_PID=$!

  for _ in $(seq 1 90); do
    if rpc_ready; then
      break
    fi
    sleep 2
  done

  if ! rpc_ready; then
    echo "local sui-node did not become ready at $RPC_URL" >&2
    echo "log file: $NODE_LOG" >&2
    exit 1
  fi
else
  if ! rpc_ready; then
    echo "RPC endpoint is not ready: $RPC_URL" >&2
    echo "Either start your local fullnode first, or rerun with --start-node" >&2
    exit 1
  fi
fi

echo "Generating benchmark keys via local Sui RPC"
echo "  db root: $DB_ROOT"
echo "  local rpc url: $RPC_URL"
echo "  key rpc url: $KEY_RPC_URL"
echo "  out dir: $OUT_DIR"
echo "  rpc timeout secs: $RPC_TIMEOUT_SECS"

scripts/gen-bench-keys-from-checkpoints.sh \
  --network "$NETWORK" \
  --rpc-url "$KEY_RPC_URL" \
  --first-checkpoint "$FIRST_CHECKPOINT" \
  --last-checkpoint "$LAST_CHECKPOINT" \
  --out-dir "$OUT_DIR" \
  --rpc-timeout-secs "$RPC_TIMEOUT_SECS"

echo "Done."
echo "Key files: $OUT_DIR"
