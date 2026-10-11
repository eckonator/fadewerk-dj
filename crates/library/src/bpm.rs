// SPDX-License-Identifier: GPL-3.0-or-later
//! Einfache Tempo-Erkennung: Energie-Anstiege (Onsets) und Autokorrelation.
//!
//! Für Titel mit klarem Beat zuverlässig genug als Startwert. Eine genauere
//! Analyse mit Beatraster kann später an dieser Stelle andocken.

use fw_engine::TrackAudio;

const HOP: usize = 256;
const MIN_BPM: f32 = 70.0;
const MAX_BPM: f32 = 180.0;
/// Höchstens so viele Sekunden aus der Titelmitte analysieren.
const WINDOW_SECS: f64 = 90.0;

/// Schätzt das Tempo in BPM, `None` bei zu kurzem oder beatlosem Material.
pub fn estimate_bpm(audio: &TrackAudio) -> Option<f32> {
    let sr = audio.sample_rate as f64;
    let len = audio.frames.len();
    let win = ((WINDOW_SECS * sr) as usize).min(len);
    let start = (len - win) / 2;
    let frames = &audio.frames[start..start + win];

    // Energie je Hop, dann positive Änderung als Onset-Kurve
    let energy: Vec<f32> = frames
        .chunks(HOP)
        .map(|c| c.iter().map(|f| { let m = 0.5 * (f[0] + f[1]); m * m }).sum::<f32>().sqrt())
        .collect();
    if energy.len() < 64 {
        return None;
    }
    let raw: Vec<f32> = energy.windows(2).map(|w| (w[1] - w[0]).max(0.0)).collect();
    // Leicht weichzeichnen, damit Beatabstände zwischen zwei ganzzahligen Lags trotzdem korrelieren
    const K: [f32; 5] = [1.0, 2.0, 3.0, 2.0, 1.0];
    let mut onset: Vec<f32> = (0..raw.len())
        .map(|i| K.iter().enumerate().map(|(k, w)| w * raw.get((i + k).wrapping_sub(2)).copied().unwrap_or(0.0)).sum())
        .collect();
    let mean = onset.iter().sum::<f32>() / onset.len() as f32;
    onset.iter_mut().for_each(|v| *v -= mean);

    let hops_per_sec = sr as f32 / HOP as f32;
    let lag_min = (60.0 / MAX_BPM * hops_per_sec).floor() as usize;
    let lag_max = (60.0 / MIN_BPM * hops_per_sec).ceil() as usize;
    if lag_max * 3 >= onset.len() / 2 {
        return None;
    }

    let ac = |lag: usize| -> f32 { onset.iter().zip(&onset[lag..]).map(|(a, b)| a * b).sum() };
    let zero = ac(0);
    if zero <= 0.0 {
        return None;
    }
    let mut best = (0usize, f32::MIN);
    for lag in lag_min..=lag_max {
        let bpm = 60.0 * hops_per_sec / lag as f32;
        // Leichte Vorliebe für das typische Tanzmusik-Tempo, damit halbe/doppelte Werte seltener gewinnen
        let weight = (-0.5 * ((bpm / 125.0).log2() / 0.9).powi(2)).exp();
        // Vielfache des Beatabstands mitzählen: das echte Tempo hat auch bei 2× und 3× Lag Treffer,
        // das halbe Tempo nur bei jedem zweiten Schlag
        let score = (ac(lag) + 0.5 * ac(2 * lag) + 0.33 * ac(3 * lag)) * weight;
        if score > best.1 {
            best = (lag, score);
        }
    }
    if best.1 <= 0.05 * zero {
        return None;
    }
    // Parabel durch die Nachbarn für einen Wert zwischen den ganzzahligen Lags
    let l = best.0;
    let (y0, y1, y2) = (ac(l - 1), ac(l), ac(l + 1));
    let denom = y0 - 2.0 * y1 + y2;
    let shift = if denom.abs() > 1e-9 { (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    let bpm = 60.0 * hops_per_sec / (l as f32 + shift);
    Some((bpm * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_track(bpm: f32, secs: f32, sr: u32) -> TrackAudio {
        let n = (secs * sr as f32) as usize;
        let period = 60.0 / bpm * sr as f32;
        let frames = (0..n)
            .map(|i| {
                let ph = (i as f32 % period) / sr as f32;
                let v = (-ph * 40.0).exp() * (i as f32 * 0.3).sin();
                [v, v]
            })
            .collect();
        TrackAudio::new(sr, frames)
    }

    #[test]
    fn finds_tempo_of_click_tracks() {
        for bpm in [96.0, 123.0, 128.0, 140.0] {
            let est = estimate_bpm(&click_track(bpm, 40.0, 22_050)).unwrap();
            assert!((est - bpm).abs() < 1.5, "erwartet {bpm}, erkannt {est}");
        }
    }

    #[test]
    fn silence_has_no_tempo() {
        assert_eq!(estimate_bpm(&TrackAudio::new(22_050, vec![[0.0; 2]; 22_050 * 30])), None);
    }
}
