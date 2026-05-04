#!/usr/bin/env bash

if [[ -z "${ROOT_DIR:-}" ]]; then
  ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi

EXPERIMENTS_DIR="$ROOT_DIR/experiments"
EXPERIMENTS_S3_URI="s3://antfarm/experiments/"
EXPERIMENTS_UNPUSHED_MARKER="$EXPERIMENTS_DIR/.unpushed_data"
EXPERIMENTS_SYNC_STATE_PATH="$EXPERIMENTS_DIR/.sync_state.json"

ensure_experiments_dir() {
  mkdir -p "$EXPERIMENTS_DIR"
}

mark_experiments_dirty() {
  ensure_experiments_dir
  : > "$EXPERIMENTS_UNPUSHED_MARKER"
}

is_experiment_server_config() {
  local server_config_path="$1"
  if [[ ! -f "$server_config_path" ]]; then
    return 1
  fi

  local config_dir
  config_dir="$(cd "$(dirname "$server_config_path")" && pwd)"
  [[ "$(basename "$(dirname "$config_dir")")" == "experiments" ]]
}

git_branch_name() {
  git -C "$ROOT_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown"
}

git_commit_id() {
  git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo "unknown"
}

git_worktree_is_dirty() {
  [[ -n "$(git -C "$ROOT_DIR" status --porcelain 2>/dev/null)" ]]
}

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
    return
  fi
  python3 - "$path" <<'PY'
import hashlib
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
print(hashlib.sha256(path.read_bytes()).hexdigest())
PY
}

fetch_remote_sync_state_to() {
  local output_path="$1"
  aws \
    --cli-connect-timeout 3 \
    --cli-read-timeout 3 \
    s3 cp "${EXPERIMENTS_S3_URI}.sync_state.json" "$output_path" >/dev/null 2>&1
}

latest_run_dir() {
  ensure_experiments_dir
  find "$EXPERIMENTS_DIR" -type d -name 'run-*' -print 2>/dev/null | LC_ALL=C sort | tail -n 1
}

write_experiments_sync_state() {
  ensure_experiments_dir

  local branch commit pushed_at latest_run run_relative experiment_name condition_name run_id run_dir_json
  branch="$(git_branch_name)"
  commit="$(git_commit_id)"
  pushed_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  latest_run="$(latest_run_dir)"
  run_dir_json="null"

  if [[ -n "$latest_run" ]]; then
    run_relative="${latest_run#"$EXPERIMENTS_DIR"/}"
    local path_part_1 path_part_2 path_part_3 path_part_4
    IFS='/' read -r path_part_1 path_part_2 path_part_3 path_part_4 <<<"$run_relative"
    experiment_name="$path_part_1"
    if [[ "$path_part_2" == "runs" ]]; then
      condition_name=""
      run_id="$path_part_3"
    else
      condition_name="$path_part_2"
      run_id="$path_part_4"
    fi

    local run_server_yaml randomized_seed server_yaml_sha256
    run_server_yaml="$latest_run/server.yaml"
    server_yaml_sha256=""
    if [[ -f "$run_server_yaml" ]]; then
      server_yaml_sha256="$(sha256_file "$run_server_yaml")"
    fi

    randomized_seed="$(python3 - "$latest_run/manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
if not path.exists():
    print("")
    raise SystemExit(0)

manifest = json.loads(path.read_text())
seed = manifest.get("randomized_seed")
print("" if seed is None else seed)
PY
)"

    run_dir_json="$(python3 - "$experiment_name" "$condition_name" "$run_id" "experiments/$run_relative" "$server_yaml_sha256" "$randomized_seed" <<'PY'
import json
import sys

experiment, condition, run_id, run_dir, server_yaml_sha256, randomized_seed = sys.argv[1:7]
payload = {
    "experiment": experiment,
    "condition": condition or None,
    "run_id": run_id,
    "run_dir": run_dir,
    "server_yaml_sha256": server_yaml_sha256 or None,
    "randomized_seed": int(randomized_seed) if randomized_seed else None,
}
print(json.dumps(payload))
PY
)"
  fi

  python3 - "$EXPERIMENTS_SYNC_STATE_PATH" "$pushed_at" "$branch" "$commit" "$run_dir_json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
pushed_at = sys.argv[2]
branch = sys.argv[3]
commit = sys.argv[4]
latest_run = json.loads(sys.argv[5])

payload = {
    "pushed_at": pushed_at,
    "branch": branch,
    "commit": commit,
    "latest_run": latest_run,
}
path.write_text(json.dumps(payload, indent=2) + "\n")
PY
}
