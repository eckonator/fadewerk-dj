// SPDX-License-Identifier: GPL-3.0-or-later
//! Sitzung: Playlists A und B überstehen Beenden und Abstürze.
//!
//! Liegt neben den Einstellungen als `playlists.json`. Die App speichert beim Beenden und zusätzlich
//! alle paar Sekunden, sobald sich etwas geändert hat.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::playlist::Playlist;
use crate::settings::{config_dir, write_atomic};

#[derive(Serialize, Deserialize, Default)]
struct SavedList {
    entries: Vec<PathBuf>,
    current: Option<usize>,
}

#[derive(Serialize, Deserialize, Default)]
struct Session {
    playlists: [SavedList; 2],
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("playlists.json"))
}

/// Lädt die gespeicherten Playlists. Dateien, die es nicht mehr gibt, fallen heraus;
/// liefert außerdem, wie viele das waren.
pub fn load() -> ([Playlist; 2], usize) {
    let Some(p) = path() else { return (Default::default(), 0) };
    let Ok(text) = std::fs::read_to_string(&p) else { return (Default::default(), 0) };
    let session: Session = match serde_json::from_str(&text) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Playlists in {} unlesbar: {e}", p.display());
            return (Default::default(), 0);
        }
    };
    let mut missing = 0;
    let lists = session.playlists.map(|saved| {
        let mut pl = Playlist { entries: saved.entries, current: saved.current };
        // Rückwärts entfernen, damit `remove` den aktuellen Titel richtig mitverschiebt
        for i in (0..pl.entries.len()).rev() {
            if !pl.entries[i].is_file() {
                pl.remove(i);
                missing += 1;
            }
        }
        pl
    });
    (lists, missing)
}

pub fn save(lists: &[Playlist; 2]) {
    let Some(p) = path() else { return };
    let session = Session {
        playlists: [0, 1].map(|d| SavedList { entries: lists[d].entries.clone(), current: lists[d].current }),
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

/// Fingerabdruck der Playlists, um Änderungen billig zu erkennen.
pub fn fingerprint(lists: &[Playlist; 2]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for pl in lists {
        pl.entries.hash(&mut h);
        pl.current.hash(&mut h);
    }
    h.finish()
}
