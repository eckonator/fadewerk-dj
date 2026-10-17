// SPDX-License-Identifier: GPL-3.0-or-later
//! Gemerkte Ergebnisse der Analyse (BPM in `bpm.json`, Tonart in `tonart.json`), damit der Browser sie
//! gleich zeigt und nichts doppelt analysiert wird.
//!
//! Keine Bibliothek: nur eine Liste „Datei → Wert“ neben den Einstellungen. Jeder Eintrag trägt Größe und
//! Änderungszeit der Datei; ändert sich die Datei (etwa neue Tags), gilt er nicht mehr.
//!
//! Wird ein Ordner umbenannt oder verschoben, stimmt der Pfad nicht mehr. Dann findet [`AnalysisCache::resolve`]
//! den Eintrag über Dateiname und Größe wieder und schreibt ihn auf den neuen Pfad um. Einträge, deren
//! Datei es länger als [`EXPIRE_DAYS`] nicht mehr gibt, räumt [`AnalysisCache::start_check`] weg.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

/// So lange bleibt ein Eintrag, dessen Datei nicht zu finden ist (etwa auf einer gerade nicht
/// angeschlossenen Platte), bevor er gelöscht wird.
pub const EXPIRE_DAYS: u64 = 90;
const DAY_SECS: u64 = 24 * 60 * 60;

/// Ein gemerkter Wert: legt Dateiname und Prüfung auf Unsinn fest.
pub trait CacheValue: Copy + PartialEq + Serialize + DeserializeOwned {
    /// Dateiname im Einstellungsordner.
    const FILE: &'static str;
    /// Für Meldungen im Log.
    const NAME: &'static str;
    /// Werte aus einer beschädigten Datei verwerfen.
    fn plausible(&self) -> bool;
}

/// Ermittelte BPM. Das Feld heißt in der Datei `bpm` (Format wie vor der Tonart, ältere Versionen lesen es).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bpm {
    pub bpm: f32,
}

impl CacheValue for Bpm {
    const FILE: &'static str = "bpm.json";
    const NAME: &'static str = "BPM-Cache";
    fn plausible(&self) -> bool {
        // Unsinnige Werte würden Sync und Anzeige verwirren
        self.bpm.is_finite() && (20.0..=400.0).contains(&self.bpm)
    }
}

/// Ermittelte Tonart als Camelot-Code (0 bis 23, siehe `Camelot::code`), [`Key::UNSURE`] = unsicher (nicht
/// erneut analysieren).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub key: u8,
}

impl Key {
    pub const UNSURE: u8 = 24;

    pub fn new(key: Option<fw_library::Camelot>) -> Self {
        Self { key: key.map_or(Self::UNSURE, |k| k.code()) }
    }

    pub fn camelot(self) -> Option<fw_library::Camelot> {
        fw_library::Camelot::from_code(self.key)
    }
}

impl CacheValue for Key {
    const FILE: &'static str = "tonart.json";
    const NAME: &'static str = "Tonart-Cache";
    fn plausible(&self) -> bool {
        self.key <= Self::UNSURE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Entry<V> {
    #[serde(flatten)]
    value: V,
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

/// Ergebnis von [`AnalysisCache::resolve`].
#[derive(Debug, PartialEq)]
pub enum Resolved<V> {
    /// Gemerkter Wert gilt (auch nach Umzug der Datei).
    Value(V),
    /// Es gab einen Eintrag, aber die Datei hat sich geändert: Wert verwerfen.
    Stale,
    /// Nichts gemerkt.
    Unknown,
}

pub struct AnalysisCache<V> {
    entries: HashMap<PathBuf, Entry<V>>,
    by_name: HashMap<Moved, PathBuf>,
    /// Seit dem letzten Speichern geändert.
    dirty: bool,
}

pub type BpmCache = AnalysisCache<Bpm>;
pub type KeyCache = AnalysisCache<Key>;

impl<V> Default for AnalysisCache<V> {
    fn default() -> Self {
        Self { entries: HashMap::new(), by_name: HashMap::new(), dirty: false }
    }
}

fn path<V: CacheValue>() -> Option<PathBuf> {
    config_dir().map(|d| d.join(V::FILE))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

use fw_library::{Stamp, file_stamp as stamp};

fn moved_key(file: &Path, size: u64) -> Option<Moved> {
    Some((file.file_name()?.to_string_lossy().into_owned(), size))
}

impl<V: CacheValue> AnalysisCache<V> {
    fn from_entries(mut entries: HashMap<PathBuf, Entry<V>>) -> Self {
        entries.retain(|_, e| e.value.plausible());
        let mut c = Self { entries, ..Default::default() };
        let index: Vec<(Moved, PathBuf)> = c.entries.iter().filter_map(|(p, e)| Some((moved_key(p, e.size)?, p.clone()))).collect();
        c.by_name.extend(index);
        c
    }

    pub fn load() -> Self {
        let Some(p) = path::<V>() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        match serde_json::from_str(&text) {
            Ok(entries) => Self::from_entries(entries),
            Err(e) => {
                log::warn!("{} in {} unlesbar, fange neu an: {e}", V::NAME, p.display());
                Self::default()
            }
        }
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(p) = path::<V>() else { return };
        match serde_json::to_string(&self.entries) {
            Ok(json) => match write_atomic(&p, &json) {
                Ok(()) => self.dirty = false,
                Err(e) => log::warn!("{} nicht gespeichert ({}): {e}", V::NAME, p.display()),
            },
            Err(e) => log::warn!("{} nicht gespeichert: {e}", V::NAME),
        }
    }

    /// Alle gemerkten Werte unter ihrem gespeicherten Pfad (ungeprüft; geprüft wird mit [`Self::resolve`]).
    pub fn all(&self) -> impl Iterator<Item = (&PathBuf, V)> {
        self.entries.iter().map(|(p, e)| (p, e.value))
    }

    fn put(&mut self, file: PathBuf, e: Entry<V>) {
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

    pub fn insert(&mut self, file: &Path, value: V) {
        let Some((size, modified)) = stamp(file) else { return };
        let e = Entry { value, size, modified, seen: now_secs() };
        if self.entries.get(file).is_none_or(|old| (old.value, old.size, old.modified) != (value, size, modified)) {
            self.put(file.to_path_buf(), e);
        }
    }

    /// Gemerkter Wert für `file`. Steht sie unter dem Pfad, muss die Datei unverändert sein. Sonst wird über
    /// Dateiname und Größe gesucht: ein Treffer, dessen alte Datei es nicht mehr gibt, zieht auf den neuen
    /// Pfad um. Kostet einen `stat` der Datei.
    #[cfg(test)]
    fn resolve(&mut self, file: &Path) -> Resolved<V> {
        self.resolve_stamped(file, stamp(file))
    }

    /// Wie [`Self::resolve`] mit schon bekannter Größe und Änderungszeit (aus dem Tag-Thread, damit die
    /// Oberfläche Dateien auf Netzlaufwerken nicht selbst abfragen muss).
    pub fn resolve_stamped(&mut self, file: &Path, stamp: Option<Stamp>) -> Resolved<V> {
        let Some((size, modified)) = stamp else { return Resolved::Unknown };
        if let Some(e) = self.entries.get_mut(file) {
            if (e.size, e.modified) != (size, modified) {
                self.remove(file);
                return Resolved::Stale;
            }
            let value = e.value;
            // „Zuletzt gesehen“ höchstens einmal am Tag auffrischen, sonst wäre der Cache ständig zu speichern
            let now = now_secs();
            if now.saturating_sub(e.seen) > DAY_SECS {
                e.seen = now;
                self.dirty = true;
            }
            return Resolved::Value(value);
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
        log::debug!("{}: {} → {}", V::NAME, old.display(), file.display());
        Resolved::Value(e.value)
    }

    /// Prüft im Hintergrund, welche gemerkten Dateien es noch gibt. Das Ergebnis mit [`Self::apply_check`]
    /// übernehmen.
    pub fn start_check(&self) -> Receiver<Vec<(PathBuf, bool)>> {
        let paths: Vec<PathBuf> = self.entries.keys().cloned().collect();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("fw-cache-check".into()).spawn(move || {
            let _ = tx.send(paths.into_iter().map(|p| {
                let exists = p.exists();
                (p, exists)
            }).collect());
        });
        if let Err(e) = spawned {
            log::warn!("Prüfung des {} startet nicht: {e}", V::NAME);
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
    fn key_cache_keeps_unsure_and_drops_nonsense() {
        let json = r#"{"/a.mp3":{"key":15,"size":1,"modified":1},"/b.mp3":{"key":24,"size":1,"modified":1},"/c.mp3":{"key":99,"size":1,"modified":1}}"#;
        let entries: HashMap<PathBuf, Entry<Key>> = serde_json::from_str(json).unwrap();
        let c = KeyCache::from_entries(entries);
        assert_eq!(c.entries.len(), 2);
        assert_eq!(c.entries[Path::new("/a.mp3")].value.camelot().map(|k| k.label()), Some("8B".into()));
        assert_eq!(c.entries[Path::new("/b.mp3")].value.camelot(), None, "unsicher");
    }

    #[test]
    fn bpm_file_format_is_unchanged() {
        let e = Entry { value: Bpm { bpm: 120.0 }, size: 2, modified: 3, seen: 4 };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"bpm":120.0,"size":2,"modified":3,"seen":4}"#);
    }

    #[test]
    fn nonsense_bpm_values_are_dropped() {
        let json = r#"{"/a.mp3":{"bpm":128.0,"size":1,"modified":1},"/b.mp3":{"bpm":-5.0,"size":1,"modified":1},"/c.mp3":{"bpm":99999.0,"size":1,"modified":1}}"#;
        let entries: HashMap<PathBuf, Entry<Bpm>> = serde_json::from_str(json).unwrap();
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
        c.insert(&file, Bpm { bpm: 124.0 });
        assert_eq!(c.resolve(&file), Resolved::Value(Bpm { bpm: 124.0 }));
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
        c.insert(&old, Bpm { bpm: 128.0 });
        std::fs::rename(dir.join("Alt"), dir.join("Neu")).unwrap();
        let new = dir.join("Neu/titel.mp3");
        assert_eq!(c.resolve(&new), Resolved::Value(Bpm { bpm: 128.0 }));
        // Der Eintrag ist umgezogen, der alte Pfad ist weg
        assert_eq!(c.all().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec![new.clone()]);
        assert_eq!(c.resolve(&new), Resolved::Value(Bpm { bpm: 128.0 }));
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
        c.insert(&a.join("t.mp3"), Bpm { bpm: 100.0 });
        assert_eq!(c.resolve(&b.join("t.mp3")), Resolved::Unknown);
        assert_eq!(c.resolve(&a.join("t.mp3")), Resolved::Value(Bpm { bpm: 100.0 }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_files_are_removed_only_after_the_grace_period() {
        let gone = PathBuf::from("/gibt/es/nicht.mp3");
        let mut c = BpmCache::from_entries(HashMap::from([(gone.clone(), Entry { value: Bpm { bpm: 90.0 }, size: 1, modified: 1, seen: 1_000 })]));
        assert_eq!(c.apply_check_at(vec![(gone.clone(), false)], 1_000 + 10 * DAY_SECS), 0);
        assert_eq!(c.all().count(), 1);
        assert_eq!(c.apply_check_at(vec![(gone, false)], 1_000 + (EXPIRE_DAYS + 1) * DAY_SECS), 1);
        assert_eq!(c.all().count(), 0);
        assert!(c.by_name.is_empty());
    }
}
