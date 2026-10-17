// SPDX-License-Identifier: GPL-3.0-or-later
//! Aufräumen unter Einstellungen → Allgemein: Größe der Caches messen, Caches leeren und
//! FadeWerk DJ ganz zurücksetzen.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use crate::i18n::{Lang, lang};
use crate::settings::config_dir;

/// Dateien und Ordner der Caches im Einstellungsordner.
const TAGS: &str = "tags.json";
const FOLDERS: &str = "ordner.json";
const BPM: &str = "bpm.json";
const KEYS: &str = "tonart.json";
pub const WAVES: &str = "wellenformen";

/// Belegter Platz der Caches in Bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheSizes {
    pub tags: u64,
    pub folders: u64,
    pub waves: u64,
    /// Ermittelte BPM und Tonarten.
    pub bpm: u64,
}

fn size_of(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path).map_or(0, |entries| entries.filter_map(Result::ok).map(|e| size_of(&e.path())).sum()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Misst die Caches im Hintergrund (der Wellenform-Ordner kann Zehntausende Dateien haben).
pub fn measure() -> Receiver<CacheSizes> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("fw-cache-size".into()).spawn(move || {
        let Some(dir) = config_dir() else { return };
        let _ = tx.send(CacheSizes {
            tags: size_of(&dir.join(TAGS)),
            folders: size_of(&dir.join(FOLDERS)),
            waves: size_of(&dir.join(WAVES)),
            bpm: size_of(&dir.join(BPM)) + size_of(&dir.join(KEYS)),
        });
    });
    if let Err(e) = spawned {
        log::warn!("Cache-Größe nicht messbar: {e}");
    }
    rx
}

/// Löscht die Cache-Dateien (mit `bpm` auch die ermittelten BPM und Tonarten). Der Wellenform-Ordner wird im
/// Hintergrund geleert. Die Caches im Speicher leert der Aufrufer.
pub fn clear(bpm: bool) {
    let Some(dir) = config_dir() else { return };
    let mut files = vec![TAGS, FOLDERS];
    if bpm {
        files.push(BPM);
        files.push(KEYS);
    }
    for name in files {
        remove(&dir.join(name));
    }
    let waves = dir.join(WAVES);
    let _ = std::thread::Builder::new().name("fw-cache-clear".into()).spawn(move || remove(&waves));
}

fn remove(path: &Path) {
    let result = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    match result {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("{} nicht gelöscht: {e}", path.display()),
    }
}

/// Löscht alles im Einstellungsordner: Einstellungen, Playlists, Historie, Caches, Log. Der Ordner
/// selbst bleibt (leer), sonst würden beim nächsten Start die Einstellungen eines früheren
/// Programmnamens wieder übernommen (`settings::migrate_old_dir`). Musikdateien liegen nie hier.
pub fn reset_all() -> Result<(), String> {
    let dir = config_dir().ok_or("kein Einstellungsordner")?;
    wipe(&dir)
}

fn wipe(dir: &Path) -> Result<(), String> {
    // Nur den eigenen Ordner leeren, nie etwas anderes
    if dir.file_name().is_none_or(|n| n != "FadeWerk DJ") {
        return Err(format!("unerwarteter Ordner {}", dir.display()));
    }
    let entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(e) => e.filter_map(Result::ok).map(|e| e.path()).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    log::info!("FadeWerk DJ wird zurückgesetzt ({} Einträge in {})", entries.len(), dir.display());
    for p in entries {
        // Die offene Logdatei lässt sich unter Windows nicht löschen; das stört nicht
        remove(&p);
    }
    Ok(())
}

/// Größe lesbar, z. B. „1,8 MB“ (Deutsch mit Komma).
pub fn fmt_bytes(bytes: u64) -> String {
    let (v, unit) = match bytes {
        b if b >= 1_000_000_000 => (b as f64 / 1e9, "GB"),
        b if b >= 1_000_000 => (b as f64 / 1e6, "MB"),
        b if b >= 1_000 => (b as f64 / 1e3, "KB"),
        b => return format!("{b} B"),
    };
    let s = format!("{v:.1}");
    let s = if lang() == Lang::De { s.replace('.', ",") } else { s };
    format!("{s} {unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipe_empties_only_our_folder() {
        let base = std::env::temp_dir().join(format!("fw-reset-{}", std::process::id()));
        let dir = base.join("FadeWerk DJ");
        std::fs::create_dir_all(dir.join(WAVES)).unwrap();
        std::fs::write(dir.join("einstellungen.json"), "{}").unwrap();
        std::fs::write(dir.join(WAVES).join("a.fww"), "x").unwrap();
        assert!(size_of(&dir) >= 3);
        wipe(&dir).unwrap();
        assert!(dir.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        // Ein fremder Ordner wird nicht angefasst
        let other = base.join("Musik");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("titel.mp3"), "x").unwrap();
        assert!(wipe(&other).is_err());
        assert!(other.join("titel.mp3").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sizes_read_like_a_person_would() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert!(fmt_bytes(1_800_000).ends_with(" MB"));
    }
}
