#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Baut „FadeWerk DJ.app“ für macOS nach dist/.
# FW_SIGN_IDENTITY / FW_SIGN_KEYCHAIN: Zertifikat zum Signieren (Standard: ad-hoc), siehe scripts/signing-keychain.sh
# Aufruf: scripts/bundle-macos.sh   (FW_SKIP_BUILD=1 überspringt das Bauen)
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP="dist/FadeWerk DJ.app"

# FW_SKIP_BUILD=1: vorhandenes target/release/fadewerk-dj verwenden (z. B. Universal-Binary aus der CI)
if [ -z "${FW_SKIP_BUILD:-}" ]; then
  cargo build --release -p fadewerk-dj
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/fadewerk-dj "$APP/Contents/MacOS/FadeWerk DJ"
if [ -f "assets/FadeWerk DJ.icns" ]; then
  cp "assets/FadeWerk DJ.icns" "$APP/Contents/Resources/FadeWerk DJ.icns"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>FadeWerk DJ</string>
  <key>CFBundleDisplayName</key><string>FadeWerk DJ</string>
  <key>CFBundleIdentifier</key><string>de.eckonator.fadewerkdj</string>
  <key>CFBundleExecutable</key><string>FadeWerk DJ</string>
  <key>CFBundleIconFile</key><string>FadeWerk DJ</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>NSHumanReadableCopyright</key><string>© 2026 Markus Eckert · GPL-3.0-or-later</string>
  <key>CFBundleDevelopmentRegion</key><string>de</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Mit dem eigenen Zertifikat des Projekts signiert, wenn es eins gibt (scripts/signing-keychain.sh):
# Dann nimmt das Update per Knopf nur Versionen mit demselben Zertifikat an. Sonst ad-hoc, damit
# Apple-Silicon-Macs die App überhaupt starten.
SIGN=(--force --sign "${FW_SIGN_IDENTITY:--}" --identifier de.eckonator.fadewerkdj)
if [ -n "${FW_SIGN_KEYCHAIN:-}" ]; then
  SIGN+=(--keychain "$FW_SIGN_KEYCHAIN")
fi
codesign "${SIGN[@]}" "$APP" >/dev/null
codesign --verify --deep --strict "$APP"
echo "Fertig: $APP"
