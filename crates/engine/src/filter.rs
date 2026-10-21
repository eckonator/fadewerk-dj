// SPDX-License-Identifier: GPL-3.0-or-later
//! Filter je Player mit einem Regler: Mitte neutral, nach links Tiefpass, nach rechts Hochpass.
//!
//! Zwei Zustandsvariablen-Filter (Trapez-Integration nach A. Simper) hintereinander, die immer
//! mitlaufen, damit beim Verlassen der Mitte nichts knackt. In der Mitte wird das trockene Signal
//! ausgegeben, kurz daneben weich auf das gefilterte übergeblendet. Mit dem Ausschlag steigt die Resonanz leicht.

use std::f32::consts::PI;

/// Unterste Grenzfrequenz des Tiefpasses (Regler ganz links).
const LP_MIN_HZ: f32 = 60.0;
/// Oberste Grenzfrequenz des Hochpasses (Regler ganz rechts).
const HP_MAX_HZ: f32 = 8000.0;
/// Grenzfrequenz des Hochpasses, solange er nicht arbeitet.
const HP_REST_HZ: f32 = 15.0;
/// Bis zu diesem Ausschlag wird vom trockenen auf das gefilterte Signal übergeblendet.
const WET_RANGE: f32 = 0.05;
/// So lange braucht der Filter für den ganzen Reglerweg (gegen Zippern bei schnellem Drehen).
const FULL_TRAVEL_SECS: f32 = 0.03;
/// Koeffizienten werden alle so viele Frames neu berechnet.
const COEFF_EVERY: usize = 16;

#[derive(Clone, Copy, Default)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Svf {
    fn set(&mut self, hz: f32, q: f32, rate: f32) {
        let g = (PI * hz / rate).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Liefert Tief- und Hochpass für Kanal `c`.
    #[inline]
    fn tick(&mut self, c: usize, v0: f32) -> (f32, f32) {
        let v3 = v0 - self.ic2[c];
        let v1 = self.a1 * self.ic1[c] + self.a2 * v3;
        let v2 = self.ic2[c] + self.a2 * self.ic1[c] + self.a3 * v3;
        self.ic1[c] = 2.0 * v1 - self.ic1[c];
        self.ic2[c] = 2.0 * v2 - self.ic2[c];
        (v2, v0 - self.k * v1 - v2)
    }
}

/// Art und Grenzfrequenz bei Reglerstellung `value` für die Anzeige: `Some((true, hz))` = Tiefpass,
/// `Some((false, hz))` = Hochpass, `None` = neutral.
pub fn cutoff_hz(value: f32) -> Option<(bool, f32)> {
    let top = 20_000.0f32;
    if value <= -WET_RANGE / 2.0 {
        Some((true, top * (LP_MIN_HZ / top).powf(-value.max(-1.0))))
    } else if value >= WET_RANGE / 2.0 {
        Some((false, HP_REST_HZ * (HP_MAX_HZ / HP_REST_HZ).powf(value.min(1.0))))
    } else {
        None
    }
}

pub struct Filter {
    rate: f32,
    target: f32,
    current: f32,
    lp: Svf,
    hp: Svf,
    countdown: usize,
}

impl Filter {
    pub fn new(sample_rate: u32) -> Self {
        let mut f = Filter { rate: sample_rate.max(1) as f32, target: 0.0, current: 0.0, lp: Svf::default(), hp: Svf::default(), countdown: 0 };
        f.update_coeffs();
        f
    }

    /// Reglerstellung −1 (Tiefpass ganz zu) bis +1 (Hochpass ganz offen nach oben), 0 = neutral.
    pub fn set(&mut self, value: f32) {
        self.target = value.clamp(-1.0, 1.0);
    }

    fn update_coeffs(&mut self) {
        let x = self.current;
        let nyquist_safe = (0.45 * self.rate).min(20_000.0);
        let lp_hz = if x < 0.0 { nyquist_safe * (LP_MIN_HZ / nyquist_safe).powf(-x) } else { nyquist_safe };
        let hp_hz = if x > 0.0 { HP_REST_HZ * (HP_MAX_HZ / HP_REST_HZ).powf(x) } else { HP_REST_HZ };
        let hp_hz = hp_hz.min(nyquist_safe);
        let q = std::f32::consts::FRAC_1_SQRT_2 + 0.8 * x.abs();
        self.lp.set(lp_hz, if x < 0.0 { q } else { std::f32::consts::FRAC_1_SQRT_2 }, self.rate);
        self.hp.set(hp_hz, if x > 0.0 { q } else { std::f32::consts::FRAC_1_SQRT_2 }, self.rate);
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        let step = 2.0 / (FULL_TRAVEL_SECS * self.rate);
        for frame in buf.iter_mut() {
            if self.countdown == 0 {
                if self.current != self.target {
                    self.current += (self.target - self.current).clamp(-step * COEFF_EVERY as f32, step * COEFF_EVERY as f32);
                }
                self.update_coeffs();
                self.countdown = COEFF_EVERY;
            }
            self.countdown -= 1;
            let t = (self.current.abs() / WET_RANGE).min(1.0);
            let wet = t * t * (3.0 - 2.0 * t);
            for (c, sample) in frame.iter_mut().enumerate() {
                let dry = *sample;
                let (lp, _) = self.lp.tick(c, dry);
                let (_, hp) = self.hp.tick(c, lp);
                *sample = dry + (hp - dry) * wet;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: f32, n: usize) -> Vec<[f32; 2]> {
        (0..n).map(|i| {
            let v = (2.0 * PI * hz * i as f32 / rate).sin() * 0.5;
            [v, v]
        }).collect()
    }

    fn peak(buf: &[[f32; 2]]) -> f32 {
        buf.iter().fold(0.0f32, |m, f| m.max(f[0].abs()))
    }

    #[test]
    fn neutral_is_bit_exact() {
        let mut f = Filter::new(48_000);
        let input = sine(1000.0, 48_000.0, 4800);
        let mut buf = input.clone();
        f.process(&mut buf);
        assert_eq!(buf, input);
    }

    #[test]
    fn lowpass_removes_highs_and_highpass_removes_lows() {
        let rate = 48_000.0;
        let mut f = Filter::new(48_000);
        f.set(-1.0);
        let mut warm = sine(5000.0, rate, 48_000);
        f.process(&mut warm);
        assert!(peak(&warm[24_000..]) < 0.01, "Tiefpass ganz zu lässt 5 kHz durch: {}", peak(&warm[24_000..]));

        let mut f = Filter::new(48_000);
        f.set(1.0);
        let mut low = sine(80.0, rate, 48_000);
        f.process(&mut low);
        assert!(peak(&low[24_000..]) < 0.01, "Hochpass ganz offen lässt 80 Hz durch: {}", peak(&low[24_000..]));
    }

    #[test]
    fn stays_finite_when_swept_fast() {
        let mut f = Filter::new(44_100);
        let input = sine(300.0, 44_100.0, 256);
        for i in 0..2000 {
            f.set(if i % 2 == 0 { -1.0 } else { 1.0 });
            let mut buf = input.clone();
            f.process(&mut buf);
            assert!(buf.iter().all(|s| s[0].is_finite() && s[0].abs() < 10.0));
        }
    }
}
