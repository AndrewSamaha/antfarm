#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  ./antfarm s3 pull [--dryrun] [--delete]
  ./antfarm s3 push [--dryrun] [--delete]
  ./antfarm s3 status

Commands:
  pull
    Sync experiments/ from s3://antfarm/experiments/

  push
    Sync experiments/ to s3://antfarm/experiments/

  status
    Show local dirty state plus local and remote sync-state metadata.
EOF
}

print_sync_state_details() {
  local label="$1"
  local path="$2"

  python3 - "$label" "$path" <<'PY'
import json
import pathlib
import sys

label = sys.argv[1]
path = pathlib.Path(sys.argv[2])
if not path.exists():
    print(f"{label}: unavailable")
    raise SystemExit(0)

data = json.loads(path.read_text())
print(f"{label}:")
print(f"  pushed_at: {data.get('pushed_at', 'unknown')}")
print(f"  branch: {data.get('branch', 'unknown')}")
print(f"  commit: {data.get('commit', 'unknown')}")
latest_run = data.get("latest_run")
if latest_run:
    condition = latest_run.get("condition")
    run_label = latest_run.get("run_id", "unknown")
    if condition:
        run_label = f"{latest_run.get('experiment', 'unknown')}/{condition}/{run_label}"
    else:
        run_label = f"{latest_run.get('experiment', 'unknown')}/{run_label}"
    print(f"  latest_run: {run_label}")
    print(f"  server_yaml_sha256: {latest_run.get('server_yaml_sha256', 'unknown')}")
    seed = latest_run.get("randomized_seed")
    if seed is not None:
        print(f"  randomized_seed: {seed}")
else:
    print("  latest_run: none")
PY
}

print_status() {
  ensure_experiments_dir

  local local_dirty="no"
  local git_dirty="no"
  if [[ -f "$EXPERIMENTS_UNPUSHED_MARKER" ]]; then
    local_dirty="yes"
  fi
  if git_worktree_is_dirty; then
    git_dirty="yes"
  fi

  echo "Experiment S3 Status"
  echo "  experiments_dir: $EXPERIMENTS_DIR"
  echo "  s3_uri: $EXPERIMENTS_S3_URI"
  echo "  current_branch: $(git_branch_name)"
  echo "  current_commit: $(git_commit_id)"
  echo "  git_worktree_dirty: $git_dirty"
  echo "  local_unpushed_data: $local_dirty"

  print_sync_state_details "local_sync_state" "$EXPERIMENTS_SYNC_STATE_PATH"

  local remote_tmp
  remote_tmp="$(mktemp)"
  trap 'rm -f "$remote_tmp"' RETURN
  if command -v aws >/dev/null 2>&1 && fetch_remote_sync_state_to "$remote_tmp"; then
    print_sync_state_details "remote_sync_state" "$remote_tmp"
    if [[ -f "$EXPERIMENTS_SYNC_STATE_PATH" ]] && cmp -s "$EXPERIMENTS_SYNC_STATE_PATH" "$remote_tmp"; then
      echo "  sync_state_match: yes"
    else
      echo "  sync_state_match: no"
    fi
  else
    echo "remote_sync_state: unavailable"
    echo "  reason: unable to fetch $EXPERIMENTS_S3_URI.sync_state.json"
  fi

  if [[ "$local_dirty" == "yes" ]]; then
    echo "summary: local experiment artifacts have unpushed changes"
  else
    echo "summary: no local unpushed marker present"
  fi
}

if [[ $# -lt 1 ]]; then
  usage
  exit 1
fi

subcommand="$1"
shift

case "$subcommand" in
  pull)
    exec "$ROOT_DIR/scripts/sync-experiments-pull.sh" "$@"
    ;;
  push)
    exec "$ROOT_DIR/scripts/sync-experiments-push.sh" "$@"
    ;;
  status)
    if [[ $# -gt 0 ]]; then
      echo "unexpected arguments for status: $*" >&2
      usage
      exit 1
    fi
    print_status
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    echo "unknown s3 subcommand: $subcommand" >&2
    usage
    exit 1
    ;;
esac
