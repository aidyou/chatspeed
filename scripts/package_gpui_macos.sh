#!/usr/bin/env bash
# Build the standalone macOS GUI bundle with the shared ChatSpeed icon.
set -euo pipefail

if [[ $(uname -s) != Darwin ]]; then
  printf '%s\n' 'This packaging script requires macOS.' >&2
  exit 1
fi

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../src-tauri"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"
cargo build --locked --release -p chatspeed-gpui
app="$CARGO_TARGET_DIR/release/ChatSpeed GPUI.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$CARGO_TARGET_DIR/release/chatspeed-gpui" "$app/Contents/MacOS/"
cp icons/icon.icns "$app/Contents/Resources/"
cp crates/gpui/macos-Info.plist "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf '%s\n' "$app"
