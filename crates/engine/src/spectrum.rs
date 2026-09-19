// SPDX-License-Identifier: GPL-3.0-or-later
//! Frequenzbänder des Master-Signals für die MAIN-Anzeige: zehn Oktav-Bandpässe (31 Hz bis 16 kHz)
//! auf dem Mono-Signal, je Band der Spitzenwert seit dem letzten Abholen.
//!
//! Läuft im Audio-Thread: keine Allokation, feste Zustände. Zehn Biquads je Sample kosten kaum Rechenzeit.

/// Anzahl der Bänder.
pub const BANDS: usize = 10;
/// Mittenfrequenzen der Bänder in Hz (Oktaven).
pub const BAND_HZ: [f32; BANDS] = [31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Güte für eine Oktave Bandbreite.
const Q: f32 = std::f32::consts::SQRT_2;

/// Bandpass mit 0 dB in der Mitte (Audio-EQ-Cookbook), Transposed Direct Form II.
#[derive(Clone, Copy, Default)]
struct BandPass {
    b0: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl BandPass {
    fn new(fc: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * (fc / sample_rate).min(0.45);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * Q);
        let a0 = 1.0 + alpha;
        Self { b0: alpha / a0, a1: -2.0 * cos / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    /// b1 = 0, b2 = −b0.
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = -self.b0 * x - self.a2 * y;
        y
    }
}

pub struct Spectrum {
    filters: [BandPass; BANDS],
    peaks: [f32; BANDS],
}

impl Spectrum {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1) as f32;
        Self { filters: BAND_HZ.map(|f| BandPass::new(f, sr)), peaks: [0.0; BANDS] }
    }

    /// Ein Sample (Mono) verarbeiten.
    #[inline]
    pub fn push(&mut self, x: f32) {
        for (f, p) in self.filters.iter_mut().zip(self.peaks.iter_mut()) {
            *p = p.max(f.run(x).abs());
        }
    }

    /// Spitzenwerte seit dem letzten Aufruf, danach zurückgesetzt.
    pub fn take(&mut self) -> [f32; BANDS] {
        std::mem::replace(&mut self.peaks, [0.0; BANDS])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spitzenwert je Band für einen Sinus.
    fn response(hz: f32) -> [f32; BANDS] {
        let sr = 48_000.0;
        let mut s = Spectrum::new(sr as u32);
        // Einschwingen, dann messen
        for i in 0..48_000 {
            s.push((2.0 * std::f32::consts::PI * hz * i as f32 / sr).sin());
        }
        s.take();
        for i in 48_000..96_000 {
            s.push((2.0 * std::f32::consts::PI * hz * i as f32 / sr).sin());
        }
        s.take()
    }

    #[test]
    fn a_sine_lights_its_own_band() {
        for (band, hz) in BAND_HZ.iter().enumerate() {
            let r = response(*hz);
            let best = r.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
            assert_eq!(best, band, "{hz} Hz: {r:?}");
            // In der Bandmitte 0 dB. Hohe Töne treffen mit wenigen Samples je Schwingung den Scheitel
            // nicht (bei 8 kHz und 48 kHz höchstens sin 60° ≈ 0,87)
            let low = if *hz <= 2000.0 { 0.95 } else { 0.85 };
            assert!(r[band] > low && r[band] < 1.05, "{hz} Hz: {}", r[band]);
            // Zwei Oktaven daneben deutlich leiser
            if band >= 2 {
                assert!(r[band - 2] < 0.3, "{hz} Hz: {r:?}");
            }
        }
    }
}
