// SPDX-License-Identifier: GPL-3.0-or-later
//! Vorschläge für FadeWerk-Bereiche aus der Wellenform-Skizze (erste und letzte 30 s) und dem Beatraster.
//!
//! Einblenden: ab dem ersten Taktbeginn, an dem die Musik da ist (Stille davor fällt weg). Ausblenden: endet an
//! der letzten Taktgrenze, bevor der Titel selbst nachlässt (Ausfaden, Ausklang), so dass er aufhört, bevor er
//! zerfällt. Beide so lang wie die Überblendzeit, mit Raster auf ganze Takte gerundet; ohne Raster nur nach der
//! Lautstärke.

use fw_library::sketch::{EDGE_RATE, WaveSketch};
use fw_library::{BeatGrid, Segment};

use crate::fadewerk::{Region, TrackFade};

/// Ab diesem Anteil der lautesten Stelle im Anfang gilt die Musik als da.
const MUSIC_LEVEL: f32 = 0.2;
/// Unter diesem Anteil des Pegels im Hauptteil gilt das Ende als nachlassend.
const DROP_LEVEL: f32 = 0.6;
/// Glättung der Pegelkurve (Sekunden).
const SMOOTH_SECS: f32 = 0.5;

/// Vorschlag für einen Titel mit Skizze `sk` und (falls bekannt) Beatraster.
pub fn suggest(sk: &WaveSketch, grid: Option<BeatGrid>, fade_secs: f32) -> TrackFade {
    let dur = sk.duration_secs;
    if dur <= 1.0 || sk.head.is_empty() || sk.tail.is_empty() {
        return TrackFade::default();
    }
    let fade_secs = fade_secs.max(0.5);
    let head_end = sk.head.len() as f32 / EDGE_RATE;
    let tail_start = dur - sk.tail.len() as f32 / EDGE_RATE;
    let start_seg = grid.and_then(|g| g.at(0.0, dur as f64));
    let end_seg = grid.and_then(|g| g.at(dur as f64, dur as f64));

    // Einblenden: erster Taktbeginn ab dem Einsatz der Musik
    let level = smooth(&sk.head);
    let peak = level.iter().copied().fold(0.0, f32::max);
    let onset = level.iter().position(|&v| v >= MUSIC_LEVEL * peak).map_or(0.0, |i| i as f32 / EDGE_RATE);
    let fade_in = match start_seg {
        Some(seg) => {
            let bar = bar_secs(seg);
            let k = ((onset as f64 - seg.downbeat) / bar - 0.02).ceil();
            let start = (seg.downbeat + k * bar) as f32;
            let len = whole_bars(fade_secs, bar);
            Region { start, end: start + len }
        }
        None => Region { start: onset, end: onset + fade_secs },
    };
    let fade_in = (fade_in.end <= head_end && fade_in.start >= 0.0).then_some(fade_in);

    // Ausblenden: Pegel im Hauptteil des Endstücks, dann die Stelle, ab der er dauerhaft darunter bleibt
    let level = smooth(&sk.tail);
    let body = median(&level[..level.len() / 2]);
    let last_loud = level.iter().rposition(|&v| v >= DROP_LEVEL * body).map_or(level.len(), |i| i + 1);
    let drop = tail_start + last_loud as f32 / EDGE_RATE;
    let fade_out = match end_seg {
        Some(seg) => {
            let bar = bar_secs(seg);
            let k = ((drop as f64 - seg.downbeat) / bar + 0.02).floor();
            let end = (seg.downbeat + k * bar) as f32;
            Region { start: end - whole_bars(fade_secs, bar), end }
        }
        None => Region { start: drop - fade_secs, end: drop },
    };
    let fade_out = (fade_out.start >= tail_start && fade_out.end <= dur && fade_out.len() > 0.0).then_some(fade_out);
    TrackFade { fade_in, fade_out }
}

/// Zeitpunkt auf den nächsten Schlag gerastet.
pub fn snap_to_beat(t: f32, seg: Segment) -> f32 {
    let beat = seg.beat_secs();
    (seg.downbeat + ((t as f64 - seg.downbeat) / beat).round() * beat) as f32
}

fn bar_secs(seg: Segment) -> f64 {
    4.0 * seg.beat_secs()
}

/// Überblendzeit auf ganze Takte gerundet (mindestens einer).
fn whole_bars(fade_secs: f32, bar: f64) -> f32 {
    ((fade_secs as f64 / bar).round().max(1.0) * bar) as f32
}

/// Pegelkurve, über `SMOOTH_SECS` gemittelt (0 bis 1).
fn smooth(peaks: &[u8]) -> Vec<f32> {
    let w = ((SMOOTH_SECS * EDGE_RATE) as usize).max(1);
    let v: Vec<f32> = peaks.iter().map(|&p| p as f32 / 255.0).collect();
    (0..v.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w / 2), (i + w / 2 + 1).min(v.len()));
            v[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect()
}

fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[s.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skizze eines 200-s-Titels: 2 s Stille am Anfang, ab 180 s Ausfaden bis 0.
    fn sketch() -> WaveSketch {
        let n = (30.0 * EDGE_RATE) as usize;
        let head = (0..n).map(|i| if (i as f32) < 2.0 * EDGE_RATE { 0 } else { 200 }).collect();
        let tail = (0..n)
            .map(|i| {
                let t = 170.0 + i as f32 / EDGE_RATE;
                if t < 180.0 { 200 } else { (200.0 * (1.0 - (t - 180.0) / 20.0)).max(0.0) as u8 }
            })
            .collect();
        WaveSketch { duration_secs: 200.0, overview: Vec::new(), head, tail }
    }

    #[test]
    fn without_grid_by_level_only() {
        let f = suggest(&sketch(), None, 8.0);
        let r = f.fade_in.unwrap();
        assert!((r.start - 2.0).abs() < 0.3, "Einstieg nach der Stille: {r:?}");
        assert!((r.len() - 8.0).abs() < 1e-3);
        // Pegel fällt ab 180 s, unter 60 % ab ~188 s: dort endet der Ausblendbereich
        let o = f.fade_out.unwrap();
        assert!((186.0..190.0).contains(&o.end), "{o:?}");
        assert!((o.len() - 8.0).abs() < 1e-3);
    }

    #[test]
    fn with_grid_on_bars() {
        // 120 BPM, Taktbeginn bei 0,5 s: Takte bei 0,5 / 2,5 / 4,5 …
        let seg = Segment { bpm: 120.0, downbeat: 0.5 };
        let grid = BeatGrid { start: Some(seg), end: Some(seg), steady: true };
        let f = suggest(&sketch(), Some(grid), 8.0);
        let r = f.fade_in.unwrap();
        assert!((r.start - 2.5).abs() < 1e-3, "erster Taktbeginn nach 2 s: {r:?}");
        assert!((r.len() - 8.0).abs() < 1e-3, "4 Takte");
        let o = f.fade_out.unwrap();
        // Letzte Taktgrenze vor ~188 s: 186,5 s
        assert!((o.end - 186.5).abs() < 1e-3, "{o:?}");
        assert!((o.len() - 8.0).abs() < 1e-3);
        // 7 s Überblendzeit bei 2-s-Takten: 4 Takte (8 s)
        assert!((suggest(&sketch(), Some(grid), 7.0).fade_in.unwrap().len() - 8.0).abs() < 1e-3);
    }

    #[test]
    fn snaps_to_the_nearest_beat() {
        let seg = Segment { bpm: 120.0, downbeat: 0.25 };
        assert!((snap_to_beat(1.1, seg) - 1.25).abs() < 1e-6);
        assert!((snap_to_beat(0.4, seg) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn empty_sketch_suggests_nothing() {
        assert_eq!(suggest(&WaveSketch::default(), None, 8.0), TrackFade::default());
    }
}
