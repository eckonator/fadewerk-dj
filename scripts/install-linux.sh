#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Installiert FadeWerk DJ aus dem entpackten Linux-Archiv für den aktuellen Benutzer:
# Programm nach ~/.local/bin, Startmenü-Eintrag und Icon nach ~/.local/share.
# Danach steht FadeWerk DJ mit Icon im Anwendungsmenü und im Dock/der Taskleiste.
#   ./install.sh              installieren (erneut ausführen = aktualisieren)
#   ./install.sh --uninstall  wieder entfernen (Einstellungen bleiben erhalten)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" || true
}

if [ "${1:-}" = "--uninstall" ]; then
  rm -f "$BIN/fadewerk-dj" "$APPS/fadewerk-dj.desktop" "$ICONS/256x256/apps/fadewerk-dj.png"
  refresh
  echo "FadeWerk DJ entfernt."
  exit 0
fi

mkdir -p "$BIN" "$APPS" "$ICONS/256x256/apps"
install -m755 "$HERE/fadewerk-dj" "$BIN/fadewerk-dj"
install -m644 "$HERE/fadewerk-dj.png" "$ICONS/256x256/apps/fadewerk-dj.png"
# Exec mit vollem Pfad: ~/.local/bin steht nicht bei jeder Desktop-Umgebung im Suchpfad
sed "s|^Exec=.*|Exec=\"$BIN/fadewerk-dj\"|" "$HERE/fadewerk-dj.desktop" > "$APPS/fadewerk-dj.desktop"
chmod 644 "$APPS/fadewerk-dj.desktop"
refresh

echo "FadeWerk DJ installiert: $BIN/fadewerk-dj"
echo "Start über das Anwendungsmenü oder mit: fadewerk-dj"
