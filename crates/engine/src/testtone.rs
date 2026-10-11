// SPDX-License-Identifier: GPL-3.0-or-later
//! Testton für die Geräteauswahl: zwei kurze Töne (tief, dann hoch), selbst erzeugt.
//! Läuft hinter Master- bzw. Kopfhörerlautstärke, damit er auch bei zugezogenem Regler zu hören ist.

use std::f32::consts::TAU;

/// Länge und Höhe der beiden Töne, Pause dazwischen (Sekunden, Hz).
const NOTE_SECS: f32 = 0.22;
const GAP_SECS: f32 = 0.08;
const NOTES_HZ: [f32; 2] = [660.0, 880.0];
/// Ein- und Ausblenden je Ton gegen Knacksen.
const RAMP_SECS: f32 = 0.01;
/// Etwa −12 dBFS: deutlich hörbar, nicht erschreckend laut.
const LEVEL: f32 = 0.25;

#[derive(Clone, Copy, Default)]
pub struct TestTone {
    /// Gespielte Zeit in Sekunden; `None` = still.
    t: Option<f32>,
    phase: f32,
    to_master: bool,
}

impl TestTone {
    /// Startet den Ton von vorn, über Master oder (`to_master` = false) auf dem Kopfhörer.
    pub fn start(&mut self, to_master: bool) {
        *self = Self { t: Some(0.0), phase: 0.0, to_master };
    }

    pub fn is_playing(&self) -> bool {
        self.t.is_some()
    }

    /// Nächster Wert für Master und Kopfhörer.
    pub fn next(&mut self, rate: f32) -> (f32, f32) {
        let Some(t) = self.t else { return (0.0, 0.0) };
        let total = 2.0 * NOTE_SECS + GAP_SECS;
        if t >= total || rate <= 0.0 {
            self.t = None;
            return (0.0, 0.0);
        }
        self.t = Some(t + 1.0 / rate);
        let (note, local) = if t < NOTE_SECS { (0, t) } else { (1, t - NOTE_SECS - GAP_SECS) };
        if !(0.0..NOTE_SECS).contains(&local) {
            return (0.0, 0.0);
        }
        let env = (local / RAMP_SECS).min((NOTE_SECS - local) / RAMP_SECS).clamp(0.0, 1.0);
        self.phase = (self.phase + NOTES_HZ[note] / rate).fract();
        let s = (self.phase * TAU).sin() * env * LEVEL;
        if self.to_master { (s, 0.0) } else { (0.0, s) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_once_on_the_chosen_path() {
        let rate = 1000.0;
        let mut tone = TestTone::default();
        assert_eq!(tone.next(rate), (0.0, 0.0));
        tone.start(false);
        let samples: Vec<(f32, f32)> = (0..1000).map(|_| tone.next(rate)).collect();
        assert!(samples.iter().all(|&(m, _)| m == 0.0), "nichts auf Master");
        let peak = samples.iter().map(|&(_, c)| c.abs()).fold(0.0, f32::max);
        assert!(peak > 0.2 && peak <= LEVEL, "Spitze {peak}");
        assert!(!tone.is_playing(), "nach gut einer halben Sekunde zu Ende");
    }
}
