#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CALLER_DIR="$(pwd)"
cd "$ROOT_DIR"

args=()
server_config_specified=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --server-config)
      [[ $# -ge 2 ]] || { echo "--server-config requires a value" >&2; exit 1; }
      server_config_specified=1
      server_config="$2"
      if [[ -d "$server_config" ]]; then
        server_config="${server_config%/}/server.yaml"
      fi
      args+=("$1" "$server_config")
      shift 2
      ;;
    *)
      args+=("$1")
      shift
      ;;
  esac
done

if [[ $server_config_specified -eq 0 ]]; then
  if [[ ${#args[@]} -gt 0 ]]; then
    args=(--server-config "$CALLER_DIR/server.yaml" "${args[@]}")
  else
    args=(--server-config "$CALLER_DIR/server.yaml")
  fi
fi

exec cargo run -p antfarm-server -- "${args[@]}"
