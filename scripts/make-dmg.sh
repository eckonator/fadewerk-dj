#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Packt „dist/FadeWerk DJ.app“ in ein DMG mit Installationsfenster (App nach „Programme“ ziehen).
# Aufruf: scripts/make-dmg.sh [Ziel.dmg]   (vorher scripts/bundle-macos.sh)
# Braucht dmgbuild (pip install dmgbuild); DMG_PYTHON wählt den Python-Interpreter dafür.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT="${1:-dist/FadeWerk-DJ-$VERSION-macOS.dmg}"
APP="dist/FadeWerk DJ.app"
[ -d "$APP" ] || { echo "Fehlt: $APP (erst scripts/bundle-macos.sh)" >&2; exit 1; }

rm -f "$OUT"
"${DMG_PYTHON:-python3}" -m dmgbuild -s scripts/dmg-settings.py \
  -D app="$PWD/$APP" -D root="$PWD" "FadeWerk DJ $VERSION" "$OUT"
echo "Fertig: $OUT"
