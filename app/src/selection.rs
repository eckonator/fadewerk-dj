// SPDX-License-Identifier: GPL-3.0-or-later
//! Mehrfachauswahl in Listen, wie im Dateimanager: Klick wählt einen Eintrag, ⌘/Strg-Klick nimmt
//! einzelne hinzu oder heraus, ⇧-Klick wählt den Bereich ab dem zuletzt geklickten Eintrag.

use std::collections::HashSet;
use std::hash::Hash;

#[derive(Debug)]
pub struct Selection<T> {
    items: HashSet<T>,
    /// Ausgangspunkt für ⇧-Klick.
    anchor: Option<T>,
}

impl<T> Default for Selection<T> {
    fn default() -> Self {
        Self { items: HashSet::new(), anchor: None }
    }
}

impl<T: Clone + Eq + Hash> Selection<T> {
    pub fn contains(&self, item: &T) -> bool {
        self.items.contains(item)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.anchor = None;
    }

    pub fn select_only(&mut self, item: &T) {
        self.items.clear();
        self.items.insert(item.clone());
        self.anchor = Some(item.clone());
    }

    /// Klick auf `item` in der sichtbaren Liste `rows`. `toggle` = ⌘/Strg, `range` = ⇧.
    pub fn click(&mut self, rows: &[T], item: &T, toggle: bool, range: bool) {
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
