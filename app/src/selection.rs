// SPDX-License-Identifier: GPL-3.0-or-later
//! Mehrfachauswahl in Listen, wie im Dateimanager: Klick wählt einen Eintrag, ⌘/Strg-Klick nimmt
//! einzelne hinzu oder heraus, ⇧-Klick wählt den Bereich ab dem zuletzt geklickten Eintrag.

use std::collections::HashSet;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub struct Selection<T> {
    items: HashSet<T>,
    /// Ausgangspunkt für ⇧-Klick.
    anchor: Option<T>,
    /// Zuletzt angeklickter oder per Pfeiltaste erreichter Eintrag, von hier aus gehen die Pfeiltasten weiter.
    cursor: Option<T>,
}

impl<T> Default for Selection<T> {
    fn default() -> Self {
        Self { items: HashSet::new(), anchor: None, cursor: None }
    }
}

impl<T: Clone + Eq + Hash> Selection<T> {
    pub fn contains(&self, item: &T) -> bool {
        self.items.contains(item)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Die ausgewählten Einträge in beliebiger Reihenfolge.
    pub fn items(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    /// Einträge umschreiben oder herausnehmen (`None`), etwa wenn sich die Liste darunter verändert hat.
    pub fn remap(&mut self, mut f: impl FnMut(&T) -> Option<T>) {
        self.items = self.items.iter().filter_map(&mut f).collect();
        self.anchor = self.anchor.as_ref().and_then(&mut f);
        self.cursor = self.cursor.as_ref().and_then(&mut f);
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.anchor = None;
        self.cursor = None;
    }

    pub fn select_only(&mut self, item: &T) {
        self.items.clear();
        self.items.insert(item.clone());
        self.anchor = Some(item.clone());
        self.cursor = Some(item.clone());
    }

    /// Klick auf `item` in der sichtbaren Liste `rows`. `toggle` = ⌘/Strg, `range` = ⇧.
    pub fn click(&mut self, rows: &[T], item: &T, toggle: bool, range: bool) {
        self.cursor = Some(item.clone());
        let anchor_pos = self.anchor.as_ref().and_then(|a| rows.iter().position(|r| r == a));
        match (range, anchor_pos, rows.iter().position(|r| r == item)) {
            (true, Some(a), Some(b)) => {
                if !toggle {
                    self.items.clear();
                }
                self.items.extend(rows[a.min(b)..=a.max(b)].iter().cloned());
                // Der Anker bleibt, damit weitere ⇧-Klicks den Bereich vom selben Punkt aus ändern
            }
            _ if toggle => {
                if !self.items.remove(item) {
                    self.items.insert(item.clone());
                }
                self.anchor = Some(item.clone());
            }
            _ => self.select_only(item),
        }
    }

    /// Pfeiltasten: `steps` Zeilen weiter (↓ positiv), mit `extend` (⇧) wird der Bereich ab dem Anker gewählt.
    /// Liefert die neue Zeile, damit die Liste dorthin scrollen kann.
    pub fn step(&mut self, rows: &[T], steps: i32, extend: bool) -> Option<usize> {
        let from = self.cursor.as_ref().and_then(|c| rows.iter().position(|r| r == c));
        let to = crate::keynav::step(from, steps, rows.len())?;
        let item = rows[to].clone();
        if extend && from.is_some() {
            self.click(rows, &item, false, true);
        } else {
            self.select_only(&item);
        }
        Some(to)
    }

    /// Zuletzt angeklickter oder per Pfeiltaste erreichter Eintrag.
    pub fn cursor(&self) -> Option<&T> {
        self.cursor.as_ref()
    }

    pub fn select_all(&mut self, rows: &[T]) {
        self.items.extend(rows.iter().cloned());
    }

    /// Die ausgewählten Einträge in der Reihenfolge von `rows`. Ausgefilterte bleiben außen vor.
    pub fn ordered(&self, rows: &[T]) -> Vec<T> {
        if self.items.is_empty() {
            return Vec::new();
        }
        rows.iter().filter(|r| self.items.contains(r)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [u32; 6] = [0, 1, 2, 3, 4, 5];

    #[test]
    fn click_toggle_and_range() {
        let mut s = Selection::default();
        s.click(&ROWS, &1, false, false);
        assert_eq!(s.ordered(&ROWS), [1]);
        s.click(&ROWS, &4, true, false);
        assert_eq!(s.ordered(&ROWS), [1, 4]);
        s.click(&ROWS, &1, true, false);
        assert_eq!(s.ordered(&ROWS), [4]);
        // Bereich vom Anker (zuletzt geklickt: 1) bis 3, ersetzt die bisherige Auswahl
        s.click(&ROWS, &3, false, true);
        assert_eq!(s.ordered(&ROWS), [1, 2, 3]);
        // Weiterer ⇧-Klick vom selben Anker aus
        s.click(&ROWS, &0, false, true);
        assert_eq!(s.ordered(&ROWS), [0, 1]);
        // ⌘⇧-Klick erweitert, statt zu ersetzen
        s.click(&ROWS, &5, true, true);
        assert_eq!(s.ordered(&ROWS), [0, 1, 2, 3, 4, 5]);
        s.click(&ROWS, &3, false, false);
        assert_eq!(s.ordered(&ROWS), [3]);
    }

    #[test]
    fn arrow_keys_move_and_extend() {
        let mut s = Selection::default();
        assert_eq!(s.step(&ROWS, 1, false), Some(0));
        s.click(&ROWS, &2, false, false);
        assert_eq!(s.step(&ROWS, 1, false), Some(3));
        assert_eq!(s.ordered(&ROWS), [3]);
        assert_eq!(s.step(&ROWS, 2, true), Some(5));
        assert_eq!(s.ordered(&ROWS), [3, 4, 5]);
        // ⇧↑ verkleinert den Bereich wieder Richtung Anker
        assert_eq!(s.step(&ROWS, -1, true), Some(4));
        assert_eq!(s.ordered(&ROWS), [3, 4]);
        assert_eq!(s.step(&ROWS, -9, false), Some(0));
        assert_eq!(s.ordered(&ROWS), [0]);
        assert_eq!(s.step(&[], 1, false), None);
    }

    #[test]
    fn range_without_anchor_selects_single() {
        let mut s = Selection::default();
        s.click(&ROWS, &3, false, true);
        assert_eq!(s.ordered(&ROWS), [3]);
    }

    #[test]
    fn ordered_follows_rows_and_skips_hidden() {
        let mut s = Selection::default();
        s.select_all(&ROWS);
        assert_eq!(s.ordered(&[5, 0, 9]), [5, 0]);
        s.clear();
        assert!(s.ordered(&ROWS).is_empty());
    }
}
