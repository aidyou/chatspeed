#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/src-tauri"
# Match the installed runtime's platform data directory (dirs::data_dir()).
platform=$(uname -s)
case "$platform" in
  Darwin)
    data_dir="$HOME/Library/Application Support"
    ;;
  Linux)
    case "${XDG_DATA_HOME:-}" in
      /*) data_dir="$XDG_DATA_HOME" ;;
      *) data_dir="$HOME/.local/share" ;;
    esac
    ;;
  MINGW*|MSYS*|CYGWIN*)
    if [[ -z "${APPDATA:-}" ]]; then
      printf 'APPDATA is not set; cannot locate the ChatSpeed runtime discovery file.\n' >&2
      exit 1
    fi
    # A mixed Windows path works for both Bash's readability check and native Cargo.
    data_dir=$(cygpath -m -- "$APPDATA")
    ;;
  *)
    printf 'Unsupported platform: %s\n' "$platform" >&2
    exit 1
    ;;
esac

discovery="$data_dir/ai.aidyou.chatspeed/runtime/control-plane-v1.json"
if [[ ! -r "$discovery" ]]; then
  printf 'Runtime discovery file is not readable: %s\nStart the installed ChatSpeed application first.\n' "$discovery" >&2
  exit 1
fi
mkdir -p ../dev_data/logs
CARGO_BUILD_JOBS=1 cargo run --release -p chatspeed-gpui -- \
  --discovery "$discovery" \
  --metrics ../dev_data/logs/chatspeed-gpui-metrics.jsonl
