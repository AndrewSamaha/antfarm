#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'EOF'
Usage:
  scripts/sync-experiments-pull.sh [--dryrun] [--delete]

Examples:
  scripts/sync-experiments-pull.sh
  scripts/sync-experiments-pull.sh --dryrun

Notes:
  Syncs all experiment data from:
    s3://antfarm/experiments/
  into:
    experiments/

  Use --delete only if you want the local experiments/ tree to mirror S3 exactly.
EOF
}

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

mkdir -p experiments

src="s3://antfarm/experiments/"
dst="experiments/"

echo "Syncing $src -> $dst"
exec aws s3 sync "$src" "$dst" "${extra_args[@]}"
