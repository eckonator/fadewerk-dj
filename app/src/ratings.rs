// SPDX-License-Identifier: GPL-3.0-or-later
//! Eigene Bewertungen der Titel (1 bis 5 Sterne) in `bewertung.json` neben den Einstellungen.
//!
//! Anders als die Caches gehören die Werte dem Nutzer: Sie gelten auch, wenn sich die Datei ändert (etwa neue
//! Tags), und fallen nie von selbst weg. Eine Bewertung aus den Tags der Datei (von einem anderen Player) gilt,
//! solange hier nichts steht; 0 Sterne heißt „bewusst entfernt“ und überdeckt sie.
//!
//! Wird ein Ordner umbenannt oder verschoben, findet [`Ratings::follow`] den Eintrag über Dateiname und Größe
//! wieder, wenn es die alte Datei nicht mehr gibt (wie beim BPM-Cache).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fw_library::{Stamp, TrackMeta};
use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

const FILE: &str = "bewertung.json";
const FORMAT: &str = "FadeWerk-DJ-Bewertungen";
const VERSION: u32 = 1;
/// Höchste Bewertung.
pub const MAX_STARS: u8 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// 0 bis 5 Sterne; 0 = bewusst ohne Bewertung (überdeckt eine Bewertung im Tag).
    #[serde(rename = "s")]
    stars: u8,
    /// Dateigröße beim Bewerten, um eine umgezogene Datei wiederzufinden.
    #[serde(rename = "g", default)]
    size: u64,
    /// Bewertet am, Sekunden seit 1970.
    #[serde(rename = "t", default)]
    at: u64,
}

#[derive(Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    #[serde(default)]
    tracks: HashMap<PathBuf, Entry>,
}

type Moved = (String, u64);

#[derive(Default)]
pub struct Ratings {
    entries: HashMap<PathBuf, Entry>,
    by_name: HashMap<Moved, PathBuf>,
    dirty: bool,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn moved_key(file: &Path, size: u64) -> Option<Moved> {
    if size == 0 {
        return None;
    }
    Some((file.file_name()?.to_string_lossy().into_owned(), size))
}

impl Ratings {
    fn from_json(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!("unbekanntes Format „{}“", file.format));
        }
        let mut r = Self::default();
        for (p, e) in file.tracks.into_iter().filter(|(_, e)| e.stars <= MAX_STARS) {
            r.put(p, e);
        }
        r.dirty = false;
        Ok(r)
    }

    pub fn load() -> Self {
        let Some(p) = config_dir().map(|d| d.join(FILE)) else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        Self::from_json(&text).unwrap_or_else(|e| {
            log::warn!("Bewertungen in {} unlesbar: {e}", p.display());
            // Die Datei nicht überschreiben: Bewertungen sind Arbeit des Nutzers, nicht nachzurechnen
            let _ = std::fs::copy(&p, p.with_extension("json.kaputt"));
            Self::default()
        })
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(p) = config_dir().map(|d| d.join(FILE)) else { return };
        let file = File { format: FORMAT.into(), version: VERSION, tracks: self.entries.clone() };
        match serde_json::to_string(&file) {
            Ok(json) => match write_atomic(&p, &json) {
                Ok(()) => self.dirty = false,
                Err(e) => log::warn!("Bewertungen nicht gespeichert ({}): {e}", p.display()),
            },
            Err(e) => log::warn!("Bewertungen nicht gespeichert: {e}"),
        }
    }

    fn put(&mut self, file: PathBuf, e: Entry) {
        if let Some(k) = moved_key(&file, e.size) {
            self.by_name.insert(k, file.clone());
        }
        self.entries.insert(file, e);
        self.dirty = true;
    }

    /// Eigene Bewertung (0 = bewusst keine), `None` = hier nichts gemerkt.
    pub fn get(&self, file: &Path) -> Option<u8> {
        self.entries.get(file).map(|e| e.stars)
    }

    /// Sterne, die für den Titel gelten: eigene Bewertung, sonst die aus dem Tag, sonst 0.
    pub fn stars(&self, file: &Path, meta: Option<&TrackMeta>) -> u8 {
        self.get(file).or_else(|| meta.and_then(|m| m.rating)).unwrap_or(0).min(MAX_STARS)
    }

    /// Bewertet den Titel (0 = Bewertung entfernen). Die Größe kommt aus `stamp`, damit die Oberfläche
    /// Dateien auf Netzlaufwerken nicht selbst abfragen muss; ohne sie lässt sich ein Umzug nicht verfolgen.
    pub fn set(&mut self, file: &Path, stars: u8, stamp: Option<Stamp>) {
        let size = stamp.map(|(s, _)| s).or_else(|| self.entries.get(file).map(|e| e.size)).unwrap_or(0);
        self.put(file.to_path_buf(), Entry { stars: stars.min(MAX_STARS), size, at: now_secs() });
    }

    /// Titel mit frisch gelesenen Tags: Steht er hier nicht, aber eine gleichnamige, gleich große Datei,
    /// die es nicht mehr gibt, zieht deren Bewertung um. Liefert `true`, wenn sich etwas geändert hat.
    pub fn follow(&mut self, file: &Path, stamp: Option<Stamp>) -> bool {
        if self.entries.contains_key(file) {
            return false;
        }
        let Some(key) = stamp.and_then(|(size, _)| moved_key(file, size)) else { return false };
        let Some(old) = self.by_name.get(&key).cloned() else { return false };
        if old.exists() {
            return false;
        }
        let Some(e) = self.entries.remove(&old) else { return false };
        log::debug!("Bewertung: {} → {}", old.display(), file.display());
        self.put(file.to_path_buf(), e);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_rating_beats_the_tag_and_zero_hides_it() {
        let mut r = Ratings::default();
        let tagged = TrackMeta { rating: Some(4), ..Default::default() };
        let p = Path::new("/m/a.mp3");
        assert_eq!(r.stars(p, Some(&tagged)), 4);
        r.set(p, 2, Some((10, 1)));
        assert_eq!(r.stars(p, Some(&tagged)), 2);
        r.set(p, 0, None);
        assert_eq!(r.stars(p, Some(&tagged)), 0);
        assert_eq!(r.stars(Path::new("/m/b.mp3"), None), 0);
    }

    #[test]
    fn survives_a_round_trip_and_drops_nonsense() {
        let mut r = Ratings::default();
        r.set(Path::new("/m/a.mp3"), 5, Some((10, 1)));
        let json = serde_json::to_string(&File { format: FORMAT.into(), version: VERSION, tracks: r.entries.clone() }).unwrap();
        let back = Ratings::from_json(&json).unwrap();
        assert_eq!(back.get(Path::new("/m/a.mp3")), Some(5));
        let bad = r#"{"format":"FadeWerk-DJ-Bewertungen","version":1,"tracks":{"/x.mp3":{"s":9},"/y.mp3":{"s":3}}}"#;
        let r = Ratings::from_json(bad).unwrap();
        assert_eq!((r.get(Path::new("/x.mp3")), r.get(Path::new("/y.mp3"))), (None, Some(3)));
        assert!(Ratings::from_json("{}").is_err());
    }

    #[test]
    fn rating_follows_a_moved_file_but_not_a_copy() {
        let mut r = Ratings::default();
        r.set(Path::new("/gibt/es/nicht/titel.mp3"), 4, Some((1234, 1)));
        assert!(!r.follow(Path::new("/neu/titel.mp3"), Some((999, 1))), "andere Größe");
        assert!(r.follow(Path::new("/neu/titel.mp3"), Some((1234, 5))));
        assert_eq!(r.get(Path::new("/neu/titel.mp3")), Some(4));
        assert_eq!(r.get(Path::new("/gibt/es/nicht/titel.mp3")), None);
        // Gibt es die alte Datei noch, ist es eine Kopie: nicht umziehen
        let dir = std::env::temp_dir().join(format!("fw-ratings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("t.mp3");
        std::fs::write(&old, b"x").unwrap();
        r.set(&old, 3, Some((1, 1)));
        assert!(!r.follow(Path::new("/kopie/t.mp3"), Some((1, 1))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
