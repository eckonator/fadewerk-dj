#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Legt das Signatur-Zertifikat von FadeWerk DJ in einen eigenen Schlüsselbund und gibt die Zeilen aus,
# die scripts/bundle-macos.sh braucht. Das Zertifikat ist selbst signiert: macOS fragt beim ersten Start
# trotzdem einmal nach, erkennt aber jede spätere Version als dieselbe App, und das Update per Knopf
# nimmt nur an, was mit demselben Zertifikat signiert ist.
#
# Aufruf: eval "$(scripts/signing-keychain.sh ZERTIFIKAT.p12 SCHLUESSELBUND)"   (Passwort in FW_SIGN_PASSWORD)
set -euo pipefail

P12="$1"
KEYCHAIN="$2"
PASSWORD="${FW_SIGN_PASSWORD:?FW_SIGN_PASSWORD ist nicht gesetzt}"
# Nur dieses Skript nutzt den Schlüsselbund, sein eigenes Passwort muss nicht geheim sein
LOCK=fadewerk-dj

rm -f "$KEYCHAIN"
security create-keychain -p "$LOCK" "$KEYCHAIN"
security set-keychain-settings "$KEYCHAIN"
security unlock-keychain -p "$LOCK" "$KEYCHAIN"
security import "$P12" -k "$KEYCHAIN" -P "$PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$LOCK" "$KEYCHAIN" >/dev/null
# codesign sucht den privaten Schlüssel in der Suchliste; auf dem CI-Rechner den Schlüsselbund dort
# eintragen (auf einem Entwickler-Mac bleibt die Liste, wie sie ist)
if [ -n "${CI:-}" ]; then
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')
fi
IDENTITY=$(security find-identity -p codesigning "$KEYCHAIN" | awk '/^ *1\)/ {print $2; exit}')
if [ -z "$IDENTITY" ]; then
  echo "keine Signatur-Identität in $P12" >&2
  exit 1
fi
echo "export FW_SIGN_IDENTITY=$IDENTITY"
echo "export FW_SIGN_KEYCHAIN=$KEYCHAIN"
