// SPDX-License-Identifier: GPL-3.0-or-later
//! Gemerkte Titellisten langsamer Ordner (etwa großer Sammlungen auf Netzlaufwerken).
//!
//! Beim Öffnen zeigt der Browser die gemerkte Liste sofort, während die Live-Suche wie immer im
//! Hintergrund läuft; deren Ergebnis ersetzt die Liste und wird wieder gemerkt. Der Cache ist also nur
//! ein schneller erster Blick, nie die Wahrheit. Gemerkt werden nur Ordner, deren Suche mindestens
//! [`MIN_SCAN_SECS`] gedauert hat, höchstens [`MAX_FOLDERS`] (die zuletzt benutzten), in `ordner.json`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

/// Erst ab dieser Suchdauer lohnt das Merken (schnelle, lokale Ordner bleiben draußen).
pub const MIN_SCAN_SECS: f64 = 1.0;
pub const MAX_FOLDERS: usize = 20;
/// So lange bleibt ein Ordner, der nicht geöffnet wird.
const EXPIRE_DAYS: u64 = 90;
const FORMAT: &str = "FadeWerk-DJ-Ordner";

/// Gesuchte Ordner, je mit „Unterordner ja/nein“, in der Reihenfolge der Suche.
pub type Roots = Vec<(PathBuf, bool)>;

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    roots: Roots,
    /// Zuletzt geöffnet, Sekunden seit 1970.
    used: u64,
    /// Titel als Läufe „Ordner, Dateinamen“ in der Reihenfolge der Suche (der Ordnerpfad steht so nur
    /// einmal je Lauf drin).
    runs: Vec<(PathBuf, Vec<String>)>,
}

impl Entry {
    fn tracks(&self) -> Vec<PathBuf> {
        self.runs.iter().flat_map(|(dir, names)| names.iter().map(move |n| dir.join(n))).collect()
    }
}

fn runs(tracks: &[PathBuf]) -> Vec<(PathBuf, Vec<String>)> {
    let mut out: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for t in tracks {
        let (Some(dir), Some(name)) = (t.parent(), t.file_name()) else { continue };
        let name = name.to_string_lossy().into_owned();
        match out.last_mut() {
            Some((d, names)) if d == dir => names.push(name),
            _ => out.push((dir.to_path_buf(), vec![name])),
        }
    }
    out
}

#[derive(Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    folders: Vec<Entry>,
}

#[derive(Default)]
pub struct FolderCache {
    entries: Vec<Entry>,
    dirty: bool,
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("ordner.json"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl FolderCache {
    pub fn load() -> Self {
        let Some(p) = path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        match Self::from_json(&text, now_secs()) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Ordner-Cache in {} unlesbar, fange neu an: {e}", p.display());
                Self::default()
            }
        }
    }

    fn from_json(text: &str, now: u64) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!("unbekanntes Format „{}“", file.format));
        }
        let cutoff = now.saturating_sub(EXPIRE_DAYS * 24 * 60 * 60);
        let n = file.folders.len();
        let mut entries: Vec<Entry> = file.folders.into_iter().filter(|e| e.used >= cutoff && !e.roots.is_empty()).collect();
        entries.truncate(MAX_FOLDERS);
        Ok(Self { dirty: entries.len() != n, entries })
    }

    fn to_json(&self) -> Result<String, String> {
        let file = File { format: FORMAT.into(), version: 1, folders: self.entries.clone() };
        serde_json::to_string(&file).map_err(|e| e.to_string())
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(p) = path() else { return };
        match self.to_json().and_then(|json| write_atomic(&p, &json).map_err(|e| e.to_string())) {
            Ok(()) => self.dirty = false,
            Err(e) => log::warn!("Ordner-Cache nicht gespeichert ({}): {e}", p.display()),
        }
    }

    /// Gemerkte Titel dieser Ordner, sonst `None`.
    pub fn get(&mut self, roots: &[(PathBuf, bool)]) -> Option<Vec<PathBuf>> {
        let e = self.entries.iter_mut().find(|e| e.roots == roots)?;
        e.used = now_secs();
        self.dirty = true;
        Some(e.tracks())
    }

    pub fn contains(&self, roots: &[(PathBuf, bool)]) -> bool {
        self.entries.iter().any(|e| e.roots == roots)
    }

    /// Merkt das Ergebnis einer fertigen Live-Suche; der am längsten nicht benutzte Ordner fällt heraus,
    /// wenn es mehr als [`MAX_FOLDERS`] werden.
    pub fn put(&mut self, roots: Roots, tracks: &[PathBuf]) {
        self.entries.retain(|e| e.roots != roots);
        self.entries.insert(0, Entry { roots, used: now_secs(), runs: runs(tracks) });
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.used));
        self.entries.truncate(MAX_FOLDERS);
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(p: &str) -> Roots {
        vec![(PathBuf::from(p), true)]
    }

    #[test]
    fn round_trip_keeps_order_and_groups_folders() {
        let tracks: Vec<PathBuf> = ["/net/M/A/1.mp3", "/net/M/A/2.mp3", "/net/M/B/1.mp3", "/net/M/A/3.mp3"].iter().map(PathBuf::from).collect();
        let mut c = FolderCache::default();
        c.put(roots("/net/M"), &tracks);
        let json = c.to_json().unwrap();
        assert_eq!(json.matches("/net/M/B").count(), 1);
        let mut back = FolderCache::from_json(&json, now_secs()).unwrap();
        assert_eq!(back.get(&roots("/net/M")), Some(tracks));
        assert_eq!(back.get(&roots("/net/andere")), None);
        assert_eq!(back.get(&[(PathBuf::from("/net/M"), false)]), None);
    }

    #[test]
    fn keeps_only_the_most_recent_folders() {
        let mut c = FolderCache::default();
        for i in 0..MAX_FOLDERS + 5 {
            c.put(roots(&format!("/net/{i}")), &[PathBuf::from(format!("/net/{i}/x.mp3"))]);
            c.entries[0].used = i as u64 + 1;
        }
        assert_eq!(c.entries.len(), MAX_FOLDERS);
        assert!(c.contains(&roots(&format!("/net/{}", MAX_FOLDERS + 4))));
        assert!(!c.contains(&roots("/net/0")));
    }

    #[test]
    fn broken_or_old_files_start_fresh() {
        assert!(FolderCache::from_json("[]", 0).is_err());
        let mut c = FolderCache::default();
        c.put(roots("/alt"), &[PathBuf::from("/alt/a.mp3")]);
        c.entries[0].used = 0;
        let back = FolderCache::from_json(&c.to_json().unwrap(), now_secs()).unwrap();
        assert!(back.entries.is_empty());
    }
}
