// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::atomic::{AtomicF32, AtomicF64};
use crate::deck::Deck;
use crate::eq::Eq;
use crate::headlink::HeadSink;
use crate::mixer::{Curve, HeadMode};
use crate::motor::Motor;
use crate::track::TrackAudio;

/// Anzahl der Player.
pub const DECKS: usize = 2;
/// Index der Vorhör-Stimme für den Ordnerbrowser. Sie läuft nur auf dem Kopfhörer.
pub const PREVIEW: usize = 2;
/// Player plus Vorhör-Stimme.
pub const VOICES: usize = 3;

/// Größter Block, der am Stück gerechnet wird. Größere Blöcke werden aufgeteilt,
/// damit Parameter wie der Crossfader fein genug geglättet werden.
const CHUNK: usize = 256;

/// Befehle von der Oberfläche an den Audio-Thread.
pub enum Command {
    Load { deck: usize, id: u64, track: Arc<TrackAudio>, keep_playing: bool, start_secs: f64 },
    Eject { deck: usize },
    Play { deck: usize },
    Pause { deck: usize },
    TogglePlay { deck: usize },
    Stop { deck: usize },
    Cue { deck: usize },
    Seek { deck: usize, secs: f64 },
    SetPitch { deck: usize, pitch: f32 },
    SetRepeatTrack { deck: usize, on: bool },
    SetLoop { deck: usize, range: Option<(f64, f64)> },
    ScratchBegin { deck: usize },
    ScratchTo { deck: usize, secs: f64 },
    ScratchEnd { deck: usize },
    SetCrossfader(f32),
    SetCurve(Curve),
    SetChannelVolume { deck: usize, volume: f32 },
    SetPfl { deck: usize, on: bool },
    /// Pegel eines EQ-Bandes (0 = Bässe, 1 = Mitten, 2 = Höhen) als Faktor, 0 = Kill.
    SetEq { deck: usize, band: usize, gain: f32 },
    SetHeadMode(HeadMode),
    SetHeadMix(f32),
    SetHeadVolume(f32),
    SetMasterVolume(f32),
    SetMotor(Motor),
    /// Verbindung zum eigenen Kopfhörergerät setzen oder lösen.
    SetHeadSink(Option<HeadSink>),
}

/// Zustand eines Players, wie ihn die Oberfläche sieht.
#[derive(Default)]
pub struct DeckStatus {
    track_id: AtomicU64,
    position: AtomicF64,
    duration: AtomicF64,
    speed: AtomicF32,
    rate: AtomicF32,
    playing: AtomicBool,
    ended: AtomicU64,
}

impl DeckStatus {
    pub fn track_id(&self) -> u64 { self.track_id.load(Ordering::Relaxed) }
    pub fn position_secs(&self) -> f64 { self.position.load() }
    pub fn duration_secs(&self) -> f64 { self.duration.load() }
    /// Motordrehzahl ohne Pitch, 0.0 bis 1.0.
    pub fn speed(&self) -> f32 { self.speed.load() }
    /// Tatsächliche Lesegeschwindigkeit inklusive Pitch und Scratch.
    pub fn rate(&self) -> f32 { self.rate.load() }
    pub fn is_playing(&self) -> bool { self.playing.load(Ordering::Relaxed) }
    /// Zähler, der bei jedem Titelende hochzählt.
    pub fn ended_count(&self) -> u64 { self.ended.load(Ordering::Relaxed) }
}

/// Von Audio-Thread und Oberfläche gemeinsam genutzter Zustand.
pub struct Shared {
    pub sample_rate: u32,
    pub channels: usize,
    pub decks: [DeckStatus; VOICES],
    /// Spitzenpegel seit dem letzten Abholen: A, B, Master, jeweils links/rechts.
    levels: [[AtomicF32; 2]; 3],
    crossfader: AtomicF32,
}

impl Shared {
    /// Holt die Spitzenpegel seit dem letzten Aufruf ab: `[A, B, Master]`, je `[L, R]`.
    /// Die Kanäle A und B werden nach dem Kanalfader und vor dem Crossfader gemessen.
    pub fn take_levels(&self) -> [[f32; 2]; 3] {
        std::array::from_fn(|i| [self.levels[i][0].take(), self.levels[i][1].take()])
    }

    /// Aktuelle (geglättete) Crossfader-Stellung im Audio-Thread.
    pub fn crossfader(&self) -> f32 {
        self.crossfader.load()
    }
}

/// Griff der Oberfläche auf die Engine.
pub struct Controller {
    tx: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Arc<TrackAudio>>,
    pub shared: Arc<Shared>,
}

impl Controller {
    /// Schickt einen Befehl. Ist der Puffer voll (Audio-Thread hängt), wird er verworfen.
    pub fn send(&mut self, cmd: Command) {
        if self.tx.push(cmd).is_err() {
            log_dropped();
        }
    }

    /// Gibt ausgeworfene Titel frei. Regelmäßig aus der Oberfläche aufrufen.
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }
}

#[cold]
fn log_dropped() {
    eprintln!("fw-engine: Befehlspuffer voll, Befehl verworfen");
}

/// Läuft im Audio-Thread.
pub struct Engine {
    rx: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Arc<TrackAudio>>,
    shared: Arc<Shared>,
    decks: [Deck; VOICES],
    bufs: [Vec<[f32; 2]>; VOICES],
    motor: Motor,
    xf_target: f32,
    xf: f32,
    curve: Curve,
    volume: [f32; DECKS],
    pfl: [bool; DECKS],
    eq: [Eq; DECKS],
    head_mode: HeadMode,
    head_mix: f32,
    head_volume: f32,
    master_volume: f32,
    head_sink: Option<HeadSink>,
}

impl Engine {
    /// Legt Engine (für den Audio-Thread) und Controller (für die Oberfläche) an.
    pub fn new(sample_rate: u32, channels: usize) -> (Engine, Controller) {
        let (tx, rx) = rtrb::RingBuffer::new(1024);
        let (gtx, grx) = rtrb::RingBuffer::new(64);
        let shared = Arc::new(Shared {
            sample_rate,
            channels,
            decks: Default::default(),
            levels: Default::default(),
            crossfader: AtomicF32::new(-1.0),
        });
        let engine = Engine {
            rx,
            garbage: gtx,
            shared: shared.clone(),
            decks: Default::default(),
            bufs: std::array::from_fn(|_| vec![[0.0; 2]; CHUNK]),
            motor: Motor::default(),
            xf_target: -1.0,
            xf: -1.0,
            curve: Curve::Soft,
            volume: [0.85; DECKS],
            pfl: [false; DECKS],
            eq: std::array::from_fn(|_| Eq::new(sample_rate)),
            head_mode: HeadMode::Off,
            head_mix: 0.0,
            head_volume: 0.8,
            master_volume: 0.9,
            head_sink: None,
        };
        (engine, Controller { tx, garbage: grx, shared })
    }

    fn discard(&mut self, old: Option<Arc<TrackAudio>>) {
        if let Some(t) = old {
            // Freigeben übernimmt die Oberfläche. Ist der Rückweg voll, bleibt nur Freigeben hier.
            let _ = self.garbage.push(t);
        }
    }

    fn apply(&mut self, cmd: Command) {
        use Command::*;
        let ok = |d: usize| d < VOICES;
        match cmd {
            Load { deck, id, track, keep_playing, start_secs } if ok(deck) => {
                let old = self.decks[deck].load(track, id, keep_playing);
                if start_secs > 0.0 {
                    self.decks[deck].seek_secs(start_secs);
                }
                self.discard(old);
            }
            Eject { deck } if ok(deck) => {
                let old = self.decks[deck].eject();
                self.discard(old);
            }
            Play { deck } if ok(deck) => self.decks[deck].play(),
            Pause { deck } if ok(deck) => self.decks[deck].pause(),
            TogglePlay { deck } if ok(deck) => self.decks[deck].toggle(),
            Stop { deck } if ok(deck) => self.decks[deck].stop(),
            Cue { deck } if ok(deck) => self.decks[deck].cue(),
            Seek { deck, secs } if ok(deck) => self.decks[deck].seek_secs(secs),
            SetPitch { deck, pitch } if ok(deck) => self.decks[deck].set_pitch(pitch),
            SetRepeatTrack { deck, on } if ok(deck) => self.decks[deck].set_repeat_track(on),
            SetLoop { deck, range } if ok(deck) => self.decks[deck].set_loop_secs(range),
            ScratchBegin { deck } if ok(deck) => self.decks[deck].scratch_begin(),
            ScratchTo { deck, secs } if ok(deck) => self.decks[deck].scratch_to_secs(secs),
            ScratchEnd { deck } if ok(deck) => self.decks[deck].scratch_end(),
            SetCrossfader(x) => self.xf_target = x.clamp(-1.0, 1.0),
            SetCurve(c) => self.curve = c,
            SetChannelVolume { deck, volume } if deck < DECKS => self.volume[deck] = volume.clamp(0.0, 1.0),
            SetPfl { deck, on } if deck < DECKS => self.pfl[deck] = on,
            SetEq { deck, band, gain } if deck < DECKS => self.eq[deck].set_gain(band, gain),
            SetHeadMode(m) => self.head_mode = m,
            SetHeadMix(m) => self.head_mix = m.clamp(0.0, 1.0),
            SetHeadVolume(v) => self.head_volume = v.clamp(0.0, 1.0),
            SetMasterVolume(v) => self.master_volume = v.clamp(0.0, 1.0),
            SetMotor(m) => self.motor = m,
            SetHeadSink(s) => self.head_sink = s,
            _ => {}
        }
    }

    /// Füllt einen verschachtelten Ausgabepuffer mit `channels` Kanälen.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(cmd) = self.rx.pop() {
            self.apply(cmd);
        }
        if channels == 0 {
            return;
        }
        let rate = self.shared.sample_rate as f32;
        let mut peaks = [[0.0f32; 2]; 3];

        for block in out.chunks_mut(CHUNK * channels) {
            let n = block.len() / channels;
            for (v, (deck, buf)) in self.decks.iter_mut().zip(self.bufs.iter_mut()).enumerate() {
                // Im Kopfhörer (PFL oder Vorhören) spielt der Titel normal, ohne Plattenspieler-Rampen
                let headphones = v == PREVIEW || (v < DECKS && self.pfl[v]);
                let motor = if headphones { &Motor::INSTANT } else { &self.motor };
                deck.render(motor, rate, &mut buf[..n]);
            }
            // EQ vor Kanalfader und Vorhören, wie am Mischpult
            for (eq, buf) in self.eq.iter_mut().zip(self.bufs.iter_mut()) {
                eq.process(&mut buf[..n]);
            }

            // Crossfader gegen Knacksen glätten: höchstens ein voller Weg in 20 ms
            let max_step = 2.0 * n as f32 / (0.02 * rate);
            let xf0 = self.xf;
            let xf1 = xf0 + (self.xf_target - xf0).clamp(-max_step, max_step);
            self.xf = xf1;
            let (ga0, gb0) = self.curve.gains(xf0);
            let (ga1, gb1) = self.curve.gains(xf1);

            for i in 0..n {
                let t = i as f32 / n as f32;
                let ga = ga0 + (ga1 - ga0) * t;
                let gb = gb0 + (gb1 - gb0) * t;
                let a = self.bufs[0][i];
                let b = self.bufs[1][i];
                let p = self.bufs[PREVIEW][i];
                let ca = [a[0] * self.volume[0], a[1] * self.volume[0]];
                let cb = [b[0] * self.volume[1], b[1] * self.volume[1]];
                let mut master = [0.0f32; 2];
                let mut cue = [p[0], p[1]];
                for c in 0..2 {
                    master[c] = (ca[c] * ga + cb[c] * gb) * self.master_volume;
                    // Vorhören greift vor Kanalfader und Crossfader ab
                    if self.pfl[0] { cue[c] += a[c]; }
                    if self.pfl[1] { cue[c] += b[c]; }
                    peaks[0][c] = peaks[0][c].max(ca[c].abs());
                    peaks[1][c] = peaks[1][c].max(cb[c].abs());
                    peaks[2][c] = peaks[2][c].max(master[c].abs());
                }
                let head = [
                    (cue[0] * (1.0 - self.head_mix) + master[0] * self.head_mix) * self.head_volume,
                    (cue[1] * (1.0 - self.head_mix) + master[1] * self.head_mix) * self.head_volume,
                ];
                write_frame(&mut block[i * channels..(i + 1) * channels], self.head_mode, master, head);
                if self.head_mode == HeadMode::Device
                    && let Some(sink) = self.head_sink.as_mut()
                {
                    // Ist der Puffer voll (Kopfhörergerät hängt), geht dieser Frame verloren
                    let _ = sink.push([soft_clip(head[0]), soft_clip(head[1])]);
                }
            }
        }

        self.publish(peaks);
    }

    fn publish(&mut self, peaks: [[f32; 2]; 3]) {
        let s = &self.shared;
        for (deck, st) in self.decks.iter_mut().zip(s.decks.iter()) {
            st.track_id.store(deck.track_id(), Ordering::Relaxed);
            st.position.store(deck.position_secs());
            st.duration.store(deck.duration_secs());
            st.speed.store(deck.speed());
            st.rate.store(deck.rate());
            st.playing.store(deck.is_playing(), Ordering::Relaxed);
            if deck.take_ended() {
                st.ended.fetch_add(1, Ordering::Relaxed);
            }
        }
        for (lv, pk) in s.levels.iter().zip(peaks) {
            lv[0].fetch_max_positive(pk[0]);
            lv[1].fetch_max_positive(pk[1]);
        }
        s.crossfader.store(self.xf);
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

#[inline]
fn write_frame(frame: &mut [f32], mode: HeadMode, master: [f32; 2], head: [f32; 2]) {
    let mono = |s: [f32; 2]| 0.5 * (s[0] + s[1]);
    let (l, r) = match mode {
        HeadMode::Off | HeadMode::Outputs34 | HeadMode::Device => (master[0], master[1]),
        HeadMode::Split => (mono(master), mono(head)),
        HeadMode::Solo => (head[0], head[1]),
    };
    match frame.len() {
        1 => frame[0] = soft_clip(0.5 * (l + r)),
        len => {
            frame[0] = soft_clip(l);
            frame[1] = soft_clip(r);
            if len >= 4 && mode == HeadMode::Outputs34 {
                frame[2] = soft_clip(head[0]);
                frame[3] = soft_clip(head[1]);
                frame[4..].fill(0.0);
            } else {
                frame[2..].fill(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, sr: u32) -> Arc<TrackAudio> {
        let n = (secs * sr as f32) as usize;
        Arc::new(TrackAudio::new(sr, (0..n).map(|i| {
            let v = (i as f32 * 0.05).sin() * 0.5;
            [v, v]
        }).collect()))
    }

    fn run(e: &mut Engine, secs: f32) -> Vec<f32> {
        let frames = (secs * e.shared.sample_rate as f32) as usize;
        let mut out = vec![0.0; frames * 2];
        for chunk in out.chunks_mut(512 * 2) {
            e.process(chunk, 2);
        }
        out
    }

    #[test]
    fn crossfader_selects_deck() {
        let (mut e, mut c) = Engine::new(8000, 2);
        c.send(Command::SetMotor(Motor { spin_up_secs: 0.01, spin_down_secs: 0.01 }));
        c.send(Command::Load { deck: 1, id: 1, track: tone(5.0, 8000), keep_playing: false, start_secs: 0.0 });
        c.send(Command::Play { deck: 1 });
        // Crossfader ganz auf A: B ist nicht zu hören
        let quiet = run(&mut e, 0.5);
        assert!(quiet.iter().all(|s| s.abs() < 1e-6));
        c.send(Command::SetCrossfader(1.0));
        run(&mut e, 0.2);
        let loud = run(&mut e, 0.2);
        assert!(loud.iter().any(|s| s.abs() > 0.2));
        // Kanalpegel B ist schon vorher sichtbar gewesen
        let lv = c.shared.take_levels();
        assert!(lv[1][0] > 0.2);
    }

    #[test]
    fn pfl_goes_to_right_channel_in_split_mode() {
        let (mut e, mut c) = Engine::new(8000, 2);
        c.send(Command::SetMotor(Motor { spin_up_secs: 0.01, spin_down_secs: 0.01 }));
        c.send(Command::Load { deck: 1, id: 1, track: tone(5.0, 8000), keep_playing: false, start_secs: 0.0 });
        c.send(Command::Play { deck: 1 });
        c.send(Command::SetPfl { deck: 1, on: true });
        c.send(Command::SetHeadMode(HeadMode::Split));
        run(&mut e, 0.2);
        let out = run(&mut e, 0.2);
        let left = out.iter().step_by(2).fold(0.0f32, |m, s| m.max(s.abs()));
        let right = out.iter().skip(1).step_by(2).fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(left < 1e-6, "Master ist still, weil der Crossfader auf A steht");
        assert!(right > 0.2, "Kopfhörer hört B vor");
    }

    #[test]
    fn pfl_and_preview_skip_motor_ramps() {
        let (mut e, mut c) = Engine::new(8000, 2);
        c.send(Command::SetMotor(Motor { spin_up_secs: 2.0, spin_down_secs: 2.0 }));
        for v in [0, 1, PREVIEW] {
            c.send(Command::Load { deck: v, id: 1, track: tone(5.0, 8000), keep_playing: false, start_secs: 0.0 });
            c.send(Command::Play { deck: v });
        }
        c.send(Command::SetPfl { deck: 1, on: true });
        run(&mut e, 0.1);
        let shared = c.shared.clone();
        let speed = |v: usize| shared.decks[v].speed();
        assert!(speed(0) < 0.1, "A ohne Kopfhörer läuft langsam an");
        assert_eq!(speed(1), 1.0, "B im Kopfhörer läuft sofort");
        assert_eq!(speed(PREVIEW), 1.0, "Vorhören läuft sofort");
        for v in [1, PREVIEW] {
            c.send(Command::Pause { deck: v });
        }
        run(&mut e, 0.05);
        assert_eq!(speed(1), 0.0);
        assert_eq!(speed(PREVIEW), 0.0);
    }

    #[test]
    fn ejected_tracks_are_returned_for_cleanup() {
        let (mut e, mut c) = Engine::new(8000, 2);
        let t = tone(1.0, 8000);
        c.send(Command::Load { deck: 0, id: 1, track: t.clone(), keep_playing: false, start_secs: 0.0 });
        run(&mut e, 0.01);
        c.send(Command::Eject { deck: 0 });
        run(&mut e, 0.01);
        assert_eq!(Arc::strong_count(&t), 2, "Engine hält den Titel nicht mehr, Rückweg schon");
        c.collect_garbage();
        assert_eq!(Arc::strong_count(&t), 1);
    }
}
