# FadeWerk DJ

**Two decks, two playlists, auto-fade.** A DJ player for macOS, Windows and Linux,
written in Rust. Click a folder and start playing: no database, no library import.

![FadeWerk DJ with the “Night” skin](docs/screenshot.png)

<details>
<summary>“Classic” skin with brushed steel, the crossfade curve, a day of the history and a track playing in the headphones</summary>

![FadeWerk DJ with the “Classic” skin and brushed steel](docs/screenshot-steel.png)
</details>

*The screenshots show self-generated demo tracks (`scripts/make-demo-tracks.py`).*

## Features

- **Two decks, each with its own playlist**, and **auto-fade**: when the playing track reaches
  the fade time, the other deck starts and the crossfader blends over. The finished deck then
  loads its next track. Fade curve smooth, linear, sharp or drawn by hand. Optional **bass swap**:
  the incoming track starts without bass and the basses swap halfway through the fade.
  Starting a deck while the other one is stopped glides the crossfader over to it.
- **No import**: click a folder and all tracks, including subfolders, are there instantly.
  BPM detected by the tempo analysis is remembered; “Analyse BPM” (tracks) and “Detect missing BPM”
  (folders) run it in the background.
  Quick search across title, artist, album and path; the advanced search (magnifier, ⇧⌘F) filters
  by title, artist, album, file name, folder, genre, year range and BPM range, shown as removable filter chips; duplicate copies of a track are hidden while searching (can be turned off). Tags, BPM and waveform are analysed in the background.
  Select several tracks or folders (⌘/Ctrl-click, Shift-click) and drag them into a playlist, or use
  the right-click menu: append, play next, shuffle in, spread across A and B, show in Finder.
  Choose the track list columns (artist, album, genre, year, comment, subfolder, file name, format,
  BPM, time) and drag their widths; click a column header to sort ascending or descending. All of
  this is remembered.
- **History**: below the folders, “History” lists every day with the tracks that were played, in order,
  with time and deck. A track counts once it has been audible on master for 30 seconds; tracks until
  6 am belong to the day before. Give a day a name (“Grandma’s birthday”), remove entries, export it as
  M3U8 playlist, CSV table or FadeWerk DJ history file, and import such files or any M3U playlist.
  “Restore” fills playlists A and B again the way the day was played.
  Recording can be switched off in the settings.
- **A turntable per deck**: drag the platter or the waveform to scratch forwards and backwards
  (or use the mouse wheel / trackpad over the platter); let go with momentum and the record coasts on.
  Hold it to stop the record; click the platter to play or pause, right-click to stop; tap the waveform to play or pause.
  Eject slides the record off the deck (A to the left, B to the right); a newly loaded one slides in. Embedded cover art turns the record
  into a spinning picture disc. A tonearm swings onto the record on play, follows the track from the
  outer groove inwards, lifts on pause and returns to its rest on stop; drag it to drop the needle
  anywhere in the track. Prefer a compact deck? Hide the turntables under Settings → Appearance.
- **Motor**: start, pause and stop spin up and down like a real turntable
  (start and stop time adjustable in the Motor tab). CUE jumps instantly.
- **Transport per deck**: CUE, play/pause, stop, previous, next, repeat (off, track, list;
  track repeat fades the end into the start with the fade time and curve), eject, pitch ±8 %, sync, reverse (the record spins down and starts up the other way), loops of 4, 8 or 16 beats.
- **Pre-listening** per deck, in the playlists and right in the folder browser. Headphones via
  split output (left master, right headphones), on outputs 3/4 of a multichannel interface,
  or on a separate second audio device.
- **Mixer** with LED meters (A, main, B; on wider windows the main meter adds a spectrum of up to
  ten octave bands between left and right), channel faders, master and headphone mix, and a
  **3-band EQ per deck** (isolator, −26 to +6 dB) with kill buttons. Below the crossfader, tabs for
  pads, headphones, curve (crossfade curve and fade time) and motor keep the centre column tidy; one is open at a time.
  Right-click or double-click resets any fader to its default; with the mouse over a fader or knob,
  the arrow keys or the mouse wheel move it step by step. Click the fade-time display to type in a value.
- **Effect pads**: eight DJ effects (air horn, siren, laser, boom, riser, drop, rewind, brake), synthesised by
  FadeWerk DJ itself. Trigger them with a click or the keys 1–8, right-click to pre-listen in the headphones;
  a separate FX fader sets their volume. Replace any of them with your own audio file, name and one of
  16 drawn symbols under Settings → Effects.
- **Playlists**: fill and reorder by drag & drop, also between A and B (or via the right-click menu);
  drop tracks straight onto a deck. The loaded track always moves to the top of its playlist, the
  played ones stay above it. Playlists and settings are kept when you quit.
  After clicking into a playlist or the folder track list, ↑/↓ move through the tracks; Enter loads
  the marked track into the deck (playlists) or pre-listens to it (folder browser).
  Each playlist has a quick search that jumps to the matching track (Enter: next match).
- **Skins** Night, Classic, Day and Neon, optionally with a brushed steel look.
  The layout adapts to the window size, including 4K monitors. **Full screen** via the button
  in the folder browser, F11 or ⌃⌘F on macOS. On wide monitors, the button next to it moves the
  folder browser from below the decks to their right (folder tree above the track list).
- **Light on resources**: when nothing is playing, the interface only redraws on input. A
  **lightweight display** for older or slower computers drops shadows, gradients, glow and the
  spectrum and limits drawing to 30 frames per second.
- **Keeps playing in the background**: auto-fade and the next track also work while the window is
  minimised or covered by another app. When one playlist runs out, the other deck plays on alone.
- **No sleep while playing**: display and computer stay awake as long as a deck or pre-listen is
  playing (can be switched off).
- User interface in **English and German**.
- **No telemetry, no account.** The only network request is an optional check for new versions on
  GitHub. FadeWerk DJ asks once before enabling it (switch it on or off under *Settings → General*);
  it sends nothing about your music or folders.

## Download

Ready-to-run builds are on the [Releases](../../releases) page, one release per version.

| System | File | Notes |
|---|---|---|
| macOS 11 or later (Apple Silicon and Intel) | `…-macOS.dmg` or `…-macOS.zip` | Easiest: the [one-line Terminal install](#macos-cannot-be-verified) below. Or open the DMG and drag FadeWerk DJ to Applications (or unzip and move it there). The app is not notarized by Apple, so see below for the first launch. |
| Windows 10/11 (64-bit) | `…-Windows.zip` | Unzip and run `FadeWerk DJ.exe`. SmartScreen warns about unknown apps: **More info → Run anyway**. |
| Linux (x86_64) | `…-Linux-x86_64.tar.gz` | Unpack and run `./fadewerk-dj`. Needs ALSA (or PipeWire/PulseAudio with the ALSA layer). |
| Linux (arm64) | `…-Linux-arm64.tar.gz` | The same for 64-bit ARM, e.g. Raspberry Pi 4 or 5 with 64-bit Raspberry Pi OS (not yet tested on a real Pi). On a Pi 4, the lightweight display (Settings → Appearance) is recommended. |

### macOS: "cannot be verified"

The app is signed ad hoc but not notarized (that needs a paid Apple Developer account).
Files downloaded in the browser get a quarantine flag, and Gatekeeper then refuses to
open the app.

**Easiest: install from Terminal.** This downloads the latest release, copies it to Applications
and starts it. Downloads made this way carry no quarantine flag, so there is no security prompt.
Running the same command again updates to the newest version.

```sh
curl -fsSL https://raw.githubusercontent.com/eckonator/fadewerk-dj/main/scripts/install-macos.sh | bash
```

**Installed from the DMG or ZIP?** On recent macOS versions (Sequoia and later) *System Settings →
Privacy & Security → Open Anyway* often does not help. Remove the flag in Terminal instead:

```sh
xattr -dr com.apple.quarantine "/Applications/FadeWerk DJ.app"
```

Then open the app as usual. You need to do this once per downloaded version. The app is
open source; you can also build it yourself (see below), which needs no workaround.

## Building

You need a current Rust toolchain ([rustup.rs](https://rustup.rs)).
On Linux you also need the ALSA and X11/Wayland development packages, e.g. on Debian/Ubuntu:

```sh
sudo apt install libasound2-dev libxkbcommon-dev libwayland-dev libx11-dev libdbus-1-dev pkg-config
```

```sh
cargo run --release                     # starts with your music folder
cargo run --release -- ~/Music/Sets     # starts with a specific folder
cargo test --workspace                  # tests for engine, library and playlist logic
scripts/bundle-macos.sh                 # builds "dist/FadeWerk DJ.app" (macOS)
scripts/make-dmg.sh                     # packs it into a DMG with install window (needs `pip install dmgbuild`)
scripts/make-icon.sh                    # regenerates the icons from assets/icon.svg
scripts/make-demo-tracks.py ~/Demo      # generates royalty-free demo tracks
```

GitHub Actions builds all three systems on every push to `main` (`.github/workflows/release.yml`)
and publishes them as release `v<version>`, with the version taken from `Cargo.toml`.
A push without a version change replaces that release; raise `version` for a new one.

## Architecture

| Part | Purpose |
|---|---|
| `crates/engine` | Real-time audio: decks with motor, scratch, cue, loop and reverse, mixer, crossfader, EQ, headphone path, sampler for the effect pads, spectrum bands for the main meter, auto-fade logic. No GUI or system dependencies, tested. |
| `crates/library` | Live folder reading, tags (lofty), decoding (Symphonia), waveform, BPM detection, synthesis of the built-in effects, background threads. |
| `app` | User interface with egui, audio output with cpal, settings, skins; ties everything together. |

The engine runs in the audio thread and receives commands through a lock-free ring buffer.
It never locks, allocates or frees memory there: ejected tracks are handed back to the UI
thread to be dropped.

Code comments are in German; issues and contributions in English are welcome.

### Developer switches (debug builds only)

| Variable | Effect |
|---|---|
| `FW_DEMO=1` | Spreads the tracks of the start folder across both playlists and starts deck A. |
| `FW_SCREENSHOT=/path/image.bmp` | Saves a screenshot of the window shortly after start, then quits. |
| `FW_SIZE=1600x1000` | Window size. |
| `FW_SKIN=nacht\|classic\|tag\|neon`, `FW_STEEL=1`, `FW_LITE=1`, `FW_LANG=de\|en` | Look (`FW_LITE` = lightweight display) and language. |
| `FW_PREVIEW=1` | Pre-listens to the second track in the demo. |
| `FW_SELECT=1` | Selects tracks 2 to 4 in the folder browser. |
| `FW_HISTORY=1` | Creates two example days in the history (one with a missing file) and opens the newer one. |
| `FW_TAGS=1` | Opens “Track info & tags” for the first track. |
| `FW_BPM=1` | Detects the missing BPM of the start folder. |
| `FW_SEARCH="genre=rock,year=1980-1989"` | Opens the advanced search with these fields. |
| `FW_COLUMNS=artist,album,genre,…` | Shows these columns in the track list. |
| `FW_SORT=artist` / `FW_SORT=bpm:desc` | Sorts the track list by this column. |
| `FW_POINTER=x,y` | Moves the mouse there (points); `x,y,1` clicks, `x,y,2` right-clicks, `x1,y1,x2,y2` drags. |
| `FW_SETTINGS=allgemein\|audio\|effekte\|sprache\|aussehen\|ueber` | Opens the settings window. |
| `FW_TAB=pads\|kopfhoerer\|fade\|motor\|zu` | Opens this tab below the crossfader (`zu` = all closed). |
| `FW_UPDATE_ASK=1` | Shows the question whether to check for updates. |
| `FW_UPDATE_URL=file:///path/release.json` | Checks for updates against your own release JSON (`tag_name`, `html_url`). |

With these switches FadeWerk DJ saves nothing (`FW_PERSIST=1` forces saving, best with a separate `HOME`).

## Roadmap

- Cache for the waveform, so it shows up right away
- Beat grid with phase, hot cues, key detection, keylock
- MIDI controllers (jog wheels as turntables)
- Recording
- Importing playlists from other DJ software

## Legal

FadeWerk DJ is an independent development; code, graphics, icon, effect sounds and demo tracks are original work.

Bundled third-party components:

- Fonts [Chakra Petch](https://github.com/cadsondemak/Chakra-Petch) and
  [DSEG](https://github.com/keshikan/DSEG) under the SIL Open Font License 1.1
  (`assets/fonts/OFL-*.txt`)
- Rust libraries mostly under MIT/Apache-2.0; the audio decoders of
  [Symphonia](https://github.com/pdeljanov/Symphonia) under MPL-2.0. The full list ships with
  every release as `THIRD-PARTY-LICENSES.yml`.

## License

Copyright © 2026 Markus Eckert

FadeWerk DJ is free software: you can redistribute it and/or modify it under the terms of the
[GNU General Public License](LICENSE) as published by the Free Software Foundation, either
version 3 of the License, or (at your option) any later version.

FadeWerk DJ is distributed in the hope that it will be useful, but **without any warranty**;
without even the implied warranty of merchantability or fitness for a particular purpose.
See the [LICENSE](LICENSE) file for details.
