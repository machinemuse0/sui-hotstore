#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/monitor-benchmark-memory.sh [options] -- <command> [args...]
  scripts/monitor-benchmark-memory.sh [options] --pid <pid>

Options:
  --output-dir <path>   Directory for memory-samples.csv and memory-summary.json.
                        Default: reports/memory/<label>-<timestamp>
  --interval <seconds>  Sampling interval. Default: 1
  --label <name>        Label written to the summary. Default: benchmark
  --pid <pid>           Monitor an existing process tree instead of launching a command.
  -h, --help            Show this help.

The script samples the root process plus all live descendants and records:
  - total RSS / VSZ for the process tree
  - process count
  - root process VmRSS / VmHWM / VmSize when /proc/<pid>/status is available

Example:
  scripts/monitor-benchmark-memory.sh \
    --label rocksdb-snappy \
    --output-dir /data4/.../reports/memory \
    -- scripts/run-benchmark-suite.sh --backend rocksdb ...

Example with prebuilt benchmark binaries:
  scripts/monitor-benchmark-memory.sh \
    --label rocksdb-snappy \
    --interval 1 \
    --output-dir /data4/.../reports/memory \
    -- scripts/run-benchmark-suite.sh \
      --backend rocksdb \
      --db-path /data4/.../db-a \
      --keys-dir /data4/.../keys \
      --report-dir /data4/.../reports \
      --dataset mainnet-270700000-270759999 \
      --requests 1000000 \
      --warmup-requests 100000 \
      --concurrency 1,4,8,16,32,64 \
      --access-pattern uniform \
      --scan-mode count \
      --cache-state hot \
      --min-hit-rate 1.0 \
      --batch-size 10 \
      --bin-dir /data/osc/sui-hotstore/target1/release
EOF
}

LABEL="benchmark"
INTERVAL_SECONDS="1"
OUTPUT_DIR=""
MONITOR_PID=""
COMMAND=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    --interval)
      INTERVAL_SECONDS="${2:-}"
      shift 2
      ;;
    --label)
      LABEL="${2:-}"
      shift 2
      ;;
    --pid)
      MONITOR_PID="${2:-}"
      shift 2
      ;;
    --)
      shift
      COMMAND=("$@")
      break
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

if [[ -n "$MONITOR_PID" && "${#COMMAND[@]}" -gt 0 ]]; then
  echo "--pid cannot be combined with a command" >&2
  exit 1
fi

if [[ -z "$MONITOR_PID" && "${#COMMAND[@]}" -eq 0 ]]; then
  echo "either --pid or a command after -- is required" >&2
  usage >&2
  exit 1
fi

if ! [[ "$INTERVAL_SECONDS" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "--interval must be a positive number" >&2
  exit 1
fi

if [[ "$INTERVAL_SECONDS" == "0" || "$INTERVAL_SECONDS" == "0.0" ]]; then
  echo "--interval must be greater than zero" >&2
  exit 1
fi

timestamp_slug() {
  date -u +"%Y%m%dT%H%M%SZ"
}

now_ms() {
  local value
  value="$(date +%s%3N 2>/dev/null || true)"
  if [[ "$value" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$value"
  else
    printf '%s000\n' "$(date +%s)"
  fi
}

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

collect_process_tree() {
  local root_pid="$1"
  local process_table
  if ! process_table="$(ps -eo pid=,ppid= 2>/dev/null)"; then
    printf '%s\n' "$root_pid"
    return
  fi

  awk -v root="$root_pid" '
    {
      pid = $1
      ppid = $2
      children[ppid] = children[ppid] " " pid
      exists[pid] = 1
    }
    END {
      if (!exists[root]) {
        exit 0
      }
      queue[1] = root
      seen[root] = 1
      count = 1
      out = root
      for (i = 1; i <= count; i++) {
        n = split(children[queue[i]], kids, " ")
        for (j = 1; j <= n; j++) {
          child = kids[j]
          if (child != "" && !seen[child]) {
            seen[child] = 1
            queue[++count] = child
            out = out " " child
          }
        }
      }
      print out
    }
  ' <<<"$process_table"
}

aggregate_memory() {
  local pids="$1"
  if [[ -z "$pids" ]]; then
    printf '0 0 0\n'
    return
  fi

  local pid_csv
  pid_csv="$(tr ' ' ',' <<<"$pids")"
  ps -o pid=,rss=,vsz= -p "$pid_csv" 2>/dev/null | awk '
    {
      count += 1
      rss += $2
      vsz += $3
    }
    END {
      printf "%d %d %d\n", count, rss, vsz
    }
  ' || printf '0 0 0\n'
}

root_proc_status() {
  local root_pid="$1"
  local status_file="/proc/${root_pid}/status"
  if [[ ! -r "$status_file" ]]; then
    printf '0 0 0\n'
    return
  fi

  awk '
    /^VmRSS:/ { rss = $2 }
    /^VmHWM:/ { hwm = $2 }
    /^VmSize:/ { size = $2 }
    END {
      printf "%d %d %d\n", rss, hwm, size
    }
  ' "$status_file"
}

if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="reports/memory/${LABEL}-$(timestamp_slug)"
fi

mkdir -p "$OUTPUT_DIR"

SAMPLES_CSV="${OUTPUT_DIR}/memory-samples.csv"
SUMMARY_JSON="${OUTPUT_DIR}/memory-summary.json"
COMMAND_TXT="${OUTPUT_DIR}/command.txt"

COMMAND_TEXT=""
MODE="pid"
ROOT_PID="$MONITOR_PID"

if [[ "${#COMMAND[@]}" -gt 0 ]]; then
  MODE="command"
  COMMAND_TEXT="${COMMAND[*]}"
  printf '%s\n' "$COMMAND_TEXT" > "$COMMAND_TXT"
  "${COMMAND[@]}" &
  ROOT_PID="$!"
else
  COMMAND_TEXT="pid:${ROOT_PID}"
  printf '%s\n' "$COMMAND_TEXT" > "$COMMAND_TXT"
fi

if ! kill -0 "$ROOT_PID" 2>/dev/null; then
  echo "process is not running: $ROOT_PID" >&2
  exit 1
fi

printf '%s\n' \
  "timestamp_unix_ms,elapsed_ms,root_pid,pid_count,total_rss_kib,total_vsz_kib,total_rss_bytes,total_vsz_bytes,root_vmrss_kib,root_vmhwm_kib,root_vmsize_kib" \
  > "$SAMPLES_CSV"

START_MS="$(now_ms)"
START_UNIX="$(date +%s)"
SAMPLE_COUNT=0
PEAK_RSS_KIB=0
PEAK_VSZ_KIB=0
PEAK_PID_COUNT=0
FINAL_RSS_KIB=0
FINAL_VSZ_KIB=0
STOP_REQUESTED=0

handle_signal() {
  STOP_REQUESTED=1
  if [[ "$MODE" == "command" ]] && kill -0 "$ROOT_PID" 2>/dev/null; then
    kill -TERM "$ROOT_PID" 2>/dev/null || true
  fi
}

trap handle_signal INT TERM

echo "monitoring pid ${ROOT_PID}; writing ${SAMPLES_CSV}" >&2

while kill -0 "$ROOT_PID" 2>/dev/null; do
  CURRENT_MS="$(now_ms)"
  ELAPSED_MS=$((CURRENT_MS - START_MS))
  PIDS="$(collect_process_tree "$ROOT_PID")"
  read -r PID_COUNT RSS_KIB VSZ_KIB < <(aggregate_memory "$PIDS")
  read -r ROOT_VMRSS_KIB ROOT_VMHWM_KIB ROOT_VMSIZE_KIB < <(root_proc_status "$ROOT_PID")

  RSS_BYTES=$((RSS_KIB * 1024))
  VSZ_BYTES=$((VSZ_KIB * 1024))

  printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$CURRENT_MS" \
    "$ELAPSED_MS" \
    "$ROOT_PID" \
    "$PID_COUNT" \
    "$RSS_KIB" \
    "$VSZ_KIB" \
    "$RSS_BYTES" \
    "$VSZ_BYTES" \
    "$ROOT_VMRSS_KIB" \
    "$ROOT_VMHWM_KIB" \
    "$ROOT_VMSIZE_KIB" \
    >> "$SAMPLES_CSV"

  SAMPLE_COUNT=$((SAMPLE_COUNT + 1))
  FINAL_RSS_KIB="$RSS_KIB"
  FINAL_VSZ_KIB="$VSZ_KIB"

  if (( RSS_KIB > PEAK_RSS_KIB )); then
    PEAK_RSS_KIB="$RSS_KIB"
    PEAK_PID_COUNT="$PID_COUNT"
  fi
  if (( VSZ_KIB > PEAK_VSZ_KIB )); then
    PEAK_VSZ_KIB="$VSZ_KIB"
  fi

  if (( STOP_REQUESTED == 1 )); then
    break
  fi

  sleep "$INTERVAL_SECONDS"
done

EXIT_CODE=0
if [[ "$MODE" == "command" ]]; then
  set +e
  wait "$ROOT_PID"
  EXIT_CODE="$?"
  set -e
fi

FINISH_UNIX="$(date +%s)"
ELAPSED_SECONDS=$((FINISH_UNIX - START_UNIX))
COMMAND_JSON="$(json_escape "$COMMAND_TEXT")"
LABEL_JSON="$(json_escape "$LABEL")"

cat > "$SUMMARY_JSON" <<EOF
{
  "label": "${LABEL_JSON}",
  "mode": "${MODE}",
  "command": "${COMMAND_JSON}",
  "root_pid": ${ROOT_PID},
  "started_at_unix": ${START_UNIX},
  "finished_at_unix": ${FINISH_UNIX},
  "elapsed_seconds": ${ELAPSED_SECONDS},
  "interval_seconds": ${INTERVAL_SECONDS},
  "sample_count": ${SAMPLE_COUNT},
  "exit_code": ${EXIT_CODE},
  "peak_rss_kib": ${PEAK_RSS_KIB},
  "peak_rss_bytes": $((PEAK_RSS_KIB * 1024)),
  "peak_vsz_kib": ${PEAK_VSZ_KIB},
  "peak_vsz_bytes": $((PEAK_VSZ_KIB * 1024)),
  "peak_pid_count": ${PEAK_PID_COUNT},
  "final_rss_kib": ${FINAL_RSS_KIB},
  "final_rss_bytes": $((FINAL_RSS_KIB * 1024)),
  "final_vsz_kib": ${FINAL_VSZ_KIB},
  "final_vsz_bytes": $((FINAL_VSZ_KIB * 1024)),
  "samples_csv": "${SAMPLES_CSV}"
}
EOF

echo "memory summary: ${SUMMARY_JSON}" >&2

exit "$EXIT_CODE"
