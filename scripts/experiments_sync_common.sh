#!/usr/bin/env bash

if [[ -z "${ROOT_DIR:-}" ]]; then
  ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi

EXPERIMENTS_DIR="$ROOT_DIR/experiments"
EXPERIMENTS_S3_URI="s3://antfarm/experiments/"

ensure_experiments_dir() {
  mkdir -p "$EXPERIMENTS_DIR"
}

experiment_dir_by_name() {
  local experiment_name="$1"
  echo "$EXPERIMENTS_DIR/$experiment_name"
}

experiment_unpushed_marker_path() {
  local experiment_name="$1"
  echo "$(experiment_dir_by_name "$experiment_name")/.unpushed_data"
}

experiment_sync_state_path() {
  local experiment_name="$1"
  echo "$(experiment_dir_by_name "$experiment_name")/.sync_state.json"
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

experiment_name_from_server_config() {
  local server_config_path="$1"
  if ! is_experiment_server_config "$server_config_path"; then
    return 1
  fi

  local config_dir
  config_dir="$(cd "$(dirname "$server_config_path")" && pwd)"
  basename "$config_dir"
}

mark_experiment_dirty() {
  local experiment_name="$1"
  local experiment_dir
  experiment_dir="$(experiment_dir_by_name "$experiment_name")"
  mkdir -p "$experiment_dir"
  : > "$(experiment_unpushed_marker_path "$experiment_name")"
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
  local experiment_name="$1"
  local output_path="$2"
  aws \
    --cli-connect-timeout 3 \
    --cli-read-timeout 3 \
    s3 cp "${EXPERIMENTS_S3_URI}${experiment_name}/.sync_state.json" "$output_path" >/dev/null 2>&1
}

list_local_experiments() {
  ensure_experiments_dir
  find "$EXPERIMENTS_DIR" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; | LC_ALL=C sort
}

list_remote_experiments() {
  aws \
    --cli-connect-timeout 3 \
    --cli-read-timeout 3 \
    s3 ls "$EXPERIMENTS_S3_URI" 2>/dev/null | awk '/PRE / {print $2}' | sed 's:/$::' | LC_ALL=C sort
}

latest_run_dir_for_experiment() {
  local experiment_name="$1"
  local experiment_dir
  experiment_dir="$(experiment_dir_by_name "$experiment_name")"
  if [[ ! -d "$experiment_dir" ]]; then
    return 0
  fi
  find "$experiment_dir" -type d -name 'run-*' -print 2>/dev/null | LC_ALL=C sort | tail -n 1
}

write_experiment_sync_state() {
  local experiment_name="$1"
  local experiment_dir sync_state_path
  experiment_dir="$(experiment_dir_by_name "$experiment_name")"
  sync_state_path="$(experiment_sync_state_path "$experiment_name")"
  mkdir -p "$experiment_dir"

  local branch commit pushed_at latest_run run_relative condition_name run_id run_dir_json
  branch="$(git_branch_name)"
  commit="$(git_commit_id)"
  pushed_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  latest_run="$(latest_run_dir_for_experiment "$experiment_name")"
  run_dir_json="null"

  if [[ -n "$latest_run" ]]; then
    run_relative="${latest_run#"$experiment_dir"/}"
    local path_part_1 path_part_2 path_part_3
    IFS='/' read -r path_part_1 path_part_2 path_part_3 <<<"$run_relative"
    if [[ "$path_part_1" == "runs" ]]; then
      condition_name=""
      run_id="$path_part_2"
    else
      condition_name="$path_part_1"
      run_id="$path_part_3"
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

    run_dir_json="$(python3 - "$experiment_name" "$condition_name" "$run_id" "experiments/${experiment_name}/$run_relative" "$server_yaml_sha256" "$randomized_seed" <<'PY'
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

  python3 - "$sync_state_path" "$pushed_at" "$branch" "$commit" "$run_dir_json" <<'PY'
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
