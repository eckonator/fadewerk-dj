// SPDX-License-Identifier: GPL-3.0-or-later
//! Fehlende Titel finden und verschobene wiederfinden (Playlists beim Start, „Playlist prüfen“, Historie-Import).
//!
//! Gesucht wird über den Dateinamen in den Musikordnern. Ist die Größe bekannt, muss sie passen; sonst nur,
//! wenn es genau eine Datei dieses Namens gibt (sonst landet „01 Intro.mp3“ beim falschen Album).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// Neue Orte fehlender Dateien: alter Pfad → gefundener Pfad.
pub fn find_moved(missing: &[(PathBuf, Option<u64>)], roots: &[PathBuf]) -> HashMap<PathBuf, PathBuf> {
    let names: HashSet<OsString> = missing.iter().filter_map(|(p, _)| p.file_name().map(|n| n.to_owned())).collect();
    let mut moved = HashMap::new();
    if names.is_empty() {
        return moved;
    }
    let mut found: HashMap<OsString, Vec<PathBuf>> = HashMap::new();
    for root in roots.iter().filter(|r| r.is_dir()) {
        fw_library::walk_audio(root, || true, |tracks| {
            for t in tracks {
                if let Some(n) = t.file_name().filter(|n| names.contains(*n)) {
                    let list = found.entry(n.to_owned()).or_default();
                    if !list.contains(&t) {
                        list.push(t.clone());
                    }
                }
            }
        });
    }
    for (path, size) in missing {
        let Some(cands) = path.file_name().and_then(|n| found.get(n)) else { continue };
        let hit = match size {
            Some(s) => cands.iter().find(|p| std::fs::metadata(p).is_ok_and(|m| m.len() == *s)),
            None if cands.len() == 1 => cands.first(),
            None => None,
        };
        if let Some(p) = hit {
            moved.insert(path.clone(), p.clone());
        }
    }
    moved
}

/// Nächster noch vorhandener Ordner über `path`, in dem sich das Suchen lohnt (nicht `/`, `/Volumes` oder der
/// Benutzerordner selbst): bei einem umbenannten Albumordner liegt die Datei meist darunter.
fn nearest_dir(path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir();
    path.ancestors()
        .skip(1)
        .find(|a| a.is_dir())
        .filter(|a| a.components().count() >= 3 && home.as_deref() != Some(*a))
        .map(Path::to_path_buf)
}

/// Sucht eine einzelne fehlende Datei im Thread `fw-relocate`: erst im nächsten vorhandenen Ordner darüber,
/// dann in den Musikordnern. Liefert den neuen Ort, wenn er eindeutig ist.
pub fn find_one_in_background(path: PathBuf, roots: Vec<PathBuf>, repaint: impl Fn() + Send + 'static) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("fw-relocate".into()).spawn(move || {
        let wanted = [(path.clone(), None)];
        let near = nearest_dir(&path);
        let mut hit = near.as_ref().and_then(|n| find_moved(&wanted, std::slice::from_ref(n)).remove(&path));
        if hit.is_none() {
            let rest: Vec<PathBuf> = roots.into_iter().filter(|r| near.as_ref().is_none_or(|n| !r.starts_with(n))).collect();
            hit = find_moved(&wanted, &rest).remove(&path);
        }
        let _ = tx.send(hit);
        repaint();
    });
    if let Err(e) = spawned {
        log::warn!("Suche startet nicht: {e}");
    }
    rx
}

/// Ergebnis einer Prüfung: Dateien, die es nicht gibt, und davon wiedergefundene.
pub struct Checked {
    pub missing: Vec<PathBuf>,
    pub moved: HashMap<PathBuf, PathBuf>,
}

/// Prüft im Thread `fw-check`, welche Dateien fehlen (auf Netzlaufwerken dauert das), und sucht sie in `roots`.
pub fn check_in_background(paths: Vec<PathBuf>, roots: Vec<PathBuf>, repaint: impl Fn() + Send + 'static) -> Receiver<Checked> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("fw-check".into()).spawn(move || {
        let mut seen = HashSet::new();
        let missing: Vec<PathBuf> = paths.into_iter().filter(|p| seen.insert(p.clone())).filter(|p| !p.is_file()).collect();
        let wanted: Vec<(PathBuf, Option<u64>)> = missing.iter().map(|p| (p.clone(), None)).collect();
        let moved = find_moved(&wanted, &roots);
        let _ = tx.send(Checked { missing, moved });
        repaint();
    });
    if let Err(e) = spawned {
        log::warn!("Prüfung startet nicht: {e}");
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_moved_files_by_name_and_size() {
        let dir = std::env::temp_dir().join(format!("fw-relocate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (sub, name, len) in [("neu", "a.wav", 10), ("x", "intro.wav", 5), ("y", "intro.wav", 7), ("z", "b.wav", 3)] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join(name), vec![0u8; len]).unwrap();
        }
        let missing = [
            (PathBuf::from("/alt/a.wav"), None),
            (PathBuf::from("/alt/intro.wav"), None),
            (PathBuf::from("/alt2/intro.wav"), Some(7)),
            (PathBuf::from("/alt/b.wav"), Some(99)),
            (PathBuf::from("/alt/weg.wav"), None),
        ];
        let moved = find_moved(&missing, std::slice::from_ref(&dir));
        assert_eq!(moved.get(&PathBuf::from("/alt/a.wav")), Some(&dir.join("neu/a.wav")));
        assert_eq!(moved.get(&PathBuf::from("/alt/intro.wav")), None, "zwei Kandidaten ohne Größe: keiner");
        assert_eq!(moved.get(&PathBuf::from("/alt2/intro.wav")), Some(&dir.join("y/intro.wav")));
        assert_eq!(moved.get(&PathBuf::from("/alt/b.wav")), None, "Größe passt nicht");
        assert_eq!(moved.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_file_after_album_folder_was_renamed() {
        let dir = std::env::temp_dir().join(format!("fw-relocate-one-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let new = dir.join("Alben/Bravo Hits Vol/Vol.18/CD1/03 Mr. Wichtig.mp3");
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::fs::write(&new, b"x").unwrap();
        let old = dir.join("Alben/Bravo_Hits_Vol/Vol.18/CD1/03 Mr. Wichtig.mp3");
        assert_eq!(nearest_dir(&old), Some(dir.join("Alben")));
        let hit = find_one_in_background(old, Vec::new(), || {}).recv().unwrap();
        assert_eq!(hit, Some(new));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
