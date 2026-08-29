// SPDX-License-Identifier: GPL-3.0-or-later
use fw_engine::TrackAudio;

/// Auflösung der Wellenform: Werte pro Sekunde.
pub const WAVE_RATE: f32 = 100.0;

/// Wellenform für die Anzeige: Spitzenwert je 10 ms, normiert auf 0..1.
#[derive(Debug, Clone, Default)]
pub struct Waveform {
    pub peaks: Vec<f32>,
}

impl Waveform {
    pub fn from_audio(audio: &TrackAudio) -> Self {
        let hop = (audio.sample_rate as f32 / WAVE_RATE).max(1.0) as usize;
        let mut peaks: Vec<f32> = audio
            .frames
            .chunks(hop)
            .map(|c| c.iter().fold(0.0f32, |m, f| m.max((0.5 * (f[0] + f[1])).abs())))
            .collect();
        let max = peaks.iter().cloned().fold(0.0f32, f32::max);
        if max > 0.0 {
            peaks.iter_mut().for_each(|p| *p /= max);
        }
        Self { peaks }
    }

    pub fn duration_secs(&self) -> f32 {
        self.peaks.len() as f32 / WAVE_RATE
    }

    /// Spitzenwert zur Zeit `secs`, 0 außerhalb.
    pub fn at(&self, secs: f64) -> f32 {
        if secs < 0.0 {
            return 0.0;
        }
        self.peaks.get((secs * WAVE_RATE as f64) as usize).copied().unwrap_or(0.0)
    }

    /// Größter Wert im Zeitbereich, für die verkleinerte Übersicht.
    pub fn max_in(&self, from_secs: f64, to_secs: f64) -> f32 {
        let a = (from_secs.max(0.0) * WAVE_RATE as f64) as usize;
        let b = ((to_secs.max(0.0) * WAVE_RATE as f64) as usize).max(a + 1).min(self.peaks.len());
        self.peaks.get(a..b).map_or(0.0, |s| s.iter().cloned().fold(0.0, f32::max))
    }
}
