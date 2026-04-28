#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

emit_json_log() {
  python3 - "$@" <<'PY'
import json
import sys
import time

event = sys.argv[1]
msg_source = sys.argv[2]
payload = {
    "ts_ms": int(time.time() * 1000),
    "event": event,
    "msg_source": msg_source,
}
for arg in sys.argv[3:]:
    key, value = arg.split("=", 1)
    payload[key] = value
print(json.dumps(payload))
PY
}

usage() {
  cat <<'EOF'
Usage:
  ./antfarm experiment --server-config <path> [--num-runs <n>] [server args...]

Examples:
  ./antfarm experiment --server-config ./experiments/experiment-1/server.yaml --num-runs 10
  ./antfarm experiment --server-config ./experiments/experiment-1 --num-runs 10

Notes:
  The target experiment config should set `experiment.terminate_server_on_completion: true`,
  otherwise each launched server process will continue running and the batch will stall.
EOF
}

num_runs=1
server_config=""
num_runs_override=0
forwarded_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --num-runs)
      [[ $# -ge 2 ]] || { echo "--num-runs requires a value" >&2; exit 1; }
      num_runs="$2"
      num_runs_override=1
      shift 2
      ;;
    --server-config)
      [[ $# -ge 2 ]] || { echo "--server-config requires a value" >&2; exit 1; }
      server_config="$2"
      forwarded_args+=("$1" "$2")
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      forwarded_args+=("$1")
      shift
      ;;
  esac
done

if [[ -z "$server_config" ]]; then
  echo "--server-config is required for experiment runs" >&2
  usage
  exit 1
fi

if ! [[ "$num_runs" =~ ^[1-9][0-9]*$ ]]; then
  echo "--num-runs must be a positive integer" >&2
  exit 1
fi

condition_plan="$("$ROOT_DIR/scripts/server.sh" --server-config "$server_config" --list-condition-plan)"

while IFS= read -r line; do
  if [[ -z "$line" ]]; then
    continue
  fi
  IFS=$'\t' read -r condition_name configured_runs <<<"$line"
  if [[ -z "${configured_runs:-}" ]]; then
    continue
  fi
  run_count="$configured_runs"
  if [[ "$num_runs_override" -eq 1 ]]; then
    run_count="$num_runs"
  fi
  if ! [[ "$run_count" =~ ^[1-9][0-9]*$ ]]; then
    echo "invalid run count in condition plan: $line" >&2
    exit 1
  fi

  for ((run_index = 1; run_index <= run_count; run_index++)); do
    if [[ "$condition_name" == "-" ]]; then
      emit_json_log \
        experiment_run_launching \
        antfarm-experiment \
        "server_config=${server_config}" \
        "run_index=${run_index}" \
        "run_count=${run_count}"
      "$ROOT_DIR/scripts/server.sh" "${forwarded_args[@]}"
    else
      emit_json_log \
        experiment_run_launching \
        antfarm-experiment \
        "server_config=${server_config}" \
        "condition=${condition_name}" \
        "run_index=${run_index}" \
        "run_count=${run_count}"
      "$ROOT_DIR/scripts/server.sh" "${forwarded_args[@]}" --condition "$condition_name"
    fi
  done
done <<<"$condition_plan"
