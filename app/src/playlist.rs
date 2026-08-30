// SPDX-License-Identifier: GPL-3.0-or-later
//! Playlist eines Players: Reihenfolge, laufender Titel, Verschieben.

use std::path::PathBuf;

/// Was per Drag & Drop transportiert wird.
#[derive(Clone, Debug)]
pub enum DragItem {
    /// Titel aus dem Ordnerbrowser.
    Files(Vec<PathBuf>),
    /// Eine Zeile aus einer Playlist.
    Row { list: usize, index: usize },
}

#[derive(Default, Debug, Clone)]
pub struct Playlist {
    pub entries: Vec<PathBuf>,
    /// Index des Titels, der im Player geladen ist (oder zuletzt war).
    pub current: Option<usize>,
}

impl Playlist {
    /// Nächster Titel nach dem aktuellen. Mit `wrap` geht es am Ende von vorn los.
    pub fn next_index(&self, wrap: bool) -> Option<usize> {
        let n = self.current.map_or(0, |c| c + 1);
        if n < self.entries.len() {
            Some(n)
        } else if wrap && !self.entries.is_empty() {
            Some(0)
        } else {
            None
        }
    }

    pub fn prev_index(&self) -> Option<usize> {
        self.current.and_then(|c| c.checked_sub(1))
    }

    pub fn insert(&mut self, at: usize, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let at = at.min(self.entries.len());
        let mut n = 0;
        for (k, p) in paths.into_iter().enumerate() {
            self.entries.insert(at + k, p);
            n += 1;
        }
        if let Some(c) = self.current.as_mut()
            && at <= *c {
                *c += n;
            }
        n
    }

    pub fn push(&mut self, paths: impl IntoIterator<Item = PathBuf>) -> usize {
        let len = self.entries.len();
        self.insert(len, paths)
    }

    pub fn remove(&mut self, i: usize) -> Option<PathBuf> {
        if i >= self.entries.len() {
            return None;
        }
        let p = self.entries.remove(i);
        self.current = match self.current {
            Some(c) if c == i => None,
            Some(c) if i < c => Some(c - 1),
            c => c,
        };
        Some(p)
    }

    /// Verschiebt einen Eintrag. `before` ist die Einfügestelle in der Liste vor dem Entfernen.
    /// Der laufende Titel bleibt markiert, egal wohin er oder andere wandern.
    pub fn move_entry(&mut self, from: usize, before: usize) {
        if from >= self.entries.len() {
            return;
        }
        let to = if before > from { before - 1 } else { before }.min(self.entries.len() - 1);
        if to == from {
            return;
        }
        let p = self.entries.remove(from);
        self.entries.insert(to, p);
        if let Some(c) = self.current.as_mut() {
            if *c == from {
                *c = to;
            } else {
                if from < *c {
                    *c -= 1;
                }
                if to <= *c {
                    *c += 1;
                }
            }
        }
    }

    /// Mischt die kommenden Titel zufällig. Der geladene Titel und die schon gespielten davor bleiben,
    /// wo sie sind, damit ein laufender Set nicht durcheinanderkommt. Ohne geladenen Titel wird alles gemischt.
    pub fn shuffle_upcoming(&mut self, seed: u64) {
        let start = self.current.map_or(0, |c| c + 1).min(self.entries.len());
        let tail = &mut self.entries[start..];
        // Xorshift genügt hier; Fisher-Yates für eine gleichmäßige Verteilung
        let mut x = seed | 1;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for i in (1..tail.len()).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            tail.swap(i, j);
        }
    }

    /// Anzahl der Titel, die `shuffle_upcoming` mischen würde.
    pub fn upcoming(&self) -> usize {
        self.entries.len().saturating_sub(self.current.map_or(0, |c| c + 1))
    }

    pub fn clear_keep(&mut self, keep_current: bool) {
        match (keep_current, self.current) {
            (true, Some(c)) => {
                let p = self.entries[c].clone();
                self.entries = vec![p];
                self.current = Some(0);
            }
            _ => {
                self.entries.clear();
                self.current = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(n: usize, current: Option<usize>) -> Playlist {
        Playlist { entries: (0..n).map(|i| PathBuf::from(format!("{i}"))).collect(), current }
    }

    fn names(p: &Playlist) -> String {
        p.entries.iter().map(|e| e.to_string_lossy().into_owned()).collect::<Vec<_>>().join("")
    }

    #[test]
    fn play_next_moves_behind_current() {
        // Titel 4 hinter den laufenden Titel 1: 0 1 4 2 3, der laufende bleibt markiert
        let mut p = pl(5, Some(1));
        p.move_entry(4, 2);
        assert_eq!(names(&p), "01423");
        assert_eq!(p.current, Some(1));
        // Ein schon gespielter Titel (0) wandert ebenfalls hinter den laufenden
        p.move_entry(0, 2);
        assert_eq!(names(&p), "10423");
        assert_eq!(p.current, Some(0));
    }

    #[test]
    fn moving_down_and_up_keeps_current_track() {
        let mut p = pl(5, Some(2));
        p.move_entry(0, 5); // 0 ans Ende
        assert_eq!(names(&p), "12340");
        assert_eq!(p.current, Some(1)); // Titel "2" steht jetzt an Stelle 1
        p.move_entry(4, 0); // zurück an den Anfang
        assert_eq!(names(&p), "01234");
        assert_eq!(p.current, Some(2));
    }

    #[test]
    fn moving_the_current_track_moves_the_marker() {
        let mut p = pl(4, Some(1));
        p.move_entry(1, 4);
        assert_eq!(names(&p), "0231");
        assert_eq!(p.current, Some(3));
    }

    #[test]
    fn dropping_on_itself_changes_nothing() {
        let mut p = pl(3, Some(0));
        p.move_entry(1, 1);
        p.move_entry(1, 2);
        assert_eq!(names(&p), "012");
    }

    #[test]
    fn next_and_wrap() {
        let p = pl(2, Some(1));
        assert_eq!(p.next_index(false), None);
        assert_eq!(p.next_index(true), Some(0));
        assert_eq!(pl(2, None).next_index(false), Some(0));
    }

    #[test]
    fn shuffle_keeps_played_and_current_in_place() {
        let mut p = pl(10, Some(3));
        p.shuffle_upcoming(42);
        assert_eq!(&names(&p)[..4], "0123", "gespielte und laufender Titel bleiben");
        let mut rest: Vec<char> = names(&p)[4..].chars().collect();
        assert_ne!(rest.iter().collect::<String>(), "456789", "der Rest ist gemischt");
        rest.sort();
        assert_eq!(rest.iter().collect::<String>(), "456789", "kein Titel geht verloren");
        assert_eq!(p.current, Some(3));
    }

    #[test]
    fn shuffle_without_current_mixes_everything() {
        let mut p = pl(6, None);
        p.shuffle_upcoming(7);
        let mut all: Vec<char> = names(&p).chars().collect();
        assert_ne!(all.iter().collect::<String>(), "012345");
        all.sort();
        assert_eq!(all.iter().collect::<String>(), "012345");
    }

    #[test]
    fn insert_and_remove_shift_current() {
        let mut p = pl(3, Some(1));
        p.insert(0, [PathBuf::from("x")]);
        assert_eq!(p.current, Some(2));
        p.remove(0);
        assert_eq!(p.current, Some(1));
        p.remove(1);
        assert_eq!(p.current, None);
    }
}
