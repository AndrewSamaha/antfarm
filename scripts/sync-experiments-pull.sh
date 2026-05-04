#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  scripts/sync-experiments-pull.sh <experiment-name> [--dryrun] [--delete]

Examples:
  scripts/sync-experiments-pull.sh chamber-ants-1
  scripts/sync-experiments-pull.sh chamber-ants-1 --dryrun

Notes:
  Syncs one experiment from:
    s3://antfarm/experiments/<experiment-name>/
  into:
    experiments/<experiment-name>/

  Use --delete only if you want the local experiment directory to mirror S3 exactly.
EOF
}

experiment_name=""
extra_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --dryrun|--delete)
      extra_args+=("$1")
      shift
      ;;
    *)
      if [[ -n "$experiment_name" ]]; then
        echo "unexpected argument: $1" >&2
        usage
        exit 1
      fi
      experiment_name="$1"
      shift
      ;;
  esac
done

if [[ -z "$experiment_name" ]]; then
  echo "missing experiment name" >&2
  usage
  exit 1
fi

if ! command -v aws >/dev/null 2>&1; then
  echo "aws CLI not found in PATH" >&2
  exit 1
fi

ensure_experiments_dir

local_marker="$(experiment_unpushed_marker_path "$experiment_name")"
if [[ -f "$local_marker" ]]; then
  echo "warning: local experiment artifacts may be newer than S3; $local_marker exists" >&2
fi

src="${EXPERIMENTS_S3_URI}${experiment_name}/"
dst="$(experiment_dir_by_name "$experiment_name")/"

echo "Syncing $src -> $dst"
mkdir -p "$dst"
if [[ ${#extra_args[@]} -eq 0 ]]; then
  exec aws s3 sync "$src" "$dst" --exclude ".unpushed_data"
fi
exec aws s3 sync "$src" "$dst" --exclude ".unpushed_data" "${extra_args[@]}"
