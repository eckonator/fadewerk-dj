// SPDX-License-Identifier: GPL-3.0-or-later
//! Erweiterte Suche im Ordnerbrowser: gezielt nach Titel, Interpret, Album, Dateiname, Ordner, Genre,
//! Jahr und BPM filtern. Das Ergebnis filtert die normale Titelliste, zusätzlich zur Schnellsuche.

use std::path::Path;

use fw_library::TrackMeta;
use serde::{Deserialize, Serialize};

/// Ein Feld der erweiterten Suche.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Artist,
    Album,
    FileName,
    Folder,
    Genre,
    Year,
    Bpm,
}

impl Field {
    pub const ALL: [Self; 8] = [Self::Title, Self::Artist, Self::Album, Self::FileName, Self::Folder, Self::Genre, Self::Year, Self::Bpm];

    /// Anzeigename (deutscher Schlüssel für `tr`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "Titel",
            Self::Artist => "Interpret",
            Self::Album => "Album",
            Self::FileName => "Dateiname",
            Self::Folder => "Ordner",
            Self::Genre => "Genre",
            Self::Year => "Jahr",
            Self::Bpm => "BPM",
        }
    }
}

/// Eingaben der erweiterten Suche. Leere Felder filtern nicht. Wird mit „Suchfilter merken“ gespeichert.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSearch {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Dateiname samt Endung.
    pub file_name: String,
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
            Field::FileName => &mut self.file_name,
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
            Field::FileName => &self.file_name,
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

    /// Für viele Titel vorbereitete Suche: Suchwörter einmal klein geschrieben, Bereiche einmal gelesen.
    /// Mit `whole_words` treffen Textfelder nur ganze Wörter.
    pub fn prepare(&self, whole_words: bool) -> PreparedSearch {
        let words = |s: &str| s.to_lowercase().split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let parse = |s: &str| s.trim().replace(',', ".").parse::<f32>().ok();
        PreparedSearch {
            whole_words,
            title: words(&self.title),
            artist: words(&self.artist),
            album: words(&self.album),
            file_name: words(&self.file_name),
            folder: words(&self.folder),
            genre: words(&self.genre),
            year: (parse(&self.year.0), parse(&self.year.1)),
            bpm: (parse(&self.bpm.0), parse(&self.bpm.1)),
        }
    }

    /// Passt der Titel? Für einzelne Prüfungen; bei vielen Titeln [`Self::prepare`] benutzen.
    #[cfg(test)]
    pub fn matches(&self, meta: &TrackMeta, path: &str, bpm: Option<f32>) -> bool {
        self.prepare(false).matches(meta, Path::new(path), bpm)
    }
}

/// Vorbereitete erweiterte Suche, siehe [`AdvancedSearch::prepare`].
pub struct PreparedSearch {
    whole_words: bool,
    title: Vec<String>,
    artist: Vec<String>,
    album: Vec<String>,
    file_name: Vec<String>,
    folder: Vec<String>,
    genre: Vec<String>,
    year: (Option<f32>, Option<f32>),
    bpm: (Option<f32>, Option<f32>),
}

impl PreparedSearch {
    /// Passt der Titel unter `path`? `bpm` ist die bekannte Geschwindigkeit. Textfelder: alle Wörter müssen
    /// vorkommen, Groß/Klein egal; Ordner = Pfad des Dateiordners, Dateiname samt Endung.
    /// Bereiche: Titel ohne Angabe fallen heraus.
    pub fn matches(&self, meta: &TrackMeta, path: &Path, bpm: Option<f32>) -> bool {
        let text_ok = |words: &[String], hay: &str| {
            if words.is_empty() {
                return true;
            }
            let hay = hay.to_lowercase();
            if self.whole_words {
                words.iter().all(|w| contains_word(&hay, w))
            } else {
                words.iter().all(|w| hay.contains(w.as_str()))
            }
        };
        text_ok(&self.title, &meta.title)
            && text_ok(&self.artist, &meta.artist)
            && text_ok(&self.album, &meta.album)
            && text_ok(&self.genre, &meta.genre)
            && in_range(self.year, || meta.year.trim().parse::<f32>().ok())
            && in_range(self.bpm, || bpm.map(|b| b.round()))
            && (self.file_name.is_empty() || text_ok(&self.file_name, &path.file_name().unwrap_or_default().to_string_lossy()))
            && (self.folder.is_empty() || text_ok(&self.folder, &path.parent().map(|d| d.to_string_lossy()).unwrap_or_default()))
    }
}

/// Kommt `word` in `hay` als ganzes Wort vor, also nicht zwischen Buchstaben oder Ziffern?
/// „cro“ trifft „cro & bausa“ und „cro-feat“, nicht „crosby“ oder „macro“.
fn contains_word(hay: &str, word: &str) -> bool {
    let joins = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    hay.match_indices(word).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + word.len()..].chars().next();
        // Ein Satzzeichen am Rand des Suchworts (etwa „ac/dc“) begrenzt selbst
        let starts = word.chars().next().is_some_and(|c| !c.is_alphanumeric());
        let ends = word.chars().next_back().is_some_and(|c| !c.is_alphanumeric());
        (starts || !joins(before)) && (ends || !joins(after))
    })
}

/// Liegt der Wert im Bereich? Leere oder unlesbare Grenzen gelten nicht.
fn in_range((from, to): (Option<f32>, Option<f32>), value: impl FnOnce() -> Option<f32>) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    let Some(v) = value() else { return false };
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
        assert!(s.matches(&m, "/Musik/Sunday.mp3", None));
        // Der Interpret steht nicht im Titel
        let s = AdvancedSearch { title: "u2".into(), ..Default::default() };
        assert!(!s.matches(&m, "/Musik/Sunday.mp3", None));
        let s = AdvancedSearch { genre: "rock".into(), folder: "musik".into(), ..Default::default() };
        assert!(s.matches(&m, "/Musik/Sunday.mp3", None));
        // Dateiname samt Endung, der Ordner gehört nicht dazu
        let s = AdvancedSearch { file_name: "sunday mp3".into(), ..Default::default() };
        assert!(s.matches(&m, "/Musik/Sunday.mp3", None));
        let s = AdvancedSearch { file_name: "musik".into(), ..Default::default() };
        assert!(!s.matches(&m, "/Musik/Sunday.mp3", None));
    }

    #[test]
    fn whole_words_skip_longer_words() {
        let s = AdvancedSearch { artist: "Cro".into(), ..Default::default() }.prepare(true);
        let hit = |artist: &str| s.matches(&meta("Easy", artist, "", ""), Path::new("/x.mp3"), None);
        assert!(hit("Cro"));
        assert!(hit("Cro & Bausa"));
        assert!(hit("Teesy feat. Cro"));
        assert!(hit("CRO-Remix"));
        assert!(!hit("Crosby, Stills & Nash"));
        assert!(!hit("Macro Blue"));
        assert!(!hit("Cro2"));
        // Umlaute zählen als Buchstaben, Satzzeichen im Suchwort gehören dazu
        let s = AdvancedSearch { artist: "bär ac/dc".into(), ..Default::default() }.prepare(true);
        assert!(s.matches(&meta("", "Bär, AC/DC", "", ""), Path::new("/x.mp3"), None));
        assert!(!s.matches(&meta("", "Bären, AC/DC", "", ""), Path::new("/x.mp3"), None));
        // Ohne Häkchen wie bisher
        let s = AdvancedSearch { artist: "Cro".into(), ..Default::default() }.prepare(false);
        assert!(s.matches(&meta("", "Crosby", "", ""), Path::new("/x.mp3"), None));
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
