// SPDX-License-Identifier: GPL-3.0-or-later
//! Playlist eines Players: Reihenfolge, laufender Titel, Verschieben.

use std::path::PathBuf;

use crate::fadewerk::TrackFade;

/// Was per Drag & Drop transportiert wird.
#[derive(Clone, Debug)]
pub enum DragItem {
    /// Titel aus dem Ordnerbrowser.
    Files(Vec<PathBuf>),
    /// Eine Zeile aus einer Playlist.
    Row { list: usize, index: usize },
    /// Mehrere markierte Zeilen aus einer Playlist, aufsteigend.
    Rows { list: usize, indices: Vec<usize> },
}

#[derive(Default, Debug, Clone)]
pub struct Playlist {
    pub entries: Vec<PathBuf>,
    /// Index des Titels, der im Player geladen ist (oder zuletzt war).
    pub current: Option<usize>,
    /// FadeWerk-Bereiche je Eintrag, parallel zu `entries`. Kann kürzer sein (fehlende = nicht gesetzt),
    /// die Methoden hier halten beides gleich lang.
    pub fades: Vec<TrackFade>,
}

impl Playlist {
    pub fn new(entries: Vec<PathBuf>, current: Option<usize>) -> Self {
        Self { entries, current, fades: Vec::new() }
    }

    /// Bringt `fades` auf die Länge von `entries` (etwa nachdem `entries` direkt gesetzt wurde).
    fn sync(&mut self) {
        self.fades.resize(self.entries.len(), TrackFade::default());
    }

    pub fn fade(&self, i: usize) -> TrackFade {
        self.fades.get(i).copied().unwrap_or_default()
    }

    pub fn set_fade(&mut self, i: usize, fade: TrackFade) {
        self.sync();
        if let Some(f) = self.fades.get_mut(i) {
            *f = fade;
        }
    }

    pub fn has_fades(&self) -> bool {
        self.fades.iter().any(|f| !f.is_empty())
    }

    pub fn clear_fades(&mut self) {
        self.fades.clear();
        self.sync();
    }

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
        self.insert_with_fades(at, paths.into_iter().map(|p| (p, TrackFade::default())))
    }

    /// Wie [`Playlist::insert`], mit den Fade-Bereichen der Einträge (beim Verschieben zwischen den Playlists).
    pub fn insert_with_fades(&mut self, at: usize, items: impl IntoIterator<Item = (PathBuf, TrackFade)>) -> usize {
        self.sync();
        let at = at.min(self.entries.len());
        let mut n = 0;
        for (k, (p, f)) in items.into_iter().enumerate() {
            self.entries.insert(at + k, p);
            self.fades.insert(at + k, f);
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
        self.remove_with_fade(i).map(|(p, _)| p)
    }

    pub fn remove_with_fade(&mut self, i: usize) -> Option<(PathBuf, TrackFade)> {
        if i >= self.entries.len() {
            return None;
        }
        self.sync();
        let p = self.entries.remove(i);
        let f = self.fades.remove(i);
        self.current = match self.current {
            Some(c) if c == i => None,
            Some(c) if i < c => Some(c - 1),
            c => c,
        };
        Some((p, f))
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
        self.sync();
        let p = self.entries.remove(from);
        self.entries.insert(to, p);
        let f = self.fades.remove(from);
        self.fades.insert(to, f);
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

    /// Verschiebt mehrere Einträge als Block in ihrer bisherigen Reihenfolge. `before` ist die Einfügestelle
    /// in der Liste vor dem Entfernen. Der laufende Titel bleibt markiert.
    pub fn move_entries(&mut self, indices: &[usize], before: usize) {
        self.sync();
        let n = self.entries.len();
        let mut picked = vec![false; n];
        indices.iter().filter(|&&i| i < n).for_each(|&i| picked[i] = true);
        let current = self.current;
        let items = self.entries.drain(..).zip(self.fades.drain(..)).enumerate();
        let (mut block, mut rest): (Vec<_>, Vec<_>) = items.map(|(i, (p, f))| (p, f, current == Some(i), picked[i])).partition(|e| e.3);
        let at = (0..before.min(n)).filter(|&i| !picked[i]).count();
        rest.splice(at..at, block.drain(..));
        self.current = rest.iter().position(|e| e.2);
        (self.entries, self.fades) = rest.into_iter().map(|(p, f, _, _)| (p, f)).unzip();
    }

    /// Nimmt mehrere Einträge heraus und liefert sie samt Fade-Bereichen in ihrer bisherigen Reihenfolge.
    pub fn remove_many(&mut self, indices: &[usize]) -> Vec<(PathBuf, TrackFade)> {
        let mut sorted: Vec<usize> = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut out: Vec<_> = sorted.into_iter().rev().filter_map(|i| self.remove_with_fade(i)).collect();
        out.reverse();
        out
    }

    /// Mischt die kommenden Titel zufällig. Der geladene Titel und die schon gespielten davor bleiben,
    /// wo sie sind, damit ein laufender Set nicht durcheinanderkommt. Ohne geladenen Titel wird alles gemischt.
    pub fn shuffle_upcoming(&mut self, seed: u64) {
        self.sync();
        let start = self.current.map_or(0, |c| c + 1).min(self.entries.len());
        let tail = &mut self.entries[start..];
        let fades = &mut self.fades[start..];
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
            fades.swap(i, j);
        }
    }

    /// Ordnet die kommenden Titel (nach dem geladenen) neu: `order[k]` = bisherige Stelle (ab dem ersten
    /// kommenden gezählt) des Titels, der an Stelle `k` rückt. Fade-Bereiche wandern mit. Ungültiges wird ignoriert.
    pub fn reorder_upcoming(&mut self, order: &[usize]) {
        self.sync();
        let start = self.current.map_or(0, |c| c + 1).min(self.entries.len());
        let n = self.entries.len() - start;
        let mut seen = vec![false; n];
        if order.len() != n || !order.iter().all(|&i| i < n && !std::mem::replace(&mut seen[i], true)) {
            return;
        }
        let entries: Vec<PathBuf> = order.iter().map(|&i| self.entries[start + i].clone()).collect();
        let fades: Vec<TrackFade> = order.iter().map(|&i| self.fades[start + i]).collect();
        self.entries.splice(start.., entries);
        self.fades.splice(start.., fades);
    }

    /// Die kommenden Titel (nach dem geladenen).
    pub fn upcoming_entries(&self) -> &[PathBuf] {
        &self.entries[self.current.map_or(0, |c| c + 1).min(self.entries.len())..]
    }

    /// Anzahl der Titel, die `shuffle_upcoming` mischen würde.
    pub fn upcoming(&self) -> usize {
        self.entries.len().saturating_sub(self.current.map_or(0, |c| c + 1))
    }

    pub fn clear_keep(&mut self, keep_current: bool) {
        match (keep_current, self.current) {
            (true, Some(c)) => {
                let p = self.entries[c].clone();
                let f = self.fade(c);
                self.entries = vec![p];
                self.fades = vec![f];
                self.current = Some(0);
            }
            _ => {
                self.entries.clear();
                self.fades.clear();
                self.current = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(n: usize, current: Option<usize>) -> Playlist {
        Playlist::new((0..n).map(|i| PathBuf::from(format!("{i}"))).collect(), current)
    }

    fn names(p: &Playlist) -> String {
        p.entries.iter().map(|e| e.to_string_lossy().into_owned()).collect::<Vec<_>>().join("")
    }

    #[test]
    fn fades_move_with_their_entries() {
        use crate::fadewerk::{Region, TrackFade};
        let mark = |x: f32| TrackFade { fade_in: Some(Region { start: 0.0, end: x }), fade_out: None };
        let mut p = pl(4, Some(0));
        p.set_fade(3, mark(3.0));
        p.set_fade(1, mark(1.0));
        p.move_entry(3, 1);
        assert_eq!(names(&p), "0312");
        assert_eq!((p.fade(1), p.fade(2)), (mark(3.0), mark(1.0)));
        p.shuffle_upcoming(42);
        for i in 0..4 {
            let n: f32 = p.entries[i].to_string_lossy().parse().unwrap();
            let expect = if n == 3.0 || n == 1.0 { mark(n) } else { TrackFade::default() };
            assert_eq!(p.fade(i), expect, "Eintrag {n}");
        }
        let at = p.entries.iter().position(|e| e.to_string_lossy() == "3").unwrap();
        let item = p.remove_with_fade(at).unwrap();
        let mut q = pl(1, None);
        q.insert_with_fades(0, [item]);
        assert_eq!(q.fade(0), mark(3.0));
        assert_eq!(q.fade(1), TrackFade::default());
        q.clear_fades();
        assert!(!q.has_fades());
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
    fn moving_several_keeps_their_order_and_the_current_track() {
        let mut p = pl(6, Some(2));
        // 1, 2 (läuft) und 4 hinter die 5
        p.move_entries(&[4, 1, 2], 6);
        assert_eq!(names(&p), "035124");
        assert_eq!(p.current, Some(4));
        // zurück an den Anfang
        p.move_entries(&[3, 4, 5], 0);
        assert_eq!(names(&p), "124035");
        assert_eq!(p.current, Some(1));
        // Einfügestelle mitten im Block
        p.move_entries(&[0, 2], 1);
        assert_eq!(names(&p), "142035");
    }

    #[test]
    fn removing_several() {
        let mut p = pl(5, Some(3));
        let gone = p.remove_many(&[4, 0, 9, 0]);
        assert_eq!(gone.iter().map(|(e, _)| e.to_string_lossy().into_owned()).collect::<String>(), "04");
        assert_eq!(names(&p), "123");
        assert_eq!(p.current, Some(2));
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
