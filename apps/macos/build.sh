#!/usr/bin/env bash
#
# build.sh — build the Rust FFI staticlib, then the SwiftUI macOS AgentBar host.
#
# Steps:
#   1. cargo build ab-core staticlib in release-ffi (panic=unwind, no strip)
#   2. sync agentbar.h into Sources/CAgentBar/include
#   3. swift build -c release (force_load libab_core.a)
#
# Build-only — does not run the app. Packaging is package.sh.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "$HERE/../../rust" && pwd)"

echo "==> [1/3] Building Rust FFI staticlib (release-ffi -p ab-core)…"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
( cd "$RUST_DIR" && cargo build --profile release-ffi -p ab-core )

STATICLIB="$RUST_DIR/target/release-ffi/libab_core.a"
if [[ ! -f "$STATICLIB" ]]; then
    echo "ERROR: expected staticlib not found: $STATICLIB" >&2
    exit 1
fi
echo "    staticlib: $STATICLIB"

echo "==> [2/3] Syncing agentbar.h…"
mkdir -p "$HERE/Sources/CAgentBar/include"
cp -f "$RUST_DIR/crates/ab-core/include/agentbar.h" "$HERE/Sources/CAgentBar/include/agentbar.h"

echo "==> [3/3] Swift build (release)…"
# Drop previous linked binary so Swift always relinks against fresh libab_core.a.
rm -f "$HERE/.build/release/AgentBar"
( cd "$HERE" && swift build -c release )

APP_BIN="$HERE/.build/release/AgentBar"
if [[ ! -x "$APP_BIN" ]]; then
    # Show-bin-path may differ by toolchain.
    APP_BIN="$(cd "$HERE" && swift build -c release --show-bin-path)/AgentBar"
fi
if [[ ! -x "$APP_BIN" ]]; then
    echo "ERROR: AgentBar executable not found after swift build" >&2
    exit 1
fi
if ! nm -gU "$APP_BIN" 2>/dev/null | grep -q 'ab_engine_start'; then
    # Soft check — some toolchains strip local symbols differently.
    echo "    note: could not nm-verify ab_engine_start (continuing)"
fi

echo
echo "==> Build complete."
echo "    Rust staticlib : $STATICLIB"
echo "    App executable : $APP_BIN"
