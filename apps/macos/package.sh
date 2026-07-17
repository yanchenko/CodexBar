#!/usr/bin/env bash
#
# package.sh — best-effort AgentBar.app + zip (PR12b).
# Unsigned by default; set SIGN_IDENTITY / NOTARIZE=1 when keys are available.
#
#   ./apps/macos/package.sh
#   OUTDIR=~/Desktop ./apps/macos/package.sh

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUTDIR="${OUTDIR:-$REPO/dist}"
VERSION="${VERSION:-0.1.0}"
ARCH="$(uname -m 2>/dev/null || echo unknown)"

echo "==> AgentBar macOS package $VERSION ($ARCH) → $OUTDIR"
mkdir -p "$OUTDIR"

# Build first.
"$HERE/build.sh"

BIN_DIR="$(cd "$HERE" && swift build -c release --show-bin-path)"
APP_BIN="$BIN_DIR/AgentBar"
[[ -x "$APP_BIN" ]] || { echo "missing $APP_BIN" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT INT TERM HUP
APP="$STAGE/AgentBar.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$HERE/Bundle/Info.plist" "$APP/Contents/Info.plist"
cp "$APP_BIN" "$APP/Contents/MacOS/AgentBar"
chmod +x "$APP/Contents/MacOS/AgentBar"

# Best-effort ad-hoc sign (optional).
if command -v codesign >/dev/null 2>&1; then
    if [[ -n "${SIGN_IDENTITY:-}" ]]; then
        codesign --force --deep --options runtime --sign "$SIGN_IDENTITY" "$APP" || true
    else
        codesign --force --deep --sign - "$APP" 2>/dev/null || true
    fi
fi

ZIP="$OUTDIR/agentbar-$VERSION-macos-$ARCH.zip"
(
    cd "$STAGE"
    if command -v ditto >/dev/null 2>&1; then
        ditto -c -k --keepParent AgentBar.app "$ZIP"
    else
        zip -qry "$ZIP" AgentBar.app
    fi
)
echo "    → $ZIP"
ls -lh "$ZIP"
echo "==> Done (unsigned/best-effort unless SIGN_IDENTITY set)."
