#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  scripts/sync-experiments-push.sh [--dryrun] [--delete]

Examples:
  scripts/sync-experiments-push.sh
  scripts/sync-experiments-push.sh --dryrun

Notes:
  Syncs all experiment data from:
    experiments/
  into:
    s3://antfarm/experiments/

  This includes server.yaml files as well as generated artifacts.
  Use --delete only if you want the S3 prefix to mirror local contents exactly.
EOF
}

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
      echo "unexpected argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if ! command -v aws >/dev/null 2>&1; then
  echo "aws CLI not found in PATH" >&2
  exit 1
fi

if [[ ! -d experiments ]]; then
  echo "missing experiments directory: experiments/" >&2
  exit 1
fi

if [[ $dryrun_requested -eq 0 ]]; then
  write_experiments_sync_state
else
  echo "Dry run: leaving local sync markers unchanged" >&2
fi

src="experiments/"
dst="$EXPERIMENTS_S3_URI"

echo "Syncing $src -> $dst"
aws s3 sync "$src" "$dst" --exclude ".unpushed_data" "${extra_args[@]}"
if [[ $dryrun_requested -eq 0 ]]; then
  rm -f "$EXPERIMENTS_UNPUSHED_MARKER"
fi
