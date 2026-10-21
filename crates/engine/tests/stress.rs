// SPDX-License-Identifier: GPL-3.0-or-later
//! Stresstest: zehntausende zufällige Befehle und Blockgrößen gegen die Engine. Es darf nichts abstürzen,
//! und das Ausgangssignal muss immer endlich und in vernünftigen Grenzen bleiben.

use std::sync::Arc;

use fw_engine::{Command, Curve, Engine, HeadMode, Motor, TrackAudio, FX_SLOTS, PREVIEW, head_link};

thread_local! {
    /// Auch ungültige Zahlen (NaN) einstreuen.
    static NAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Kleiner, reproduzierbarer Zufallsgenerator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Zahl zwischen `a` und `b`, gelegentlich auch Randwerte und Unsinn (negativ, riesig).
    fn f(&mut self, a: f32, b: f32) -> f32 {
        match self.below(20) {
            0 => a,
            1 => b,
            2 => -b,
            3 => b * 1000.0,
            4 => 0.0,
            5 if NAN.with(|n| n.get()) => f32::NAN,
            _ => a + (b - a) * (self.below(10_000) as f32 / 10_000.0),
        }
    }
}

/// Titel mit Rauschen in verschiedenen Längen und Abtastraten, auch ganz kurz oder leer.
fn track(rng: &mut Rng) -> Arc<TrackAudio> {
    let rate = [8_000, 22_050, 44_100, 48_000, 96_000][rng.below(5) as usize];
    let len = match rng.below(6) {
        0 => 0,
        1 => 1,
        2 => 17,
        _ => (rng.below(3) + 1) as usize * rate as usize / 4,
    };
    let frames = (0..len).map(|_| {
        let v = (rng.below(2000) as f32 / 1000.0 - 1.0) * 0.9;
        [v, -v]
    });
    Arc::new(TrackAudio::new(rate, frames.collect()))
}

fn random_command(rng: &mut Rng) -> Command {
    let deck = rng.below(3) as usize;
    let d2 = rng.below(2) as usize;
    match rng.below(36) {
        0 | 1 => Command::Load { deck, id: rng.next(), track: track(rng), keep_playing: rng.below(2) == 0, start_secs: rng.f(0.0, 2.0) as f64 },
        2 => Command::Eject { deck },
        3 | 4 => Command::Play { deck },
        5 => Command::Pause { deck },
        6 => Command::TogglePlay { deck },
        7 => Command::Stop { deck },
        8 => Command::Cue { deck },
        9 => Command::Seek { deck, secs: rng.f(-1.0, 5.0) as f64 },
        10 => Command::SetPitch { deck, pitch: rng.f(-0.5, 0.5) },
        11 => Command::SetRepeatTrack { deck, on: rng.below(2) == 0 },
        12 => Command::SetReverse { deck, on: rng.below(2) == 0 },
        13 => Command::SetRepeatFade(rng.f(0.0, 20.0)),
        14 => {
            let a = rng.f(0.0, 2.0) as f64;
            Command::SetLoop { deck, range: (rng.below(3) > 0).then(|| (a, a + rng.f(0.0, 1.0) as f64)) }
        }
        15 => Command::ScratchBegin { deck },
        16 | 17 => Command::ScratchTo { deck, secs: rng.f(-1.0, 5.0) as f64 },
        18 => Command::ScratchEnd { deck },
        19 => Command::SetCrossfader(rng.f(-1.0, 1.0)),
        20 => Command::SetCurve(match rng.below(4) {
            0 => Curve::Soft,
            1 => Curve::Linear,
            2 => Curve::Cut,
            _ => Curve::Custom([rng.f(0.0, 1.0), rng.f(0.0, 1.0), rng.f(0.0, 1.0), rng.f(0.0, 1.0)]),
        }),
        21 => Command::SetChannelVolume { deck: d2, volume: rng.f(0.0, 1.0) },
        22 => Command::SetPfl { deck: d2, on: rng.below(2) == 0 },
        23 => Command::SetEq { deck: d2, band: rng.below(3) as usize, gain: rng.f(0.0, 2.0) },
        24 => Command::SetHeadMode([HeadMode::Off, HeadMode::Split, HeadMode::Outputs34, HeadMode::Device][rng.below(4) as usize]),
        25 => Command::SetHeadMix(rng.f(0.0, 1.0)),
        26 => Command::SetMasterVolume(rng.f(0.0, 1.0)),
        27 => Command::SetMotor(Motor { spin_up_secs: rng.f(0.0, 2.0), spin_down_secs: rng.f(0.0, 4.0) }),
        28 => Command::LoadFx { slot: rng.below(FX_SLOTS as u64 + 1) as usize, sound: track(rng) },
        29 => Command::TriggerFx { slot: rng.below(FX_SLOTS as u64 + 1) as usize, to_master: rng.below(2) == 0 },
        30 => Command::SetFxVolume(rng.f(0.0, 1.0)),
        31 => Command::SetSplitFade(rng.below(2) == 0),
        32 => Command::TestTone { to_master: rng.below(2) == 0 },
        33 => Command::SetFilter { deck: d2, value: rng.f(-1.5, 1.5) },
        34 => Command::SetRecording(rng.below(2) == 0),
        _ => Command::SetHeadVolume(rng.f(0.0, 1.0)),
    }
}

#[test]
fn random_commands_never_crash_or_produce_garbage() {
    run(false);
}

/// Auch NaN in Reglerwerten (etwa aus einer Rechnung in der Oberfläche) darf das Signal nicht vergiften.
#[test]
fn invalid_numbers_are_ignored() {
    run(true);
}

fn run(nan: bool) {
    NAN.with(|n| n.set(nan));
    // Im Debug-Build weniger Schritte, damit `cargo test` flott bleibt
    let steps = if cfg!(debug_assertions) { 600 } else { 4_000 };
    for seed in 1..=8u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let channels = if seed % 2 == 0 { 4 } else { 2 };
        let (mut engine, mut ctl) = Engine::new(48_000, channels);
        let (sink, _receiver) = head_link(48_000, 44_100, 1024);
        ctl.send(Command::SetHeadSink(Some(sink)));
        let mut out = vec![0.0f32; 4096 * channels];
        for step in 0..steps {
            for _ in 0..rng.below(4) {
                ctl.send(random_command(&mut rng));
            }
            let frames = match rng.below(10) {
                0 => 0,
                1 => 1,
                2 => 4096,
                _ => rng.below(1024) as usize + 1,
            };
            let block = &mut out[..frames * channels];
            engine.process(block, channels);
            for (i, &s) in block.iter().enumerate() {
                assert!(s.is_finite(), "Seed {seed}, Schritt {step}: Sample {i} ist {s}");
                assert!(s.abs() < 50.0, "Seed {seed}, Schritt {step}: Sample {i} = {s} ist viel zu laut");
            }
            ctl.collect_garbage();
            // Zustand für die Oberfläche bleibt lesbar und sinnvoll
            for d in 0..=PREVIEW {
                let st = &ctl.shared.decks[d];
                assert!(st.position_secs().is_finite() && st.position_secs() >= 0.0, "Position {}", st.position_secs());
                assert!(st.rate().is_finite(), "Geschwindigkeit {}", st.rate());
            }
        }
    }
}
