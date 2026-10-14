// SPDX-License-Identifier: GPL-3.0-or-later
//! Gemerkte BPM aus der Tempo-Erkennung, damit der Browser sie gleich zeigt und nichts doppelt analysiert wird.
//!
//! Keine Bibliothek: nur eine Liste „Datei → BPM“ in `bpm.json` neben den Einstellungen. Jeder Eintrag
//! trägt Größe und Änderungszeit der Datei; ändert sich die Datei (etwa neue Tags), gilt er nicht mehr.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    bpm: f32,
    size: u64,
    /// Änderungszeit in Millisekunden seit 1970.
    modified: u64,
}

#[derive(Default)]
pub struct BpmCache {
    entries: HashMap<PathBuf, Entry>,
    /// Seit dem letzten Speichern geändert.
    dirty: bool,
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("bpm.json"))
}

/// Größe und Änderungszeit der Datei, `None` wenn sie nicht lesbar ist.
fn stamp(file: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(file).ok()?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    Some((m.len(), modified))
}

impl BpmCache {
    pub fn load() -> Self {
        let Some(p) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        match serde_json::from_str(&text) {
            Ok(entries) => Self { entries, dirty: false },
            Err(e) => {
                log::warn!("BPM-Cache in {} unlesbar, fange neu an: {e}", p.display());
                Self::default()
            }
        }
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(p) = path() else { return };
        match serde_json::to_string(&self.entries) {
            Ok(json) => match write_atomic(&p, &json) {
                Ok(()) => self.dirty = false,
                Err(e) => log::warn!("BPM-Cache nicht gespeichert ({}): {e}", p.display()),
            },
            Err(e) => log::warn!("BPM-Cache nicht gespeichert: {e}"),
        }
    }

    /// Alle gemerkten Werte (ungeprüft; geprüft wird, sobald die Tags eines Titels gelesen werden).
    pub fn all(&self) -> impl Iterator<Item = (&PathBuf, f32)> {
        self.entries.iter().map(|(p, e)| (p, e.bpm))
    }

    pub fn insert(&mut self, file: &Path, bpm: f32) {
        let Some((size, modified)) = stamp(file) else { return };
        let e = Entry { bpm, size, modified };
        if self.entries.get(file) != Some(&e) {
            self.entries.insert(file.to_path_buf(), e);
            self.dirty = true;
        }
    }

    /// Gilt der Eintrag noch? Hat sich die Datei geändert oder gibt es sie nicht mehr, fällt er heraus
    /// und es kommt `false`. Ohne Eintrag `true`.
    pub fn validate(&mut self, file: &Path) -> bool {
        let Some(e) = self.entries.get(file) else { return true };
        if stamp(file) == Some((e.size, e.modified)) {
            return true;
        }
        self.entries.remove(file);
        self.dirty = true;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_expires_when_the_file_changes() {
        let file = std::env::temp_dir().join(format!("fw-bpm-cache-{}.mp3", std::process::id()));
        std::fs::write(&file, b"abc").unwrap();
        let mut c = BpmCache::default();
        c.insert(&file, 124.0);
        assert!(c.validate(&file));
        assert_eq!(c.all().count(), 1);
        std::fs::write(&file, b"abcdef").unwrap();
        assert!(!c.validate(&file));
        assert_eq!(c.all().count(), 0);
        std::fs::remove_file(&file).unwrap();
    }
}
