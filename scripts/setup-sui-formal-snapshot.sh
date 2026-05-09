#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  setup-sui-formal-snapshot.sh [options]

Purpose:
  Download a Sui formal snapshot, write a local fullnode config, and prepare a
  full node database for fast recovery from snapshot state.

Defaults:
  - network: mainnet
  - epoch: latest
  - source: public formal snapshot bucket via --no-sign-request
  - base dir: ./data/sui-<network>-formal
  - parallel downloads: 50

Examples:
  scripts/setup-sui-formal-snapshot.sh

  scripts/setup-sui-formal-snapshot.sh \
    --network testnet \
    --epoch latest \
    --base-dir /opt/sui/testnet-formal

  scripts/setup-sui-formal-snapshot.sh \
    --network mainnet \
    --epoch 1234 \
    --base-dir /opt/sui/mainnet-formal \
    --snapshot-bucket s3://mysten-mainnet-formal/ \
    --snapshot-bucket-type s3 \
    --start-node

Options:
  --network <mainnet|testnet>
  --epoch <latest|NUMBER>
  --base-dir <path>
  --config-dir <path>
  --db-dir <path>
  --genesis-path <path>
  --fullnode-yaml <path>
  --parallel-downloads <N>
  --snapshot-bucket <url>
  --snapshot-bucket-type <s3|gcs|azure|file|http|https>
  --with-credentials
  --start-node
  --force
  --help

Environment overrides:
  SUI_TOOL_BIN            default: sui-tool
  SUI_NODE_BIN            default: sui-node
  FULLNODE_TEMPLATE_URL   default: official Sui GitHub raw template
EOF
}

NETWORK="mainnet"
EPOCH="latest"
BASE_DIR=""
CONFIG_DIR=""
DB_DIR=""
GENESIS_PATH=""
FULLNODE_YAML=""
PARALLEL_DOWNLOADS="50"
SNAPSHOT_BUCKET=""
SNAPSHOT_BUCKET_TYPE=""
USE_PUBLIC_BUCKET="1"
START_NODE="0"
FORCE="0"

SUI_TOOL_BIN="${SUI_TOOL_BIN:-sui-tool}"
SUI_NODE_BIN="${SUI_NODE_BIN:-sui-node}"
FULLNODE_TEMPLATE_URL="${FULLNODE_TEMPLATE_URL:-https://raw.githubusercontent.com/MystenLabs/sui/main/crates/sui-config/data/fullnode-template.yaml}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      NETWORK="${2:?missing value for --network}"
      shift 2
      ;;
    --epoch)
      EPOCH="${2:?missing value for --epoch}"
      shift 2
      ;;
    --base-dir)
      BASE_DIR="${2:?missing value for --base-dir}"
      shift 2
      ;;
    --config-dir)
      CONFIG_DIR="${2:?missing value for --config-dir}"
      shift 2
      ;;
    --db-dir)
      DB_DIR="${2:?missing value for --db-dir}"
      shift 2
      ;;
    --genesis-path)
      GENESIS_PATH="${2:?missing value for --genesis-path}"
      shift 2
      ;;
    --fullnode-yaml)
      FULLNODE_YAML="${2:?missing value for --fullnode-yaml}"
      shift 2
      ;;
    --parallel-downloads)
      PARALLEL_DOWNLOADS="${2:?missing value for --parallel-downloads}"
      shift 2
      ;;
    --snapshot-bucket)
      SNAPSHOT_BUCKET="${2:?missing value for --snapshot-bucket}"
      shift 2
      ;;
    --snapshot-bucket-type)
      SNAPSHOT_BUCKET_TYPE="${2:?missing value for --snapshot-bucket-type}"
      shift 2
      ;;
    --with-credentials)
      USE_PUBLIC_BUCKET="0"
      shift
      ;;
    --start-node)
      START_NODE="1"
      shift
      ;;
    --force)
      FORCE="1"
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

case "$NETWORK" in
  mainnet|testnet)
    ;;
  *)
    echo "Unsupported network: $NETWORK (expected mainnet or testnet)" >&2
    exit 1
    ;;
esac

if [[ "$EPOCH" != "latest" && ! "$EPOCH" =~ ^[0-9]+$ ]]; then
  echo "--epoch must be 'latest' or a numeric epoch" >&2
  exit 1
fi

if [[ ! "$PARALLEL_DOWNLOADS" =~ ^[0-9]+$ || "$PARALLEL_DOWNLOADS" -lt 1 ]]; then
  echo "--parallel-downloads must be an integer >= 1" >&2
  exit 1
fi

if [[ -z "$BASE_DIR" ]]; then
  BASE_DIR="$(pwd)/data/sui-${NETWORK}-formal"
fi

if [[ -z "$CONFIG_DIR" ]]; then
  CONFIG_DIR="${BASE_DIR}/config"
fi

if [[ -z "$DB_DIR" ]]; then
  DB_DIR="${BASE_DIR}/db"
fi

if [[ -z "$GENESIS_PATH" ]]; then
  GENESIS_PATH="${CONFIG_DIR}/genesis.blob"
fi

if [[ -z "$FULLNODE_YAML" ]]; then
  FULLNODE_YAML="${CONFIG_DIR}/fullnode.yaml"
fi

if [[ "$USE_PUBLIC_BUCKET" == "0" ]]; then
  if [[ -n "$SNAPSHOT_BUCKET" && -z "$SNAPSHOT_BUCKET_TYPE" ]]; then
    echo "--snapshot-bucket-type is required when --snapshot-bucket is set" >&2
    exit 1
  fi
fi

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command not found: $1" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd "$SUI_TOOL_BIN"

mkdir -p "$CONFIG_DIR" "$DB_DIR"

GENESIS_URL=""
case "$NETWORK" in
  mainnet)
    GENESIS_URL="https://github.com/MystenLabs/sui-genesis/raw/main/mainnet/genesis.blob"
    ;;
  testnet)
    GENESIS_URL="https://github.com/MystenLabs/sui-genesis/raw/main/testnet/genesis.blob"
    ;;
esac

download_file() {
  local url="$1"
  local path="$2"
  local tmp="${path}.tmp"
  echo "Downloading ${url}"
  curl -fL "$url" -o "$tmp"
  mv "$tmp" "$path"
}

if [[ "$FORCE" == "1" || ! -f "$GENESIS_PATH" ]]; then
  download_file "$GENESIS_URL" "$GENESIS_PATH"
else
  echo "Reusing existing genesis file: $GENESIS_PATH"
fi

TEMPLATE_PATH="${CONFIG_DIR}/fullnode-template.yaml"
if [[ "$FORCE" == "1" || ! -f "$TEMPLATE_PATH" ]]; then
  download_file "$FULLNODE_TEMPLATE_URL" "$TEMPLATE_PATH"
else
  echo "Reusing existing fullnode template: $TEMPLATE_PATH"
fi

if [[ "$FORCE" == "1" || ! -f "$FULLNODE_YAML" ]]; then
  cp "$TEMPLATE_PATH" "$FULLNODE_YAML"
else
  echo "Reusing existing fullnode config: $FULLNODE_YAML"
fi

if ! grep -q '^db-path:' "$FULLNODE_YAML"; then
  echo "Could not find 'db-path:' in $FULLNODE_YAML" >&2
  exit 1
fi

if ! grep -q 'genesis-file-location:' "$FULLNODE_YAML"; then
  echo "Could not find 'genesis-file-location:' in $FULLNODE_YAML" >&2
  exit 1
fi

sed -i.bak \
  -e "s#^db-path:.*#db-path: \"${DB_DIR}\"#" \
  -e "s#genesis-file-location:.*#genesis-file-location: \"${GENESIS_PATH}\"#" \
  "$FULLNODE_YAML"
rm -f "${FULLNODE_YAML}.bak"

snapshot_cmd=(
  "$SUI_TOOL_BIN"
  download-formal-snapshot
  --genesis "$GENESIS_PATH"
  --network "$NETWORK"
  --path "$DB_DIR"
  --num-parallel-downloads "$PARALLEL_DOWNLOADS"
)

if [[ "$EPOCH" == "latest" ]]; then
  snapshot_cmd+=(--latest)
else
  snapshot_cmd+=(--epoch "$EPOCH")
fi

if [[ "$USE_PUBLIC_BUCKET" == "1" ]]; then
  snapshot_cmd+=(--no-sign-request)
else
  if [[ -n "$SNAPSHOT_BUCKET" ]]; then
    snapshot_cmd+=(--snapshot-bucket "$SNAPSHOT_BUCKET")
  fi
  if [[ -n "$SNAPSHOT_BUCKET_TYPE" ]]; then
    snapshot_cmd+=(--snapshot-bucket-type "$SNAPSHOT_BUCKET_TYPE")
  fi
fi

echo
echo "Formal snapshot restore plan"
echo "  network:            $NETWORK"
echo "  epoch:              $EPOCH"
echo "  config dir:         $CONFIG_DIR"
echo "  db dir:             $DB_DIR"
echo "  genesis path:       $GENESIS_PATH"
echo "  fullnode config:    $FULLNODE_YAML"
echo "  parallel downloads: $PARALLEL_DOWNLOADS"
if [[ "$USE_PUBLIC_BUCKET" == "1" ]]; then
  echo "  snapshot source:    public formal snapshot bucket (--no-sign-request)"
else
  echo "  snapshot source:    credentialed bucket"
  if [[ -n "$SNAPSHOT_BUCKET" ]]; then
    echo "  snapshot bucket:    $SNAPSHOT_BUCKET"
  fi
  if [[ -n "$SNAPSHOT_BUCKET_TYPE" ]]; then
    echo "  bucket type:        $SNAPSHOT_BUCKET_TYPE"
  fi
fi
echo

echo "Running formal snapshot download..."
"${snapshot_cmd[@]}"

echo
echo "Snapshot restore complete."
echo "Fullnode config written to: $FULLNODE_YAML"
echo

start_cmd=( "$SUI_NODE_BIN" --config-path "$FULLNODE_YAML" )

if [[ "$START_NODE" == "1" ]]; then
  require_cmd "$SUI_NODE_BIN"
  echo "Starting Sui node..."
  exec "${start_cmd[@]}"
else
  echo "Start the node with:"
  printf '  '
  printf '%q ' "${start_cmd[@]}"
  printf '\n'
fi
