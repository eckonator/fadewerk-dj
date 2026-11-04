// SPDX-License-Identifier: GPL-3.0-or-later
//! Limiter für den Master: hält Spitzen unter der Grenze, statt sie hart abzuschneiden.
//!
//! Das Signal läuft um `LOOKAHEAD_SECS` verzögert durch; der Pegel wird vorab gemessen, damit die
//! Absenkung schon greift, wenn die Spitze ankommt. Darunter bleibt das Signal unverändert.

/// Höchster Ausgangspegel (−0,3 dBFS).
pub const THRESHOLD: f32 = 0.966;
const LOOKAHEAD_SECS: f32 = 0.002;
const RELEASE_SECS: f32 = 0.15;
/// Platz für die Vorausschau bis 192 kHz.
const MAX_DELAY: usize = 400;

pub struct Limiter {
    delay: [[f32; 2]; MAX_DELAY],
    len: usize,
    idx: usize,
    /// Gehaltene Spitze der Vorausschau.
    env: f32,
    hold: usize,
    gain: f32,
    attack: f32,
    release: f32,
    on: bool,
}

impl Limiter {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1) as f32;
        let len = ((LOOKAHEAD_SECS * rate) as usize).clamp(1, MAX_DELAY);
        Limiter {
            delay: [[0.0; 2]; MAX_DELAY],
            len,
            idx: 0,
            env: 0.0,
            hold: 0,
            gain: 1.0,
            // Absenken in etwa einem Drittel der Vorausschau (bis die Spitze da ist, ist sie auf ~95 %)
            attack: 1.0 - (-3.0 / len as f32).exp(),
            release: 1.0 - (-1.0 / (RELEASE_SECS * rate)).exp(),
            on: true,
        }
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    /// Aktuelle Absenkung als Faktor (1 = keine).
    pub fn gain(&self) -> f32 {
        self.gain
    }

    #[inline]
    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        // Auch ausgeschaltet läuft die Verzögerung weiter, damit das Umschalten nicht springt
        let out = self.delay[self.idx];
        self.delay[self.idx] = x;
        self.idx = (self.idx + 1) % self.len;
        let peak = x[0].abs().max(x[1].abs());
        if peak >= self.env {
            self.env = peak;
            self.hold = self.len;
        } else if self.hold > 0 {
            self.hold -= 1;
        } else {
            self.env += (peak - self.env) * self.release;
        }
        let target = if self.on && self.env > THRESHOLD { THRESHOLD / self.env } else { 1.0 };
        let k = if target < self.gain { self.attack } else { self.release };
        self.gain += (target - self.gain) * k;
        // Rest der Absenkung (erste Spitze nach Stille) fängt die Grenze ab
        let g = self.gain;
        let lim = if self.on { THRESHOLD } else { f32::INFINITY };
        [(out[0] * g).clamp(-lim, lim), (out[1] * g).clamp(-lim, lim)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_signal_passes_unchanged_and_delayed() {
        let mut l = Limiter::new(48_000);
        let input: Vec<[f32; 2]> = (0..2000).map(|i| [0.5 * (i as f32 * 0.05).sin(), -0.3]).collect();
        let out: Vec<[f32; 2]> = input.iter().map(|&x| l.process(x)).collect();
        for i in l.len..input.len() {
            assert_eq!(out[i], input[i - l.len]);
        }
    }

    #[test]
    fn loud_signal_stays_below_threshold() {
        let mut l = Limiter::new(48_000);
        let mut max = 0.0f32;
        for i in 0..48_000 {
            let s = 3.0 * (i as f32 * 0.03).sin();
            let y = l.process([s, s]);
            max = max.max(y[0].abs());
        }
        assert!(max <= THRESHOLD);
        assert!(l.gain() < 0.5, "senkt deutlich ab");
    }

    #[test]
    fn off_only_delays() {
        let mut l = Limiter::new(48_000);
        l.set_on(false);
        let mut out = [0.0; 2];
        for _ in 0..l.len + 1 {
            out = l.process([2.0, -2.0]);
        }
        assert_eq!(out, [2.0, -2.0]);
    }
}
