// SPDX-License-Identifier: GPL-3.0-or-later
//! Fehlende Titel finden und verschobene wiederfinden (Playlists beim Start, „Playlist prüfen“, Historie-Import).
//!
//! Gesucht wird über den Dateinamen in den Musikordnern. Ist die Größe bekannt, muss sie passen; sonst nur,
//! wenn es genau eine Datei dieses Namens gibt (sonst landet „01 Intro.mp3“ beim falschen Album).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
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
}
