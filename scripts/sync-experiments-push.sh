#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  scripts/sync-experiments-push.sh <experiment-name> [--dryrun] [--delete]

Examples:
  scripts/sync-experiments-push.sh chamber-ants-1
  scripts/sync-experiments-push.sh chamber-ants-1 --dryrun

Notes:
  Syncs one experiment from:
    experiments/<experiment-name>/
  into:
    s3://antfarm/experiments/<experiment-name>/

  This includes server.yaml files as well as generated artifacts.
  Use --delete only if you want the S3 prefix to mirror local contents exactly.
EOF
}

experiment_name=""
extra_args=()
dryrun_requested=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --dryrun)
      dryrun_requested=1
      extra_args+=("$1")
      shift
      ;;
    --delete)
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

experiment_dir="$(experiment_dir_by_name "$experiment_name")"
if [[ ! -d "$experiment_dir" ]]; then
  echo "missing experiment directory: $experiment_dir" >&2
  exit 1
fi

if [[ $dryrun_requested -eq 0 ]]; then
  write_experiment_sync_state "$experiment_name"
else
  echo "Dry run: leaving local sync markers unchanged" >&2
fi

src="${experiment_dir}/"
dst="${EXPERIMENTS_S3_URI}${experiment_name}/"

echo "Syncing $src -> $dst"
if [[ ${#extra_args[@]} -eq 0 ]]; then
  aws s3 sync "$src" "$dst" --exclude ".unpushed_data"
else
  aws s3 sync "$src" "$dst" --exclude ".unpushed_data" "${extra_args[@]}"
fi
if [[ $dryrun_requested -eq 0 ]]; then
  rm -f "$(experiment_unpushed_marker_path "$experiment_name")"
fi
