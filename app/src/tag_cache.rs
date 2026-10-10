// SPDX-License-Identifier: GPL-3.0-or-later
//! Gemerkte Tags, damit Suche und Titelliste große Sammlungen (etwa auf Netzlaufwerken) nicht jedes Mal
//! Datei für Datei neu lesen müssen.
//!
//! Keine Bibliothek: Die Titel kommen weiter live aus den Ordnern. Hier steht nur „Datei → Tags“ in
//! `tags.json` neben den Einstellungen, gruppiert nach Ordnern (der lange Pfad steht so nur einmal drin).
//! Jeder Eintrag trägt Größe und Änderungszeit der Datei. Gemerkte Tags werden sofort gezeigt und im
//! Hintergrund geprüft (`Library::check_meta`); hat sich die Datei geändert, werden sie neu gelesen.
//! Einträge, die [`EXPIRE_DAYS`] nicht gebraucht wurden, fallen beim Laden heraus.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fw_library::{Stamp, TrackMeta};
use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

/// So lange bleibt ein Eintrag, der nicht gebraucht wird.
pub const EXPIRE_DAYS: u64 = 90;
const DAY_SECS: u64 = 24 * 60 * 60;
/// Höchstens so oft wird gespeichert (beim ersten Durchsuchen einer großen Sammlung kommen ständig
/// neue Einträge, die Datei hat dann einige MB).
const SAVE_EVERY_SECS: u64 = 30;
const FORMAT: &str = "FadeWerk-DJ-Tags";

/// Ein Eintrag mit kurzen Feldnamen; leere Felder werden nicht gespeichert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    #[serde(rename = "s")]
    size: u64,
    /// Änderungszeit in Millisekunden seit 1970.
    #[serde(rename = "m")]
    modified: u64,
    /// Zuletzt gebraucht, Sekunden seit 1970.
    #[serde(rename = "u")]
    used: u64,
    #[serde(rename = "t", default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(rename = "a", default, skip_serializing_if = "String::is_empty")]
    artist: String,
    #[serde(rename = "l", default, skip_serializing_if = "String::is_empty")]
    album: String,
    #[serde(rename = "g", default, skip_serializing_if = "String::is_empty")]
    genre: String,
    #[serde(rename = "y", default, skip_serializing_if = "String::is_empty")]
    year: String,
    #[serde(rename = "c", default, skip_serializing_if = "String::is_empty")]
    comment: String,
    #[serde(rename = "d", default)]
    duration: f64,
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    bpm: Option<f32>,
}

impl Entry {
    fn meta(&self) -> TrackMeta {
        TrackMeta {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            genre: self.genre.clone(),
            year: self.year.clone(),
            comment: self.comment.clone(),
            duration_secs: self.duration,
            bpm: self.bpm,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct File<E> {
    format: String,
    version: u32,
    /// Ordner → Dateiname → Eintrag.
    dirs: BTreeMap<PathBuf, BTreeMap<String, E>>,
}

#[derive(Default)]
pub struct TagCache {
    entries: HashMap<PathBuf, Entry>,
    dirty: bool,
    saved_at: u64,
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("tags.json"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl TagCache {
    pub fn load() -> Self {
        let Some(p) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        match Self::from_json(&text, now_secs()) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Tag-Cache in {} unlesbar, fange neu an: {e}", p.display());
                Self::default()
            }
        }
    }

    fn from_json(text: &str, now: u64) -> Result<Self, String> {
        let file: File<Entry> = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!("unbekanntes Format „{}“", file.format));
        }
        let mut c = Self { saved_at: now, ..Default::default() };
        let cutoff = now.saturating_sub(EXPIRE_DAYS * DAY_SECS);
        for (dir, files) in file.dirs {
            for (name, e) in files {
                // Unsinn aus einer beschädigten Datei verwerfen
                if e.used < cutoff || !e.duration.is_finite() || e.duration < 0.0 || e.bpm.is_some_and(|b| !b.is_finite()) {
                    c.dirty = true;
                    continue;
                }
                c.entries.insert(dir.join(name), e);
            }
        }
        Ok(c)
    }

    fn to_json(&self) -> Result<String, String> {
        let mut dirs: BTreeMap<PathBuf, BTreeMap<String, &Entry>> = BTreeMap::new();
        for (p, e) in &self.entries {
            let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else { continue };
            dirs.entry(dir.to_path_buf()).or_default().insert(name.to_string_lossy().into_owned(), e);
        }
        serde_json::to_string(&File { format: FORMAT.into(), version: 1, dirs }).map_err(|e| e.to_string())
    }

    /// Speichert, wenn sich etwas geändert hat und das letzte Speichern [`SAVE_EVERY_SECS`] her ist
    /// (mit `now = true` sofort, etwa beim Beenden).
    pub fn save_if_dirty(&mut self, now: bool) {
        let t = now_secs();
        if !self.dirty || (!now && t.saturating_sub(self.saved_at) < SAVE_EVERY_SECS) {
            return;
        }
        let Some(p) = path() else { return };
        self.saved_at = t;
        match self.to_json().and_then(|json| write_atomic(&p, &json).map_err(|e| e.to_string())) {
            Ok(()) => self.dirty = false,
            Err(e) => log::warn!("Tag-Cache nicht gespeichert ({}): {e}", p.display()),
        }
    }

    /// Gemerkte Tags samt Größe und Änderungszeit, ungeprüft (geprüft wird im Hintergrund).
    pub fn get(&mut self, file: &Path) -> Option<(TrackMeta, Stamp)> {
        let e = self.entries.get_mut(file)?;
        // „Zuletzt gebraucht“ höchstens einmal am Tag auffrischen, sonst wäre ständig zu speichern
        let now = now_secs();
        if now.saturating_sub(e.used) > DAY_SECS {
            e.used = now;
            self.dirty = true;
        }
        Some((e.meta(), (e.size, e.modified)))
    }

    pub fn insert(&mut self, file: &Path, meta: &TrackMeta, (size, modified): Stamp) {
        let e = Entry {
            size,
            modified,
            used: now_secs(),
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            album: meta.album.clone(),
            genre: meta.genre.clone(),
            year: meta.year.clone(),
            comment: meta.comment.clone(),
            duration: meta.duration_secs,
            bpm: meta.bpm,
        };
        let same = self.entries.get(file).is_some_and(|old| Entry { used: old.used, ..e.clone() } == *old);
        if !same {
            self.entries.insert(file.to_path_buf(), e);
            self.dirty = true;
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str) -> TrackMeta {
        TrackMeta { title: title.into(), artist: "Band".into(), duration_secs: 200.0, bpm: Some(124.0), ..Default::default() }
    }

    #[test]
    fn round_trip_groups_by_folder() {
        let mut c = TagCache::default();
        c.insert(Path::new("/net/Musik/Album/01 Eins.mp3"), &meta("Eins"), (1000, 5));
        c.insert(Path::new("/net/Musik/Album/02 Zwei.mp3"), &meta("Zwei"), (2000, 6));
        let json = c.to_json().unwrap();
        // Der Ordner steht nur einmal drin, leere Felder gar nicht
        assert_eq!(json.matches("/net/Musik/Album").count(), 1);
        assert!(!json.contains("\"g\""));
        let mut back = TagCache::from_json(&json, now_secs()).unwrap();
        assert_eq!(back.len(), 2);
        let (m, stamp) = back.get(Path::new("/net/Musik/Album/02 Zwei.mp3")).unwrap();
        assert_eq!(m, meta("Zwei"));
        assert_eq!(stamp, (2000, 6));
    }

    #[test]
    fn old_and_broken_entries_are_dropped() {
        let mut c = TagCache::default();
        c.insert(Path::new("/a/alt.mp3"), &meta("Alt"), (1, 1));
        c.insert(Path::new("/a/neu.mp3"), &meta("Neu"), (1, 1));
        c.entries.get_mut(Path::new("/a/alt.mp3")).unwrap().used = 0;
        let back = TagCache::from_json(&c.to_json().unwrap(), now_secs()).unwrap();
        assert_eq!(back.len(), 1);
        assert!(back.dirty);
        assert!(TagCache::from_json("{kaputt", 0).is_err());
        assert!(TagCache::from_json(r#"{"format":"anders","version":1,"dirs":{}}"#, 0).is_err());
    }

    #[test]
    fn unchanged_insert_keeps_cache_clean() {
        let mut c = TagCache::default();
        c.insert(Path::new("/a/x.mp3"), &meta("X"), (1, 1));
        c.dirty = false;
        c.insert(Path::new("/a/x.mp3"), &meta("X"), (1, 1));
        assert!(!c.dirty);
        c.insert(Path::new("/a/x.mp3"), &meta("X"), (1, 2));
        assert!(c.dirty);
    }
}
