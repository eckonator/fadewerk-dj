// SPDX-License-Identifier: GPL-3.0-or-later
//! Erweiterte Suche im Ordnerbrowser: gezielt nach Titel, Interpret, Album, Ordner, Genre,
//! Jahr und BPM filtern. Das Ergebnis filtert die normale Titelliste, zusätzlich zur Schnellsuche.

use fw_library::TrackMeta;
use serde::{Deserialize, Serialize};

/// Ein Feld der erweiterten Suche.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Artist,
    Album,
    Folder,
    Genre,
    Year,
    Bpm,
}

impl Field {
    pub const ALL: [Self; 7] = [Self::Title, Self::Artist, Self::Album, Self::Folder, Self::Genre, Self::Year, Self::Bpm];

    /// Anzeigename (deutscher Schlüssel für `tr`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "Titel",
            Self::Artist => "Interpret",
            Self::Album => "Album",
            Self::Folder => "Ordner",
            Self::Genre => "Genre",
            Self::Year => "Jahr",
            Self::Bpm => "BPM",
        }
    }
}

/// Eingaben der erweiterten Suche. Leere Felder filtern nicht. Wird mit „Suchfilter merken“ gespeichert.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSearch {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub folder: String,
    pub genre: String,
    pub year: (String, String),
    pub bpm: (String, String),
}

impl AdvancedSearch {
    pub fn is_empty(&self) -> bool {
        Field::ALL.into_iter().all(|f| !self.is_set(f))
    }

    pub fn is_set(&self, f: Field) -> bool {
        match self.range(f) {
            Some((a, b)) => !a.trim().is_empty() || !b.trim().is_empty(),
            None => !self.text(f).trim().is_empty(),
        }
    }

    /// Textfeld zum Bearbeiten (für Bereichsfelder das „von“).
    pub fn text_mut(&mut self, f: Field) -> &mut String {
        match f {
            Field::Title => &mut self.title,
            Field::Artist => &mut self.artist,
            Field::Album => &mut self.album,
            Field::Folder => &mut self.folder,
            Field::Genre => &mut self.genre,
            Field::Year => &mut self.year.0,
            Field::Bpm => &mut self.bpm.0,
        }
    }

    pub fn range_mut(&mut self, f: Field) -> Option<(&mut String, &mut String)> {
        match f {
            Field::Year => Some((&mut self.year.0, &mut self.year.1)),
            Field::Bpm => Some((&mut self.bpm.0, &mut self.bpm.1)),
            _ => None,
        }
    }

    fn text(&self, f: Field) -> &str {
        match f {
            Field::Title => &self.title,
            Field::Artist => &self.artist,
            Field::Album => &self.album,
            Field::Folder => &self.folder,
            Field::Genre => &self.genre,
            Field::Year => &self.year.0,
            Field::Bpm => &self.bpm.0,
        }
    }

    fn range(&self, f: Field) -> Option<(&str, &str)> {
        match f {
            Field::Year => Some((&self.year.0, &self.year.1)),
            Field::Bpm => Some((&self.bpm.0, &self.bpm.1)),
            _ => None,
        }
    }

    /// Entwicklerschalter `FW_SEARCH="genre=Rock,year=1980-1989"`: Felder vorbelegen.
    pub fn from_dev_spec(spec: &str) -> Self {
        let mut s = Self::default();
        for (key, value) in spec.split(',').filter_map(|kv| kv.split_once('=')) {
            let Some(f) = Field::ALL.into_iter().find(|f| format!("{f:?}").eq_ignore_ascii_case(key.trim())) else { continue };
            match (s.range_mut(f), value.split_once('-')) {
                (Some((a, b)), Some((from, to))) => {
                    *a = from.into();
                    *b = to.into();
                }
                _ => *s.text_mut(f) = value.into(),
            }
        }
        s
    }

    pub fn clear(&mut self, f: Field) {
        match self.range_mut(f) {
            Some((a, b)) => {
                a.clear();
                b.clear();
            }
            None => self.text_mut(f).clear(),
        }
    }

    /// Kurzform für die Filterleiste, z. B. „Rock“ oder „1980–1989“, „ab 120“.
    pub fn summary(&self, f: Field) -> String {
        match self.range(f) {
            Some((a, b)) => match (a.trim(), b.trim()) {
                (a, "") => format!("≥ {a}"),
                ("", b) => format!("≤ {b}"),
                (a, b) if a == b => a.to_owned(),
                (a, b) => format!("{a}–{b}"),
            },
            None => self.text(f).trim().to_owned(),
        }
    }

    /// Passt der Titel? `folder` ist der Ordner der Datei (Pfad), `bpm` die bekannte Geschwindigkeit.
    /// Textfelder: alle Wörter müssen vorkommen, Groß/Klein egal. Bereiche: Titel ohne Angabe fallen heraus.
    pub fn matches(&self, meta: &TrackMeta, folder: &str, bpm: Option<f32>) -> bool {
        let text_ok = |query: &str, hay: &str| {
            let hay = hay.to_lowercase();
            query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
        };
        let year = meta.year.trim().parse::<f32>().ok();
        text_ok(&self.title, &meta.title)
            && text_ok(&self.artist, &meta.artist)
            && text_ok(&self.album, &meta.album)
            && text_ok(&self.folder, folder)
            && text_ok(&self.genre, &meta.genre)
            && in_range(&self.year, year)
            && in_range(&self.bpm, bpm.map(|b| b.round()))
    }
}

/// Liegt `value` im Bereich? Leere Grenzen gelten nicht; unlesbare Grenzen auch nicht.
fn in_range((from, to): &(String, String), value: Option<f32>) -> bool {
    let parse = |s: &str| s.trim().replace(',', ".").parse::<f32>().ok();
    let (from, to) = (parse(from), parse(to));
    if from.is_none() && to.is_none() {
        return true;
    }
    let Some(v) = value else { return false };
    from.is_none_or(|f| v >= f) && to.is_none_or(|t| v <= t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str, artist: &str, genre: &str, year: &str) -> TrackMeta {
        TrackMeta { title: title.into(), artist: artist.into(), genre: genre.into(), year: year.into(), ..Default::default() }
    }

    #[test]
    fn empty_search_matches_everything() {
        let s = AdvancedSearch::default();
        assert!(s.is_empty());
        assert!(s.matches(&meta("", "", "", ""), "", None));
    }

    #[test]
    fn text_fields_need_all_words_in_their_field() {
        let m = meta("Sunday Bloody Sunday", "U2", "Rock/Pop", "1983");
        let s = AdvancedSearch { title: "bloody sun".into(), ..Default::default() };
        assert!(s.matches(&m, "/Musik", None));
        // Der Interpret steht nicht im Titel
        let s = AdvancedSearch { title: "u2".into(), ..Default::default() };
        assert!(!s.matches(&m, "/Musik", None));
        let s = AdvancedSearch { genre: "rock".into(), folder: "musik".into(), ..Default::default() };
        assert!(s.matches(&m, "/Musik", None));
    }

    #[test]
    fn ranges_include_bounds_and_drop_unknown() {
        let m = meta("A", "B", "", "1983");
        let s = AdvancedSearch { year: ("1980".into(), "1989".into()), ..Default::default() };
        assert!(s.matches(&m, "", None));
        assert!(!s.matches(&meta("A", "B", "", "1990"), "", None));
        assert!(!s.matches(&meta("A", "B", "", ""), "", None));
        let s = AdvancedSearch { bpm: ("120".into(), String::new()), ..Default::default() };
        assert!(s.matches(&m, "", Some(119.6)));
        assert!(!s.matches(&m, "", Some(110.0)));
        assert!(!s.matches(&m, "", None));
        assert_eq!(s.summary(Field::Bpm), "≥ 120");
    }
}
