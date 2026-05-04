#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/experiments_sync_common.sh"

usage() {
  cat <<'EOF'
Usage:
  ./antfarm s3 pull <experiment-name> [--dryrun] [--delete]
  ./antfarm s3 push <experiment-name> [--dryrun] [--delete]
  ./antfarm s3 status
  ./antfarm s3 ls

Commands:
  pull
    Sync one experiment from S3 into experiments/<experiment-name>/

  push
    Sync one experiment from experiments/<experiment-name>/ to S3

  status
    Show one-line local status for every local experiment directory.

  ls
    List remote experiment directories in S3.
EOF
}

print_sync_state_summary() {
  local path="$1"
  python3 - "$path" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
if not path.exists():
    print("-")
    raise SystemExit(0)

data = json.loads(path.read_text())
latest_run = data.get("latest_run") or {}
run_id = latest_run.get("run_id") or "-"
condition = latest_run.get("condition")
if condition:
    run_id = f"{condition}/{run_id}"
commit = data.get("commit", "-")
print(f"{data.get('pushed_at', '-')}" + "|" + commit[:12] + "|" + run_id)
PY
}

print_status() {
  ensure_experiments_dir

  local git_dirty="no"
  if git_worktree_is_dirty; then
    git_dirty="yes"
  fi

  echo "Experiment S3 Status"
  echo "  current_branch: $(git_branch_name)"
  echo "  current_commit: $(git_commit_id)"
  echo "  git_worktree_dirty: $git_dirty"
  echo

  printf "%-24s  %-5s  %-6s  %-24s  %-12s  %s\n" "experiment" "dirty" "remote" "pushed_at" "commit" "latest_run"

  local experiment_name
  while IFS= read -r experiment_name; do
    [[ -n "$experiment_name" ]] || continue

    local experiment_dir marker_path local_state remote_tmp dirty remote summary
    experiment_dir="$(experiment_dir_by_name "$experiment_name")"
    marker_path="$(experiment_unpushed_marker_path "$experiment_name")"
    local_state="$(experiment_sync_state_path "$experiment_name")"
    dirty="no"
    remote="no"
    if [[ -f "$marker_path" ]]; then
      dirty="yes"
    fi
    summary="-|-|-"

    if [[ -f "$local_state" ]]; then
      summary="$(print_sync_state_summary "$local_state")"
    fi

    remote_tmp="$(mktemp)"
    if command -v aws >/dev/null 2>&1 && fetch_remote_sync_state_to "$experiment_name" "$remote_tmp"; then
      remote="yes"
    fi
    rm -f "$remote_tmp"

    IFS='|' read -r pushed_at commit latest_run <<<"$summary"
    printf "%-24s  %-5s  %-6s  %-24s  %-12s  %s\n" \
      "$experiment_name" "$dirty" "$remote" "$pushed_at" "$commit" "$latest_run"
  done < <(list_local_experiments)
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
  ls)
    if [[ $# -gt 0 ]]; then
      echo "unexpected arguments for ls: $*" >&2
      usage
      exit 1
    fi
    if ! command -v aws >/dev/null 2>&1; then
      echo "aws CLI not found in PATH" >&2
      exit 1
    fi
    list_remote_experiments
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
