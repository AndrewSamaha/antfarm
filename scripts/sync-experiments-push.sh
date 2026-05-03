#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

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

if [[ ! -d experiments ]]; then
  echo "missing experiments directory: experiments/" >&2
  exit 1
fi

src="experiments/"
dst="s3://antfarm/experiments/"

echo "Syncing $src -> $dst"
exec aws s3 sync "$src" "$dst" "${extra_args[@]}"
