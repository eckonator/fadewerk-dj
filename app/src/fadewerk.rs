// SPDX-License-Identifier: GPL-3.0-or-later
//! FadeWerk: Ein- und Ausblendbereich je Playlist-Eintrag.
//!
//! Ein Bereich ist eine Stelle im Titel, nicht nur eine Dauer: Der Titel beginnt am Anfang seines
//! Einblendbereichs (Intro davor wird übersprungen) und endet am Ende seines Ausblendbereichs
//! (Outro dahinter wird abgeschnitten). Ein Bereich der Länge 0 ist ein harter Schnitt.

use serde::{Deserialize, Serialize};

/// Längster Titel, den ein Bereich sinnvoll beschreiben kann (10 Stunden, schützt vor kaputten Dateien).
const MAX_SECS: f32 = 36_000.0;

/// Zeitbereich im Titel in Sekunden, `start <= end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub start: f32,
    pub end: f32,
}

impl Region {
    /// Bereich zwischen zwei Zeitpunkten, egal in welcher Reihenfolge.
    pub fn between(a: f32, b: f32) -> Self {
        Self { start: a.min(b), end: a.max(b) }
    }

    pub fn len(&self) -> f32 {
        self.end - self.start
    }

    fn sane(self) -> Option<Self> {
        let ok = self.start.is_finite() && self.end.is_finite() && self.start >= 0.0 && self.end <= MAX_SECS;
        ok.then(|| Self::between(self.start, self.end))
    }
}

/// Fade-Einstellung eines Eintrags. `None` = nicht gesetzt, dann gilt die halbe Überblendzeit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackFade {
    #[serde(default)]
    pub fade_in: Option<Region>,
    #[serde(default)]
    pub fade_out: Option<Region>,
}

impl TrackFade {
    pub fn is_empty(&self) -> bool {
        self.fade_in.is_none() && self.fade_out.is_none()
    }

    /// Unsinnige Werte (aus einer beschädigten Datei) verwerfen.
    pub fn sanitized(self) -> Self {
        Self { fade_in: self.fade_in.and_then(Region::sane), fade_out: self.fade_out.and_then(Region::sane) }
    }

    /// Für den Fingerabdruck der Sitzung (f32 ist nicht `Hash`).
    pub fn hash_bits(&self) -> [u32; 4] {
        let r = |r: Option<Region>| r.map_or([u32::MAX; 2], |r| [r.start.to_bits(), r.end.to_bits()]);
        let (a, b) = (r(self.fade_in), r(self.fade_out));
        [a[0], a[1], b[0], b[1]]
    }
}

/// Ablauf der nächsten Überblendung vom laufenden auf den nächsten Titel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// Stelle im alten Titel (Sekunden), an der er zu Ende ist.
    pub end: f64,
    /// Länge der ganzen Überblendung in Echtzeit.
    pub lead: f32,
    /// Zweiphasig nach FadeWerk-Bereichen, `None` = normale Überblendung über die Überblendzeit.
    pub split: Option<SplitPlan>,
}

/// Zweiphasige Überblendung: erst blendet der neue Titel `in_secs` lang ein (der alte bleibt voll), dann
/// blendet der alte zwischen `out_start` und `out_end` (Stellen im alten Titel) aus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitPlan {
    pub in_secs: f32,
    pub out_start: f64,
    pub out_end: f64,
    /// Dauer von Phase 2 in Echtzeit.
    pub out_secs: f32,
}

/// Plant die Überblendung. `old` = Bereiche des laufenden Titels (Länge `old_secs`), `new` = die des nächsten;
/// `old_rate`/`new_rate` = Abspieltempo (1 = ohne Pitch). Ohne Ausblendbereich beim alten und Einblendbereich
/// beim neuen Titel bleibt alles wie bisher; ist nur eine Seite gesetzt, gilt für die andere die halbe Überblendzeit.
pub fn plan(old: TrackFade, old_secs: f64, new: TrackFade, fade_secs: f32, old_rate: f32, new_rate: f32) -> Transition {
    let (old_rate, new_rate) = (old_rate.max(0.05), new_rate.max(0.05));
    if old.fade_out.is_none() && new.fade_in.is_none() {
        return Transition { end: old_secs, lead: fade_secs, split: None };
    }
    let half = fade_secs * 0.5;
    let dur = old_secs.max(0.0);
    let (out_start, out_end) = match old.fade_out {
        Some(r) => ((r.start as f64).min(dur), (r.end as f64).min(dur)),
        None => ((dur - half as f64).max(0.0), dur),
    };
    let in_secs = new.fade_in.map_or(half, |r| r.len()) / new_rate;
    let out_secs = (out_end - out_start) as f32 / old_rate;
    Transition { end: out_end, lead: in_secs + out_secs, split: Some(SplitPlan { in_secs, out_start, out_end, out_secs }) }
}

/// Sekunden mit einer Nachkommastelle, im Deutschen mit Komma.
pub fn fmt_secs(secs: f32) -> String {
    let s = format!("{:.1}", secs.max(0.0));
    if crate::i18n::lang() == crate::i18n::Lang::De { s.replace('.', ",") } else { s }
}

/// Zeitpunkt im Titel als m:ss,d.
pub fn fmt_point(secs: f32) -> String {
    let s = secs.max(0.0);
    let m = (s / 60.0).floor();
    let rest = s - m * 60.0;
    let t = format!("{}:{:04.1}", m as u32, rest);
    if crate::i18n::lang() == crate::i18n::Lang::De { t.replace('.', ",") } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_values_are_dropped() {
        let f = TrackFade {
            fade_in: Some(Region { start: f32::NAN, end: 3.0 }),
            fade_out: Some(Region { start: 200.0, end: 190.0 }),
        }
        .sanitized();
        assert_eq!(f.fade_in, None);
        assert_eq!(f.fade_out, Some(Region { start: 190.0, end: 200.0 }));
        assert_eq!(TrackFade { fade_in: Some(Region { start: -1.0, end: 2.0 }), fade_out: None }.sanitized(), TrackFade::default());
    }

    #[test]
    fn plan_uses_ranges_and_half_fade_time() {
        let r = |start, end| Some(Region { start, end });
        // Ohne Bereiche wie bisher
        let t = plan(TrackFade::default(), 200.0, TrackFade::default(), 8.0, 1.0, 1.0);
        assert_eq!(t, Transition { end: 200.0, lead: 8.0, split: None });
        // Beide gesetzt: Einblenden des neuen, dann Ausblenden des alten bis zum Ende seines Bereichs
        let old = TrackFade { fade_in: None, fade_out: r(180.0, 186.0) };
        let new = TrackFade { fade_in: r(2.0, 5.0), fade_out: None };
        let t = plan(old, 200.0, new, 8.0, 1.0, 1.0);
        assert_eq!(t.end, 186.0);
        assert_eq!(t.lead, 9.0);
        assert_eq!(t.split.map(|s| (s.in_secs, s.out_start)), Some((3.0, 180.0)));
        // Nur der neue hat einen Bereich: der alte blendet in der halben Überblendzeit am Titelende aus
        let t = plan(TrackFade::default(), 200.0, new, 8.0, 1.0, 1.0);
        assert_eq!(t.split.map(|s| (s.out_start, s.out_end, s.out_secs)), Some((196.0, 200.0, 4.0)));
        // Harter Schnitt und Pitch: +25 % macht die Bereiche in Echtzeit kürzer
        let hard = TrackFade { fade_in: None, fade_out: r(150.0, 150.0) };
        let t = plan(hard, 200.0, new, 8.0, 1.25, 1.5);
        assert_eq!((t.end, t.lead), (150.0, 2.0));
    }

    #[test]
    fn points_are_formatted() {
        assert_eq!(fmt_point(0.0).replace(',', "."), "0:00.0");
        assert_eq!(fmt_point(75.3).replace(',', "."), "1:15.3");
    }
}
