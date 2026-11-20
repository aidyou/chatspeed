#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/src-tauri"
discovery="${XDG_DATA_HOME:-$HOME/.local/share}/ai.aidyou.chatspeed/runtime/control-plane-v1.json"
if [[ ! -r "$discovery" ]]; then
  printf 'Runtime discovery file is not readable: %s\nStart the installed ChatSpeed application first.\n' "$discovery" >&2
  exit 1
fi
mkdir -p ../dev_data/logs
CARGO_BUILD_JOBS=1 cargo run --release -p chatspeed-gpui -- \
  --discovery "$discovery" \
  --metrics ../dev_data/logs/chatspeed-gpui-metrics.jsonl
