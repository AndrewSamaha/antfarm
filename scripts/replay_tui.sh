#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ $# -ne 1 ]]; then
  echo "Usage: ./antfarm replay-tui <replay-artifact-path>" >&2
  exit 1
fi

exec cargo run -p antfarm-tui -- --replay "$1"
