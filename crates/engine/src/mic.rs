// SPDX-License-Identifier: GPL-3.0-or-later
//! Mikrofon-Durchsage: Talkover senkt die Musik, solange jemand spricht.
//!
//! Der Mikrofonpegel läuft durch eine Hüllkurve, die schnell anspringt und langsam abfällt (so pumpt die
//! Musik nicht zwischen zwei Wörtern). Liegt sie über `THRESHOLD`, wird die Musik weich auf `depth` abgesenkt.

/// Ab diesem Pegel (etwa −36 dBFS) gilt das Mikrofon als besprochen.
pub const THRESHOLD: f32 = 0.016;
/// Abfall der Hüllkurve: so lange hält eine Sprechpause die Musik noch unten.
const HOLD_SECS: f32 = 0.6;
const DUCK_SECS: f32 = 0.08;
const RETURN_SECS: f32 = 0.8;

pub struct Talkover {
    on: bool,
    /// Pegel der Musik beim Sprechen (Faktor).
    depth: f32,
    env: f32,
    gain: f32,
    env_fall: f32,
    duck: f32,
    back: f32,
}

impl Talkover {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        let coef = |secs: f32| 1.0 - (-1.0 / (secs * rate)).exp();
        Talkover { on: true, depth: 0.25, env: 0.0, gain: 1.0, env_fall: coef(HOLD_SECS), duck: coef(DUCK_SECS), back: coef(RETURN_SECS) }
    }

    pub fn set(&mut self, on: bool, depth: f32) {
        self.on = on;
        self.depth = depth.clamp(0.0, 1.0);
    }

    /// Faktor für die Musik bei Mikrofonpegel `level` (Betrag des aktuellen Samples).
    #[inline]
    pub fn next(&mut self, level: f32) -> f32 {
        if level > self.env {
            self.env = level;
        } else {
            self.env -= self.env * self.env_fall;
        }
        let target = if self.on && self.env > THRESHOLD { self.depth } else { 1.0 };
        let k = if target < self.gain { self.duck } else { self.back };
        self.gain += (target - self.gain) * k;
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaking_lowers_the_music_and_silence_brings_it_back() {
        let rate = 48_000;
        let mut t = Talkover::new(rate);
        t.set(true, 0.25);
        let mut g = 1.0;
        // Eine halbe Sekunde sprechen
        for i in 0..rate / 2 {
            g = t.next(0.2 * ((i as f32) * 0.05).sin().abs());
        }
        assert!((g - 0.25).abs() < 0.02, "abgesenkt: {g}");
        // Kurze Pause zwischen zwei Wörtern: bleibt unten
        for _ in 0..rate / 10 {
            g = t.next(0.0);
        }
        assert!(g < 0.3, "kurze Pause hält: {g}");
        // Fünf Sekunden still: wieder voll
        for _ in 0..rate * 5 {
            g = t.next(0.0);
        }
        assert!(g > 0.97, "zurück: {g}");
    }

    #[test]
    fn off_never_lowers() {
        let mut t = Talkover::new(48_000);
        t.set(false, 0.1);
        for _ in 0..48_000 {
            assert_eq!(t.next(0.9), 1.0);
        }
    }
}
