// SPDX-License-Identifier: GPL-3.0-or-later
//! Gemerkte BPM aus der Tempo-Erkennung, damit der Browser sie gleich zeigt und nichts doppelt analysiert wird.
//!
//! Keine Bibliothek: nur eine Liste „Datei → BPM“ in `bpm.json` neben den Einstellungen. Jeder Eintrag
//! trägt Größe und Änderungszeit der Datei; ändert sich die Datei (etwa neue Tags), gilt er nicht mehr.
//!
//! Wird ein Ordner umbenannt oder verschoben, stimmt der Pfad nicht mehr. Dann findet [`BpmCache::resolve`]
//! den Eintrag über Dateiname und Größe wieder und schreibt ihn auf den neuen Pfad um. Einträge, deren
//! Datei es länger als [`EXPIRE_DAYS`] nicht mehr gibt, räumt [`BpmCache::start_check`] weg.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

/// So lange bleibt ein Eintrag, dessen Datei nicht zu finden ist (etwa auf einer gerade nicht
/// angeschlossenen Platte), bevor er gelöscht wird.
pub const EXPIRE_DAYS: u64 = 90;
const DAY_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    bpm: f32,
    size: u64,
    /// Änderungszeit in Millisekunden seit 1970.
    modified: u64,
    /// Zuletzt gesehen (Datei vorhanden), Sekunden seit 1970. Fehlt in älteren Dateien: dann „jetzt“.
    #[serde(default = "now_secs")]
    seen: u64,
}

/// Zweiter Schlüssel: Dateiname und Größe. Beides bleibt beim Umbenennen oder Verschieben eines
/// Ordners und beim Kopieren auf eine andere Platte gleich, anders als Pfad und Änderungszeit.
type Moved = (String, u64);

/// Ergebnis von [`BpmCache::resolve`].
#[derive(Debug, PartialEq)]
pub enum Resolved {
    /// Gemerkter Wert gilt (auch nach Umzug der Datei).
    Bpm(f32),
    /// Es gab einen Eintrag, aber die Datei hat sich geändert: Wert verwerfen.
    Stale,
    /// Nichts gemerkt.
    Unknown,
}

#[derive(Default)]
pub struct BpmCache {
    entries: HashMap<PathBuf, Entry>,
    by_name: HashMap<Moved, PathBuf>,
    /// Seit dem letzten Speichern geändert.
    dirty: bool,
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("bpm.json"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

use fw_library::{Stamp, file_stamp as stamp};

fn moved_key(file: &Path, size: u64) -> Option<Moved> {
    Some((file.file_name()?.to_string_lossy().into_owned(), size))
}

impl BpmCache {
    fn from_entries(mut entries: HashMap<PathBuf, Entry>) -> Self {
        // Unsinnige Werte aus einer beschädigten Datei verwerfen (sie würden Sync und Anzeige verwirren)
        entries.retain(|_, e| e.bpm.is_finite() && (20.0..=400.0).contains(&e.bpm));
        let mut c = Self { entries, ..Default::default() };
        let index: Vec<(Moved, PathBuf)> = c.entries.iter().filter_map(|(p, e)| Some((moved_key(p, e.size)?, p.clone()))).collect();
        c.by_name.extend(index);
        c
    }

    pub fn load() -> Self {
        let Some(p) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        match serde_json::from_str(&text) {
            Ok(entries) => Self::from_entries(entries),
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

    /// Alle gemerkten Werte unter ihrem gespeicherten Pfad (ungeprüft; geprüft wird mit [`Self::resolve`]).
    pub fn all(&self) -> impl Iterator<Item = (&PathBuf, f32)> {
        self.entries.iter().map(|(p, e)| (p, e.bpm))
    }

    fn put(&mut self, file: PathBuf, e: Entry) {
        if let Some(k) = moved_key(&file, e.size) {
            self.by_name.insert(k, file.clone());
        }
        self.entries.insert(file, e);
        self.dirty = true;
    }

    fn remove(&mut self, file: &Path) {
        if let Some(e) = self.entries.remove(file) {
            if let Some(k) = moved_key(file, e.size)
                && self.by_name.get(&k).is_some_and(|p| p == file)
            {
                self.by_name.remove(&k);
            }
            self.dirty = true;
        }
    }

    pub fn insert(&mut self, file: &Path, bpm: f32) {
        let Some((size, modified)) = stamp(file) else { return };
        let e = Entry { bpm, size, modified, seen: now_secs() };
        if self.entries.get(file).is_none_or(|old| (old.bpm, old.size, old.modified) != (bpm, size, modified)) {
            self.put(file.to_path_buf(), e);
        }
    }

    /// Gemerkte BPM für `file`. Steht sie unter dem Pfad, muss die Datei unverändert sein. Sonst wird über
    /// Dateiname und Größe gesucht: ein Treffer, dessen alte Datei es nicht mehr gibt, zieht auf den neuen
    /// Pfad um. Kostet einen `stat` der Datei.
    #[cfg(test)]
    fn resolve(&mut self, file: &Path) -> Resolved {
        self.resolve_stamped(file, stamp(file))
    }

    /// Wie [`Self::resolve`] mit schon bekannter Größe und Änderungszeit (aus dem Tag-Thread, damit die
    /// Oberfläche Dateien auf Netzlaufwerken nicht selbst abfragen muss).
    pub fn resolve_stamped(&mut self, file: &Path, stamp: Option<Stamp>) -> Resolved {
        let Some((size, modified)) = stamp else { return Resolved::Unknown };
        if let Some(e) = self.entries.get_mut(file) {
            if (e.size, e.modified) != (size, modified) {
                self.remove(file);
                return Resolved::Stale;
            }
            let bpm = e.bpm;
            // „Zuletzt gesehen“ höchstens einmal am Tag auffrischen, sonst wäre der Cache ständig zu speichern
            let now = now_secs();
            if now.saturating_sub(e.seen) > DAY_SECS {
                e.seen = now;
                self.dirty = true;
            }
            return Resolved::Bpm(bpm);
        }
        let Some(key) = moved_key(file, size) else { return Resolved::Unknown };
        let Some(old) = self.by_name.get(&key).cloned() else { return Resolved::Unknown };
        // Liegt die alte Datei noch da, ist es eine Kopie oder ein Namensvetter: dann nicht umziehen
        if old.exists() {
            return Resolved::Unknown;
        }
        let Some(e) = self.entries.get(&old).copied() else { return Resolved::Unknown };
        self.remove(&old);
        self.put(file.to_path_buf(), Entry { modified, seen: now_secs(), ..e });
        log::debug!("BPM-Cache: {} → {}", old.display(), file.display());
        Resolved::Bpm(e.bpm)
    }

    /// Prüft im Hintergrund, welche gemerkten Dateien es noch gibt. Das Ergebnis mit [`Self::apply_check`]
    /// übernehmen.
    pub fn start_check(&self) -> Receiver<Vec<(PathBuf, bool)>> {
        let paths: Vec<PathBuf> = self.entries.keys().cloned().collect();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("fw-bpm-check".into()).spawn(move || {
            let _ = tx.send(paths.into_iter().map(|p| {
                let exists = p.exists();
                (p, exists)
            }).collect());
        });
        if let Err(e) = spawned {
            log::warn!("BPM-Cache-Prüfung startet nicht: {e}");
        }
        rx
    }

    /// Vorhandene Dateien gelten als gesehen; fehlende fallen heraus, wenn sie länger als
    /// [`EXPIRE_DAYS`] nicht gesehen wurden. Liefert die Zahl der entfernten Einträge.
    pub fn apply_check(&mut self, results: Vec<(PathBuf, bool)>) -> usize {
        self.apply_check_at(results, now_secs())
    }

    fn apply_check_at(&mut self, results: Vec<(PathBuf, bool)>, now: u64) -> usize {
        let mut removed = 0;
        for (p, exists) in results {
            let Some(e) = self.entries.get_mut(&p) else { continue };
            if exists {
                if now.saturating_sub(e.seen) > DAY_SECS {
                    e.seen = now;
                    self.dirty = true;
                }
            } else if now.saturating_sub(e.seen) > EXPIRE_DAYS * DAY_SECS {
                self.remove(&p);
                removed += 1;
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonsense_bpm_values_are_dropped() {
        let json = r#"{"/a.mp3":{"bpm":128.0,"size":1,"modified":1},"/b.mp3":{"bpm":-5.0,"size":1,"modified":1},"/c.mp3":{"bpm":99999.0,"size":1,"modified":1}}"#;
        let entries: HashMap<PathBuf, Entry> = serde_json::from_str(json).unwrap();
        let c = BpmCache::from_entries(entries);
        assert_eq!(c.entries.len(), 1);
        assert!(c.entries.contains_key(Path::new("/a.mp3")));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw-bpm-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn entry_expires_when_the_file_changes() {
        let dir = temp_dir("change");
        let file = dir.join("a.mp3");
        std::fs::write(&file, b"abc").unwrap();
        let mut c = BpmCache::default();
        c.insert(&file, 124.0);
        assert_eq!(c.resolve(&file), Resolved::Bpm(124.0));
        std::fs::write(&file, b"abcdef").unwrap();
        assert_eq!(c.resolve(&file), Resolved::Stale);
        assert_eq!(c.all().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn value_follows_a_renamed_folder() {
        let dir = temp_dir("move");
        std::fs::create_dir_all(dir.join("Alt")).unwrap();
        let old = dir.join("Alt/titel.mp3");
        std::fs::write(&old, b"musik").unwrap();
        let mut c = BpmCache::default();
        c.insert(&old, 128.0);
        std::fs::rename(dir.join("Alt"), dir.join("Neu")).unwrap();
        let new = dir.join("Neu/titel.mp3");
        assert_eq!(c.resolve(&new), Resolved::Bpm(128.0));
        // Der Eintrag ist umgezogen, der alte Pfad ist weg
        assert_eq!(c.all().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec![new.clone()]);
        assert_eq!(c.resolve(&new), Resolved::Bpm(128.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_copy_does_not_steal_the_value() {
        let dir = temp_dir("copy");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("t.mp3"), b"musik").unwrap();
        std::fs::write(b.join("t.mp3"), b"musik").unwrap();
        let mut c = BpmCache::default();
        c.insert(&a.join("t.mp3"), 100.0);
        assert_eq!(c.resolve(&b.join("t.mp3")), Resolved::Unknown);
        assert_eq!(c.resolve(&a.join("t.mp3")), Resolved::Bpm(100.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_files_are_removed_only_after_the_grace_period() {
        let gone = PathBuf::from("/gibt/es/nicht.mp3");
        let mut c = BpmCache::from_entries(HashMap::from([(gone.clone(), Entry { bpm: 90.0, size: 1, modified: 1, seen: 1_000 })]));
        assert_eq!(c.apply_check_at(vec![(gone.clone(), false)], 1_000 + 10 * DAY_SECS), 0);
        assert_eq!(c.all().count(), 1);
        assert_eq!(c.apply_check_at(vec![(gone, false)], 1_000 + (EXPIRE_DAYS + 1) * DAY_SECS), 1);
        assert_eq!(c.all().count(), 0);
        assert!(c.by_name.is_empty());
    }
}
