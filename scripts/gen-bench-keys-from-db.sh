#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/gen-bench-keys-from-db.sh \
    --backend <rocksdb|toplingdb> \
    --db-path <path> \
    [--out-dir <path>] \
    [--tx-limit <N>] \
    [--object-version-limit <N>] \
    [--object-id-limit <N>] \
    [--event-type-limit <N>] \
    [--cargo-profile <dev|release>] \
    [--output <path>]

Purpose:
  Read an existing HotStore DB and export benchmark key files directly from the
  stored records, without re-ingesting chain data.

Defaults:
  - out-dir: ./bench/keys/<backend>-from-db
  - tx-limit: 100000
  - object-version-limit: 100000
  - object-id-limit: 100000
  - event-type-limit: 1000
  - cargo-profile: release

Generated files:
  tx_digests.txt
  object_versions.txt
  object_ids.txt
  event_types.txt
  manifest.json
EOF
}

BACKEND=""
DB_PATH=""
OUT_DIR=""
TX_LIMIT="100000"
OBJECT_VERSION_LIMIT="100000"
OBJECT_ID_LIMIT="100000"
EVENT_TYPE_LIMIT="1000"
CARGO_PROFILE="release"
OUTPUT_PATH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --backend)
      BACKEND="${2:?missing value for --backend}"
      shift 2
      ;;
    --db-path)
      DB_PATH="${2:?missing value for --db-path}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:?missing value for --out-dir}"
      shift 2
      ;;
    --tx-limit)
      TX_LIMIT="${2:?missing value for --tx-limit}"
      shift 2
      ;;
    --object-version-limit)
      OBJECT_VERSION_LIMIT="${2:?missing value for --object-version-limit}"
      shift 2
      ;;
    --object-id-limit)
      OBJECT_ID_LIMIT="${2:?missing value for --object-id-limit}"
      shift 2
      ;;
    --event-type-limit)
      EVENT_TYPE_LIMIT="${2:?missing value for --event-type-limit}"
      shift 2
      ;;
    --cargo-profile)
      CARGO_PROFILE="${2:?missing value for --cargo-profile}"
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

if [[ -z "$BACKEND" || -z "$DB_PATH" ]]; then
  echo "--backend and --db-path are required" >&2
  usage >&2
  exit 1
fi

if [[ "$BACKEND" != "rocksdb" && "$BACKEND" != "toplingdb" ]]; then
  echo "--backend must be rocksdb or toplingdb" >&2
  exit 1
fi

if [[ ! -d "$DB_PATH" ]]; then
  echo "db path does not exist: $DB_PATH" >&2
  exit 1
fi

if [[ "$CARGO_PROFILE" != "dev" && "$CARGO_PROFILE" != "release" ]]; then
  echo "--cargo-profile must be dev or release" >&2
  exit 1
fi

for value_name in TX_LIMIT OBJECT_VERSION_LIMIT OBJECT_ID_LIMIT EVENT_TYPE_LIMIT; do
  value="${!value_name}"
  if [[ ! "$value" =~ ^[0-9]+$ || "$value" -lt 1 ]]; then
    echo "${value_name} must be an integer >= 1" >&2
    exit 1
  fi
done

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/bench/keys/${BACKEND}-from-db"
fi

if [[ "$CARGO_PROFILE" == "release" ]]; then
  CARGO_ARGS=(run --release)
else
  CARGO_ARGS=(run)
fi

CMD=(
  cargo
  "${CARGO_ARGS[@]}"
  --bin hotstore-admin
  --
  export-bench-keys
  --backend "$BACKEND"
  --db-path "$DB_PATH"
  --out-dir "$OUT_DIR"
  --tx-limit "$TX_LIMIT"
  --object-version-limit "$OBJECT_VERSION_LIMIT"
  --object-id-limit "$OBJECT_ID_LIMIT"
  --event-type-limit "$EVENT_TYPE_LIMIT"
)

if [[ -n "$OUTPUT_PATH" ]]; then
  CMD+=(--output "$OUTPUT_PATH")
fi

echo "Exporting benchmark keys from existing ${BACKEND} DB"
echo "  db path: ${DB_PATH}"
echo "  out dir: ${OUT_DIR}"
echo "  cargo profile: ${CARGO_PROFILE}"
echo "  tx limit: ${TX_LIMIT}"
echo "  object version limit: ${OBJECT_VERSION_LIMIT}"
echo "  object id limit: ${OBJECT_ID_LIMIT}"
echo "  event type limit: ${EVENT_TYPE_LIMIT}"

"${CMD[@]}"
