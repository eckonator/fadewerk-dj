// SPDX-License-Identifier: GPL-3.0-or-later
//! 3-Band-EQ je Player, gebaut als Isolator: Zwei Linkwitz-Riley-Weichen (4. Ordnung) teilen das
//! Signal in Bässe, Mitten und Höhen, jedes Band bekommt seinen Pegel, dann wird wieder summiert.
//!
//! Stehen alle Bänder auf 1, ist die Summe ein Allpass: kein Unterschied im Frequenzgang. Pegel 0
//! nimmt ein Band ganz heraus (Kill). Damit sich Bässe und der Rest phasenrichtig ergänzen, laufen
//! die Bässe zusätzlich durch den Allpass der oberen Weiche.
//!
//! Läuft im Audio-Thread: keine Allokation, feste Zustände.

/// Übergang Bässe/Mitten in Hz.
pub const LOW_MID_HZ: f32 = 250.0;
/// Übergang Mitten/Höhen in Hz.
pub const MID_HIGH_HZ: f32 = 2500.0;
/// Zeitkonstante, mit der Pegeländerungen nachgezogen werden (gegen Knacksen).
const GAIN_SMOOTH_SECS: f32 = 0.012;

/// Bänder in der Reihenfolge der Befehle.
pub const LOW: usize = 0;
pub const MID: usize = 1;
pub const HIGH: usize = 2;

/// Biquad-Filter (Transposed Direct Form II).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// Butterworth 2. Ordnung nach dem Audio-EQ-Cookbook.
    fn butterworth(fc: f32, sample_rate: f32, high: bool) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * (fc / sample_rate).min(0.49);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1) = if high { ((1.0 + cos) / 2.0, -(1.0 + cos)) } else { ((1.0 - cos) / 2.0, 1.0 - cos) };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Linkwitz-Riley 4. Ordnung: zwei gleiche Butterworth-Stufen hintereinander.
#[derive(Clone, Copy, Default)]
struct Lr4([Biquad; 2]);

impl Lr4 {
    fn new(fc: f32, sample_rate: f32, high: bool) -> Self {
        let b = Biquad::butterworth(fc, sample_rate, high);
        Self([b, b])
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.0[0].run(x);
        self.0[1].run(y)
    }
}

/// Weichen für einen Kanal.
#[derive(Clone, Copy, Default)]
struct Split {
    low: Lr4,
    rest: Lr4,
    mid: Lr4,
    high: Lr4,
    /// Allpass der oberen Weiche für die Bässe (Tiefpass + Hochpass bei `MID_HIGH_HZ`).
    low_ap_lp: Lr4,
    low_ap_hp: Lr4,
}

impl Split {
    fn new(sample_rate: f32) -> Self {
        Self {
            low: Lr4::new(LOW_MID_HZ, sample_rate, false),
            rest: Lr4::new(LOW_MID_HZ, sample_rate, true),
            mid: Lr4::new(MID_HIGH_HZ, sample_rate, false),
            high: Lr4::new(MID_HIGH_HZ, sample_rate, true),
            low_ap_lp: Lr4::new(MID_HIGH_HZ, sample_rate, false),
            low_ap_hp: Lr4::new(MID_HIGH_HZ, sample_rate, true),
        }
    }

    #[inline]
    fn bands(&mut self, x: f32) -> [f32; 3] {
        let low = self.low.run(x);
        let rest = self.rest.run(x);
        let low = self.low_ap_lp.run(low) + self.low_ap_hp.run(low);
        [low, self.mid.run(rest), self.high.run(rest)]
    }
}

/// EQ eines Players (Stereo).
#[derive(Clone)]
pub struct Eq {
    split: [Split; 2],
    gain: [f32; 3],
    target: [f32; 3],
    smooth: f32,
}

impl Eq {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1) as f32;
        Self {
            split: [Split::new(sr); 2],
            gain: [1.0; 3],
            target: [1.0; 3],
            smooth: 1.0 - (-1.0 / (GAIN_SMOOTH_SECS * sr)).exp(),
        }
    }

    /// Pegel eines Bandes als Faktor: 1 = unverändert, 0 = Kill.
    pub fn set_gain(&mut self, band: usize, gain: f32) {
        if let Some(t) = self.target.get_mut(band) {
            *t = gain.clamp(0.0, 4.0);
        }
    }

    pub fn process(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf {
            for (g, t) in self.gain.iter_mut().zip(self.target) {
                *g += (t - *g) * self.smooth;
            }
            for (c, s) in frame.iter_mut().enumerate() {
                let b = self.split[c].bands(*s);
                *s = b[0] * self.gain[LOW] + b[1] * self.gain[MID] + b[2] * self.gain[HIGH];
            }
        }
    }
}

/// Dezibel in Faktor.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Pegel (Effektivwert) eines Sinus nach dem EQ, gemessen nach dem Einschwingen.
    fn rms_after(eq: &mut Eq, hz: f32) -> f32 {
        let n = SR as usize;
        let mut buf: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let v = (2.0 * std::f32::consts::PI * hz * i as f32 / SR as f32).sin();
                [v, v]
            })
            .collect();
        eq.process(&mut buf);
        let tail = &buf[n / 2..];
        (tail.iter().map(|f| f[0] * f[0]).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn neutral_is_flat() {
        let sine_rms = std::f32::consts::FRAC_1_SQRT_2;
        for hz in [40.0, 250.0, 1000.0, 2500.0, 8000.0] {
            let r = rms_after(&mut Eq::new(SR), hz) / sine_rms;
            assert!((r - 1.0).abs() < 0.02, "{hz} Hz: {r}");
        }
    }

    #[test]
    fn kill_removes_band() {
        let sine_rms = std::f32::consts::FRAC_1_SQRT_2;
        let mut eq = Eq::new(SR);
        eq.set_gain(LOW, 0.0);
        assert!(rms_after(&mut eq, 50.0) / sine_rms < 0.03, "Bässe weg");
        let mut eq = Eq::new(SR);
        eq.set_gain(LOW, 0.0);
        assert!(rms_after(&mut eq, 1000.0) / sine_rms > 0.95, "Mitten bleiben");
        let mut eq = Eq::new(SR);
        eq.set_gain(HIGH, 0.0);
        assert!(rms_after(&mut eq, 12_000.0) / sine_rms < 0.03, "Höhen weg");
    }

    #[test]
    fn db_conversion() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(-6.0) - 0.501).abs() < 1e-3);
    }
}
