// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;

use lofty::prelude::*;

/// Angaben zu einem Titel aus Tags und Dateieigenschaften.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    /// BPM aus dem Tag, falls vorhanden. Sonst liefert die Analyse beim Laden einen Wert.
    pub bpm: Option<f32>,
}

impl TrackMeta {
    /// Ersatzangaben aus dem Dateinamen, solange die Tags noch nicht gelesen sind.
    pub fn from_filename(path: &Path) -> Self {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // "Künstler - Titel" ist die häufigste Schreibweise ohne Tags
        let (artist, title) = match stem.split_once(" - ") {
            Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
            None => (String::new(), stem),
        };
        Self { title, artist, ..Default::default() }
    }
}

/// Liest Tags und Laufzeit. Fehlen Tags, bleibt der Dateiname als Titel.
pub fn read_meta(path: &Path) -> anyhow::Result<TrackMeta> {
    let file = lofty::read_from_path(path)?;
    let mut meta = TrackMeta::from_filename(path);
    meta.duration_secs = file.properties().duration().as_secs_f64();
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
            meta.title = t.into_owned();
        }
        if let Some(a) = tag.artist().filter(|a| !a.trim().is_empty()) {
            meta.artist = a.into_owned();
        }
        if let Some(a) = tag.album() {
            meta.album = a.into_owned();
        }
        meta.bpm = tag
            .get_string(ItemKey::IntegerBpm)
            .or_else(|| tag.get_string(ItemKey::Bpm))
            .and_then(|s| s.trim().replace(',', ".").parse::<f32>().ok())
            .filter(|b| (40.0..=250.0).contains(b));
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_artist_and_title_from_filename() {
        let m = TrackMeta::from_filename(Path::new("/m/Daft Punk - One More Time.mp3"));
        assert_eq!(m.artist, "Daft Punk");
        assert_eq!(m.title, "One More Time");
        let m = TrackMeta::from_filename(Path::new("/m/01 Intro.flac"));
        assert_eq!(m.artist, "");
        assert_eq!(m.title, "01 Intro");
    }
}
