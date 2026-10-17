#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Packt PNG-Dateien zu einer Windows-ICO-Datei (PNG-Einträge, ab Windows Vista).

Aufruf: make-ico.py ziel.ico bild-16.png bild-32.png … (je Bild quadratisch, höchstens 256 px).
Nur Standardbibliothek, damit make-icon.sh keine Zusatzpakete braucht.
"""
import struct
import sys


def png_size(data: bytes) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise ValueError("keine PNG-Datei")
    return struct.unpack(">II", data[16:24])


def main() -> None:
    target, sources = sys.argv[1], sys.argv[2:]
    images = []
    for path in sources:
        with open(path, "rb") as f:
            data = f.read()
        w, h = png_size(data)
        if w > 256 or h > 256:
            raise SystemExit(f"{path}: {w}×{h} ist größer als 256 px")
        images.append((w, h, data))
    images.sort(key=lambda i: i[0])

    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for w, h, data in images:
        # 256 px wird als 0 eingetragen
        entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    with open(target, "wb") as f:
        f.write(header + entries + blobs)


if __name__ == "__main__":
    main()
