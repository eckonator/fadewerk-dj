#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Erzeugt rechtefreie Demo-Titel für Screenshots und Tests (nur Python-Standardbibliothek).

Jeder Titel ist ein einfacher, selbst synthetisierter Groove (Bassdrum, Hi-Hat, Bass, Akkordfläche)
in eigenem Tempo. Dateiname „FadeWerk DJ Demo - <Titel>.wav“, daraus liest FadeWerk DJ Interpret und Titel.

Aufruf: scripts/make-demo-tracks.py <Zielordner>
"""

import math
import random
import struct
import sys
import wave
from pathlib import Path

RATE = 44100
LENGTH_SECS = 200

# Titel, Tempo, Grundton (MIDI) und Akkordfolge (Halbtöne über dem Grundton)
TRACKS = [
    ("Steel Groove", 124, 45, [0, 5, 7, 3]),
    ("Night Shift", 128, 43, [0, 3, 8, 7]),
    ("Amber Lights", 122, 48, [0, 7, 5, 5]),
    ("Ice Blue", 126, 41, [0, 8, 3, 10]),
    ("Second Deck", 120, 46, [0, 5, 3, 7]),
    ("Turntable Dawn", 118, 44, [0, 10, 8, 7]),
    ("Brushed Metal", 130, 40, [0, 3, 5, 3]),
    ("Crossfade Theory", 125, 47, [0, 7, 8, 5]),
    ("Warm Up", 116, 42, [0, 5, 10, 8]),
    ("Last Call", 127, 45, [0, 3, 10, 5]),
]


def freq(midi: float) -> float:
    return 440.0 * 2 ** ((midi - 69) / 12)


def render_bar(bpm: int, root: int, step: int, rng: random.Random) -> list[float]:
    """Ein Takt (4 Schläge) als Liste von Abtastwerten."""
    beat = 60.0 / bpm
    n = int(4 * beat * RATE)
    out = [0.0] * n
    # Bassdrum auf jedem Schlag: abfallende Sinus-Tonhöhe
    for b in range(4):
        start = int(b * beat * RATE)
        for i in range(min(int(0.25 * RATE), n - start)):
            t = i / RATE
            f = 50 + 90 * math.exp(-t * 30)
            out[start + i] += 0.9 * math.sin(2 * math.pi * f * t) * math.exp(-t * 9)
    # Offene Hi-Hat auf den Offbeats: gefiltertes Rauschen
    for b in range(4):
        start = int((b + 0.5) * beat * RATE)
        prev = 0.0
        for i in range(min(int(0.12 * RATE), n - start)):
            noise = rng.uniform(-1, 1)
            hp = noise - prev
            prev = noise
            out[start + i] += 0.18 * hp * math.exp(-i / RATE * 28)
    # Bass auf den Achteln zwischen den Schlägen
    bass_f = freq(root + step)
    for b in range(4):
        start = int((b + 0.5) * beat * RATE)
        for i in range(min(int(0.4 * beat * RATE), n - start)):
            t = i / RATE
            out[start + i] += 0.35 * math.sin(2 * math.pi * bass_f * t) * min(1.0, t * 200) * math.exp(-t * 6)
    # Akkordfläche (Moll-Dreiklang eine Oktave höher), weich ein- und ausgeblendet
    chord = [freq(root + 12 + step + k) for k in (0, 3, 7)]
    for i in range(n):
        t = i / RATE
        env = min(1.0, t * 4) * min(1.0, (n - i) / RATE * 4)
        out[i] += 0.06 * env * sum(math.sin(2 * math.pi * f * t) for f in chord)
    return out


def write_track(path: Path, bpm: int, root: int, steps: list[int], seed: int) -> None:
    rng = random.Random(seed)
    bars = [render_bar(bpm, root, s, rng) for s in steps]
    total = LENGTH_SECS * RATE
    frames = bytearray()
    written = 0
    k = 0
    while written < total:
        bar = bars[k % len(bars)]
        # Ein- und Ausblenden über die ersten und letzten 8 Sekunden
        for v in bar:
            if written >= total:
                break
            fade = min(1.0, written / (8 * RATE), (total - written) / (8 * RATE))
            s = max(-1.0, min(1.0, v * 0.8 * fade))
            frames += struct.pack("<h", int(s * 32767))
            written += 1
        k += 1
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(bytes(frames))


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    target = Path(sys.argv[1])
    target.mkdir(parents=True, exist_ok=True)
    for i, (title, bpm, root, steps) in enumerate(TRACKS):
        path = target / f"FadeWerk DJ Demo - {title}.wav"
        write_track(path, bpm, root, steps, seed=i)
        print(f"{path.name}  ({bpm} BPM)")


if __name__ == "__main__":
    main()
