#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  ./antfarm visualizations --server-config <path-or-dir>

Examples:
  ./antfarm visualizations --server-config ./experiments/experiment-4
  ./antfarm visualizations --server-config ./experiments/experiment-4/server.yaml
EOF
}

server_config=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --server-config)
      [[ $# -ge 2 ]] || { echo "--server-config requires a value" >&2; exit 1; }
      server_config="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$server_config" ]]; then
  echo "--server-config is required" >&2
  usage
  exit 1
fi

if [[ -d "$server_config" ]]; then
  server_config="${server_config%/}/server.yaml"
fi

experiment_dir="$(cd "$(dirname "$server_config")" && pwd)"
tmp_json="$(mktemp)"
trap 'rm -f "$tmp_json"' EXIT
export UV_CACHE_DIR="$ROOT_DIR/.cache/uv"
export MPLCONFIGDIR="$ROOT_DIR/.cache/matplotlib"

"$ROOT_DIR/scripts/server.sh" --server-config "$server_config" --print-visualizations-json > "$tmp_json"

uv run --project analysis python -m antfarm_visualizations.cli \
  --experiment-dir "$experiment_dir" \
  --visualizations-json "$tmp_json"

uv run --project analysis python -m antfarm_aggregation.cli \
  --experiment-dir "$experiment_dir"

if is_experiment_server_config "$server_config"; then
  mark_experiments_dirty
fi
