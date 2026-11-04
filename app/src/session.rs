// SPDX-License-Identifier: GPL-3.0-or-later
//! Sitzung: Playlists A und B überstehen Beenden und Abstürze.
//!
//! Liegt neben den Einstellungen als `playlists.json`. Die App speichert beim Beenden und zusätzlich
//! alle paar Sekunden, sobald sich etwas geändert hat.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fadewerk::TrackFade;
use crate::playlist::Playlist;
use crate::settings::{config_dir, write_atomic};

#[derive(Serialize, Deserialize, Default)]
struct SavedList {
    entries: Vec<PathBuf>,
    current: Option<usize>,
    /// FadeWerk-Bereiche je Eintrag (fehlt in älteren Dateien).
    #[serde(default)]
    fades: Vec<TrackFade>,
}

#[derive(Serialize, Deserialize, Default)]
struct Session {
    playlists: [SavedList; 2],
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("playlists.json"))
}

/// Lädt die gespeicherten Playlists. Fehlende Dateien bleiben zunächst drin: Die App sucht sie im
/// Hintergrund (`relocate`), vielleicht wurden sie nur verschoben.
pub fn load() -> [Playlist; 2] {
    let Some(p) = path() else { return Default::default() };
    let Ok(text) = std::fs::read_to_string(&p) else { return Default::default() };
    match restore(&text, |_| true) {
        Ok((lists, _)) => lists,
        Err(e) => {
            log::warn!("Playlists in {} unlesbar: {e}", p.display());
            Default::default()
        }
    }
}

/// Playlists aus dem gespeicherten Text; `exists` prüft, ob es eine Datei noch gibt.
/// Ein geladener Titel hinter dem Ende der Liste (kaputte Datei) gilt als nicht geladen.
fn restore(text: &str, exists: impl Fn(&std::path::Path) -> bool) -> Result<([Playlist; 2], usize), String> {
    let session: Session = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut missing = 0;
    let lists = session.playlists.map(|saved| {
        let current = saved.current.filter(|&c| c < saved.entries.len());
        let mut fades: Vec<TrackFade> = saved.fades.into_iter().map(TrackFade::sanitized).collect();
        fades.truncate(saved.entries.len());
        let mut pl = Playlist { entries: saved.entries, current, fades };
        // Rückwärts entfernen, damit `remove` den aktuellen Titel richtig mitverschiebt
        for i in (0..pl.entries.len()).rev() {
            if !exists(&pl.entries[i]) {
                pl.remove(i);
                missing += 1;
            }
        }
        pl
    });
    Ok((lists, missing))
}

pub fn save(lists: &[Playlist; 2]) {
    let Some(p) = path() else { return };
    let session = Session {
        playlists: [0, 1].map(|d| SavedList { entries: lists[d].entries.clone(), current: lists[d].current, fades: lists[d].fades.clone() }),
    };
    match serde_json::to_string_pretty(&session) {
        Ok(json) => {
            if let Err(e) = write_atomic(&p, &json) {
                log::warn!("Playlists nicht gespeichert ({}): {e}", p.display());
            }
        }
        Err(e) => log::warn!("Playlists nicht gespeichert: {e}"),
    }
}

/// Stelle eines geladenen Titels, damit die Player nach einem Absturz dort weitermachen.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Place {
    pub path: PathBuf,
    pub secs: f64,
    #[serde(default)]
    pub pitch_pct: f32,
}

impl Place {
    fn sanitized(self) -> Option<Self> {
        (self.secs.is_finite() && self.secs >= 0.0 && self.pitch_pct.is_finite()).then(|| Place { pitch_pct: self.pitch_pct.clamp(-100.0, 100.0), ..self })
    }
}

/// `player.json`: liegt nur da, solange die App läuft. Beim normalen Beenden wird die Datei gelöscht;
/// findet der nächste Start sie, endete die App unerwartet.
fn places_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("player.json"))
}

/// Stellen der Player vom letzten Lauf, wenn er unerwartet endete.
pub fn load_places() -> [Option<Place>; 2] {
    let Some(text) = places_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return [None, None] };
    parse_places(&text)
}

fn parse_places(text: &str) -> [Option<Place>; 2] {
    match serde_json::from_str::<[Option<Place>; 2]>(text) {
        Ok(places) => places.map(|p| p.and_then(Place::sanitized)),
        Err(e) => {
            log::warn!("player.json unlesbar: {e}");
            [None, None]
        }
    }
}

pub fn save_places(places: &[Option<Place>; 2]) {
    let Some(p) = places_path() else { return };
    match serde_json::to_string(places) {
        Ok(json) => {
            if let Err(e) = write_atomic(&p, &json) {
                log::warn!("Stellen der Player nicht gespeichert ({}): {e}", p.display());
            }
        }
        Err(e) => log::warn!("Stellen der Player nicht gespeichert: {e}"),
    }
}

/// Beim normalen Beenden: es gibt nichts wiederherzustellen.
pub fn clear_places() {
    if let Some(p) = places_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Fingerabdruck der Playlists, um Änderungen billig zu erkennen.
pub fn fingerprint(lists: &[Playlist; 2]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pl in lists {
        pl.entries.hash(&mut h);
        pl.current.hash(&mut h);
        for f in &pl.fades {
            f.hash_bits().hash(&mut h);
        }
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_session_files_are_handled() {
        for text in ["", "{", "[]", "null", r#"{"playlists":"kaputt"}"#, r#"{"playlists":[{"entries":[1,2]},{}]}"#] {
            assert!(restore(text, |_| true).is_err(), "{text:?} sollte ein Fehler sein");
        }
        // Geladener Titel hinter dem Listenende, fehlende Datei
        let text = r#"{"playlists":[{"entries":["/a.mp3","/b.mp3"],"current":7},{"entries":["/weg.mp3","/c.mp3"],"current":1}]}"#;
        let ([a, b], missing) = restore(text, |p| p != std::path::Path::new("/weg.mp3")).unwrap();
        assert_eq!((a.entries.len(), a.current), (2, None));
        assert_eq!((b.entries.len(), b.current), (1, Some(0)), "rückt mit dem entfernten Eintrag nach");
        assert_eq!(missing, 1);
    }

    #[test]
    fn places_survive_broken_values() {
        let text = r#"[{"path":"/a.mp3","secs":151.5,"pitch_pct":2.5},{"path":"/b.mp3","secs":-1}]"#;
        let [a, b] = parse_places(text);
        assert_eq!(a, Some(Place { path: "/a.mp3".into(), secs: 151.5, pitch_pct: 2.5 }));
        assert_eq!(b, None);
        for text in ["", "{", "[1,2]", "null"] {
            assert_eq!(parse_places(text), [None, None], "{text:?}");
        }
    }

    #[test]
    fn fades_stay_with_their_entries() {
        // Die fehlende Datei nimmt ihren Bereich mit, kaputte Werte fallen weg
        let text = r#"{"playlists":[{"entries":["/weg.mp3","/a.mp3"],"current":null,
            "fades":[{"fade_in":{"start":1,"end":2}},{"fade_out":{"start":100,"end":110}}]},
            {"entries":["/b.mp3"],"current":null,"fades":[{"fade_in":{"start":-5,"end":2}},{},{}]}]}"#;
        let ([a, b], _) = restore(text, |p| p != std::path::Path::new("/weg.mp3")).unwrap();
        assert_eq!(a.entries.len(), 1);
        assert_eq!(a.fade(0).fade_out.map(|r| r.len()), Some(10.0));
        assert_eq!(a.fade(0).fade_in, None);
        assert!(!b.has_fades());
        assert_eq!(b.fades.len(), 1);
    }
}
