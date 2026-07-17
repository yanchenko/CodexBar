#!/usr/bin/env bash
# tarball-install.sh — ships inside the Linux portable tarball as ./install.sh.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${AGENTBAR_INSTALL_DIR:-$HOME/.local/bin}"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"

pkill -x agentbar-gtk 2>/dev/null || true
install -d "$BIN" "$APPS" "$ICONS"
install -m0755 "$HERE"/bin/* "$BIN/"

ESC_BIN="$(printf '%s' "$BIN" | sed -e 's/[\\&|]/\\&/g')"
if [[ -f "$HERE/share/applications/agentbar.desktop" ]]; then
  sed "s|^Exec=agentbar-gtk|Exec=\"$ESC_BIN/agentbar-gtk\"|" \
    "$HERE/share/applications/agentbar.desktop" > "$APPS/agentbar.desktop"
fi
if [[ -f "$HERE/share/icons/hicolor/scalable/apps/agentbar.png" ]]; then
  install -m0644 "$HERE/share/icons/hicolor/scalable/apps/agentbar.png" "$ICONS/agentbar.png"
fi

if [[ "${AGENTBAR_NO_AUTOSTART:-0}" != "1" ]]; then
  AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
  install -d "$AUTOSTART"
  cp "$APPS/agentbar.desktop" "$AUTOSTART/agentbar.desktop" 2>/dev/null || true
fi

echo
echo "Installed to $BIN."
echo "Launch: $BIN/agentbar-gtk  (or the \"AgentBar\" app menu entry)"
echo "CLI:    $BIN/agentbar usage --format json"
if [[ "${AGENTBAR_NO_AUTOSTART:-0}" != "1" ]]; then
  echo "Start-at-login enabled (~/.config/autostart; AGENTBAR_NO_AUTOSTART=1 to skip)"
fi
