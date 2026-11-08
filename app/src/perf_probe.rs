// SPDX-License-Identifier: GPL-3.0-or-later
//! Leistung des Rechners im echten Betrieb messen und daraus eine Empfehlung für die Darstellung ableiten.
//!
//! Kein künstlicher Test beim Start (Indexierung, Updates und Turbo-Takt verfälschen ihn, und niemand will warten):
//! Gemessen wird beim ersten Abspielen, solange ein Player spielt und das Fenster zeichnet, `MEASURE_SECS` lang die
//! Zeit je Bild (eframe `cpu_usage`, inklusive Zeichnen) und ob Ton-Aussetzer auftreten. Schnelle Rechner sehen
//! danach nichts; knappe und langsame bekommen eine Empfehlung (Leiste in `app/prefs.rs`). Unter Einstellungen →
//! Leistung lässt sich die Messung wiederholen, dann gibt es immer ein Ergebnis.
//!
//! Schwellen aus Messungen: Apple M4 ~2 ms je Bild, MacBook Pro 2008 (Core 2 Duo, GeForce 8600M) 16 ms mit voller
//! Optik, Netbook (Atom N270, GMA 950) 127 ms voll und 41 ms sparsam.

use crate::theme::Lite;

/// So lange (Sekunden mit spielendem Player und sichtbarem Fenster) wird gemessen.
pub const MEASURE_SECS: f64 = 30.0;
/// Bis hierhin (Millisekunden je Bild, Median) reicht die volle Optik.
const FAST_MS: f32 = 12.0;
/// Darüber ist der Rechner für die volle Optik zu langsam.
const SLOW_MS: f32 = 30.0;
/// Bei manueller Messung: so wenig Arbeit je Bild, dass auch die volle Optik Luft hätte.
const ROOM_MS: f32 = 6.0;

/// Stand einer Messung.
#[derive(Default)]
pub struct Probe {
    /// Zeit je Bild in Sekunden.
    samples: Vec<f32>,
    /// Gemessene Spielzeit in Sekunden.
    secs: f64,
    dropouts: u32,
    /// Von Hand gestartet (Einstellungen): Ergebnis immer zeigen, auch „schnell genug“.
    pub manual: bool,
}

impl Probe {
    pub fn manual() -> Self {
        Self { manual: true, ..Self::default() }
    }

    /// Ein Bild mit spielendem Player: `work` = Rechenzeit dieses Bildes, `dt` = Zeit seit dem letzten.
    pub fn frame(&mut self, work: f32, dt: f64) {
        if work.is_finite() && work > 0.0 {
            self.samples.push(work);
        }
        self.secs += dt.clamp(0.0, 0.5);
    }

    /// Ton-Aussetzer; in den ersten Sekunden nicht gezählt (Laden des ersten Titels, Gerät startet).
    pub fn dropout(&mut self) {
        if self.secs > 5.0 {
            self.dropouts += 1;
        }
    }

    /// Anteil der Messung (0…1).
    pub fn progress(&self) -> f32 {
        (self.secs / MEASURE_SECS).clamp(0.0, 1.0) as f32
    }

    pub fn done(&self) -> bool {
        self.secs >= MEASURE_SECS && self.samples.len() >= 20
    }

    /// Ergebnis der fertigen Messung.
    pub fn result(&self) -> Measured {
        let mut s = self.samples.clone();
        s.sort_by(f32::total_cmp);
        let median_ms = s.get(s.len() / 2).copied().unwrap_or(0.0) * 1000.0;
        let fps = if self.secs > 0.0 { (self.samples.len() as f64 / self.secs) as f32 } else { 0.0 };
        Measured { median_ms, fps, dropouts: self.dropouts }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    pub median_ms: f32,
    pub fps: f32,
    pub dropouts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// Volle Optik passt; nichts zu tun.
    Fast,
    /// Volle Optik nur knapp: leichte Sparmaßnahmen.
    Tight,
    /// Zu langsam oder Aussetzer: sparsam, ohne Plattenspieler.
    Slow,
    /// Nur bei manueller Messung mit Sparmaßnahmen: der Rechner hätte Luft für die volle Optik.
    Room,
}

/// Empfehlung samt Messung; `lite`/`platters` sind die vorgeschlagenen Einstellungen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Advice {
    pub verdict: Verdict,
    pub measured: Measured,
    pub lite: Lite,
    pub platters: bool,
}

/// Leichte Sparmaßnahmen für knappe Rechner: kaum sichtbar, aber spürbar weniger Arbeit.
pub const TIGHT: Lite = Lite { frame_limit: true, no_bands: true, flat: true, coarse_waves: true, ..Lite::NONE };

/// Empfehlung aus der Messung und den aktuellen Einstellungen. `None` = nichts vorzuschlagen (schon so eingestellt
/// bzw. schnell genug); bei `manual` kommt auch „schnell genug“ zurück, damit die Messung eine Antwort hat.
pub fn advise(m: Measured, lite: Lite, platters: bool, manual: bool) -> Option<Advice> {
    let full = lite == Lite::NONE && platters;
    let verdict = if m.dropouts > 0 || m.median_ms > SLOW_MS {
        Verdict::Slow
    } else if m.median_ms > FAST_MS {
        Verdict::Tight
    } else if manual && !full && m.median_ms < ROOM_MS {
        Verdict::Room
    } else {
        Verdict::Fast
    };
    let (want, want_platters) = match verdict {
        Verdict::Slow => (Lite::ALL, false),
        // Knapp: zu den eigenen Haken die leichten Maßnahmen dazu
        Verdict::Tight => (union(lite, TIGHT), platters),
        Verdict::Room => (Lite::NONE, true),
        Verdict::Fast => (lite, platters),
    };
    let nothing_new = match verdict {
        Verdict::Slow | Verdict::Tight => union(lite, want) == lite && (platters == want_platters || !platters),
        _ => false,
    };
    if verdict == Verdict::Fast && !manual {
        return None;
    }
    if nothing_new && !manual {
        return None;
    }
    Some(Advice { verdict, measured: m, lite: want, platters: want_platters })
}

/// Beide Sätze von Sparmaßnahmen zusammen.
fn union(a: Lite, b: Lite) -> Lite {
    Lite {
        shapes: a.shapes || b.shapes,
        no_feathering: a.no_feathering || b.no_feathering,
        flat: a.flat || b.flat,
        coarse_waves: a.coarse_waves || b.coarse_waves,
        simple_pads: a.simple_pads || b.simple_pads,
        no_bands: a.no_bands || b.no_bands,
        frame_limit: a.frame_limit || b.frame_limit,
        no_waves: a.no_waves || b.no_waves,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(ms: f32, dropouts: u32) -> Measured {
        Measured { median_ms: ms, fps: 30.0, dropouts }
    }

    #[test]
    fn fast_computers_hear_nothing() {
        assert_eq!(advise(m(2.0, 0), Lite::NONE, true, false), None);
        assert_eq!(advise(m(2.0, 0), Lite::NONE, true, true).map(|a| a.verdict), Some(Verdict::Fast));
    }

    #[test]
    fn tight_and_slow_get_advice() {
        // MacBook 2008 mit voller Optik
        let a = advise(m(16.0, 0), Lite::NONE, true, false).unwrap();
        assert_eq!(a.verdict, Verdict::Tight);
        assert_eq!(a.lite, TIGHT);
        assert!(a.platters);
        // Netbook
        let a = advise(m(127.0, 0), Lite::NONE, true, false).unwrap();
        assert_eq!((a.verdict, a.lite, a.platters), (Verdict::Slow, Lite::ALL, false));
        // Aussetzer zählen wie langsam
        assert_eq!(advise(m(8.0, 2), Lite::NONE, true, false).unwrap().verdict, Verdict::Slow);
    }

    #[test]
    fn nothing_new_when_already_set() {
        assert_eq!(advise(m(40.0, 0), Lite::ALL, false, false), None);
        assert_eq!(advise(m(16.0, 0), TIGHT, true, false), None);
    }

    #[test]
    fn manual_measurement_finds_room() {
        assert_eq!(advise(m(3.0, 0), Lite::ALL, false, true).unwrap().verdict, Verdict::Room);
        // Automatisch wird das nie vorgeschlagen
        assert_eq!(advise(m(3.0, 0), Lite::ALL, false, false), None);
    }

    #[test]
    fn probe_collects_and_finishes() {
        let mut p = Probe::default();
        for _ in 0..1000 {
            p.frame(0.016, 0.05);
        }
        assert!(p.done());
        let r = p.result();
        assert!((r.median_ms - 16.0).abs() < 0.01);
        assert!((r.fps - 20.0).abs() < 0.1);
    }
}
