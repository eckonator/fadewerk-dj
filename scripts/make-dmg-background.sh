#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Erzeugt aus assets/dmg-background.svg den Fensterhintergrund des DMG:
#   assets/dmg-background.tiff (640×480 und 1280×960 für Retina in einer Datei)
# Nur nach Änderungen am SVG nötig. Braucht Google Chrome zum Rendern.
set -euo pipefail

cd "$(dirname "$0")/../assets"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
TMP=$(mktemp -d)

for scale in 1 2; do
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --window-size=640,480 \
    --force-device-scale-factor=$scale --screenshot="$TMP/bg-$scale.png" "file://$PWD/dmg-background.svg" 2>/dev/null
done
sips -s dpiWidth 144 -s dpiHeight 144 "$TMP/bg-2.png" >/dev/null
tiffutil -cathidpicheck "$TMP/bg-1.png" "$TMP/bg-2.png" -out dmg-background.tiff >/dev/null
echo "Fertig: assets/dmg-background.tiff"
