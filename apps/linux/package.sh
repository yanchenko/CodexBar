#!/usr/bin/env bash
# package.sh — build AgentBar Linux distributable tarball (PR13b).
#
#   apps/linux/package.sh
#   OUTDIR=~/Desktop apps/linux/package.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
GTK_DIR="$HERE/gtk"
OUTDIR="${OUTDIR:-$REPO/dist}"
VERSION="${VERSION:-0.1.0}"
ARCH="$(uname -m)"
mkdir -p "$OUTDIR"
echo "==> AgentBar $VERSION ($ARCH) → $OUTDIR"

echo "==> [1/2] cargo build --release (agentbar CLI + agentbar-gtk)"
( cd "$REPO/rust" && cargo build --release -p ab-cli )
( cd "$GTK_DIR" && cargo build --release )

RREL="$REPO/rust/target/release"
GREL="$GTK_DIR/target/release"
for b in "$GREL/agentbar-gtk" "$RREL/agentbar"; do
  [ -x "$b" ] || { echo "MISSING build output: $b" >&2; exit 1; }
done

echo "==> [2/2] portable tarball"
PKG="agentbar-$VERSION-linux-$ARCH"
STAGE="$(mktemp -d)"; trap 'rm -rf "$STAGE"' EXIT INT TERM HUP
ROOT="$STAGE/$PKG"
install -d "$ROOT/bin" "$ROOT/share/applications" "$ROOT/share/icons/hicolor/scalable/apps"
install -m0755 "$GREL/agentbar-gtk" "$RREL/agentbar" "$ROOT/bin/"
install -m0644 "$HERE/agentbar.desktop" "$ROOT/share/applications/agentbar.desktop"
if [[ -f "$REPO/docs/icon.png" ]]; then
  install -m0644 "$REPO/docs/icon.png" "$ROOT/share/icons/hicolor/scalable/apps/agentbar.png" 2>/dev/null || true
fi
install -m0644 "$REPO/LICENSE" "$ROOT/LICENSE"
install -m0755 "$HERE/tarball-install.sh" "$ROOT/install.sh"
printf 'AgentBar %s (%s) portable bundle.\nRun ./install.sh to install into ~/.local/bin.\n' "$VERSION" "$ARCH" > "$ROOT/README.txt"

TARBALL="$OUTDIR/$PKG.tar.gz"
tar -C "$STAGE" -czf "$TARBALL" "$PKG"
echo "    → $TARBALL"
ls -lh "$TARBALL"
echo "==> Done."
