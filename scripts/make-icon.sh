#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Erzeugt aus assets/icon.svg die Icon-Dateien:
#   assets/icon-1024.png, assets/icon-256.png (Fenster-Icon, Linux), „assets/FadeWerk DJ.icns“ (macOS-App)
#   und assets/icon.ico (in die Windows-.exe eingebettet, app/build.rs)
# Braucht Google Chrome zum Rendern des SVG.
set -euo pipefail

cd "$(dirname "$0")/../assets"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

"$CHROME" --headless=new --disable-gpu --hide-scrollbars --window-size=1024,1024 \
  --force-device-scale-factor=1 --default-background-color=00000000 \
  --screenshot="$PWD/icon-1024.png" "file://$PWD/icon.svg" 2>/dev/null

sips -z 256 256 icon-1024.png --out icon-256.png >/dev/null

SET=$(mktemp -d)/FadeWerk-DJ.iconset
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  sips -z $s $s icon-1024.png --out "$SET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) icon-1024.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "FadeWerk DJ.icns"

ICO=$(mktemp -d)
for s in 16 20 24 32 40 48 64 128 256; do
  sips -z $s $s icon-1024.png --out "$ICO/icon-$s.png" >/dev/null
done
python3 ../scripts/make-ico.py icon.ico "$ICO"/icon-*.png
echo "Fertig: assets/FadeWerk DJ.icns, assets/icon.ico"
