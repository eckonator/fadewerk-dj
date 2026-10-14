// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;

use lofty::config::WriteOptions;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::tag::Tag;

/// Angaben zu einem Titel aus Tags und Dateieigenschaften.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    /// Erscheinungsjahr (nur die Jahreszahl, auch wenn im Tag ein ganzes Datum steht).
    pub year: String,
    pub comment: String,
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
        let get = |k: ItemKey| tag.get_string(k).map(|s| s.trim().to_owned()).unwrap_or_default();
        meta.genre = get(ItemKey::Genre);
        meta.comment = get(ItemKey::Comment);
        let date = Some(get(ItemKey::RecordingDate)).filter(|d| !d.is_empty()).unwrap_or_else(|| get(ItemKey::Year));
        meta.year = year_of(&date);
        meta.bpm = tag
            .get_string(ItemKey::IntegerBpm)
            .or_else(|| tag.get_string(ItemKey::Bpm))
            .and_then(|s| s.trim().replace(',', ".").parse::<f32>().ok())
            .filter(|b| (40.0..=250.0).contains(b));
    }
    Ok(meta)
}

/// Jahreszahl aus einer Datumsangabe wie „2021“, „2021-05-03“ oder „03.05.2021“.
fn year_of(date: &str) -> String {
    date.split(|c: char| !c.is_ascii_digit())
        .find(|p| p.len() == 4)
        .map(str::to_owned)
        .unwrap_or_else(|| date.to_owned())
}

/// Kantenlänge, auf die Cover verkleinert werden. Reicht für den größten Plattenteller.
pub const COVER_SIZE: u32 = 512;

/// Eingebettetes Cover, quadratisch zugeschnitten, als RGBA-Pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// Liest das eingebettete Cover (bevorzugt „Front Cover“, sonst das erste Bild aus irgendeinem Tag).
/// Ohne Bild oder bei unlesbaren Bilddaten `None`.
pub fn read_cover(path: &Path) -> Option<Cover> {
    let file = lofty::read_from_path(path).ok()?;
    let pictures: Vec<_> = file.tags().iter().flat_map(|t| t.pictures()).collect();
    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    match decode_cover(pic.data()) {
        Ok(c) => Some(c),
        Err(e) => {
            log::debug!("Cover in {} nicht lesbar: {e}", path.display());
            None
        }
    }
}

/// Dekodiert JPEG/PNG, schneidet mittig quadratisch zu und verkleinert auf höchstens `COVER_SIZE`.
pub fn decode_cover(data: &[u8]) -> anyhow::Result<Cover> {
    let img = image::load_from_memory(data)?;
    let (w, h) = (img.width(), img.height());
    anyhow::ensure!(w > 0 && h > 0, "leeres Bild");
    let side = w.min(h);
    let img = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let size = side.min(COVER_SIZE);
    let img = if size < side { img.resize_exact(size, size, image::imageops::FilterType::Triangle) } else { img };
    Ok(Cover { size, rgba: img.into_rgba8().into_raw() })
}

/// Bearbeitbare Tag-Felder als Text, wie sie im Editor stehen. Leer = Feld entfernen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagFields {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: String,
    pub track: String,
    pub bpm: String,
    pub comment: String,
}

impl TagFields {
    /// Felder mit ihrem Schlüssel im Tag, in der Reihenfolge des Editors.
    fn keyed(&self) -> [(ItemKey, &String); 8] {
        [
            (ItemKey::TrackTitle, &self.title),
            (ItemKey::TrackArtist, &self.artist),
            (ItemKey::AlbumTitle, &self.album),
            (ItemKey::AlbumArtist, &self.album_artist),
            (ItemKey::Genre, &self.genre),
            (ItemKey::RecordingDate, &self.year),
            (ItemKey::TrackNumber, &self.track),
            (ItemKey::Comment, &self.comment),
        ]
    }
}

/// Technische Angaben zur Datei (nur zum Anzeigen).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileInfo {
    pub format: String,
    /// Art des Tags, in den geschrieben wird, z. B. „ID3v2“.
    pub tag_format: String,
    pub duration_secs: f64,
    pub bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
    pub size_bytes: u64,
}

/// Liest alle bearbeitbaren Tag-Felder und die Dateiangaben.
pub fn read_tags(path: &Path) -> anyhow::Result<(TagFields, FileInfo)> {
    let file = lofty::read_from_path(path)?;
    let props = file.properties();
    let info = FileInfo {
        format: format!("{:?}", file.file_type()),
        tag_format: format!("{:?}", file.primary_tag_type()),
        duration_secs: props.duration().as_secs_f64(),
        bitrate_kbps: props.audio_bitrate(),
        sample_rate: props.sample_rate(),
        bit_depth: props.bit_depth(),
        channels: props.channels(),
        size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
    };
    let mut f = TagFields::default();
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        let get = |k: ItemKey| tag.get_string(k).map(|s| s.trim().to_owned()).unwrap_or_default();
        f.title = get(ItemKey::TrackTitle);
        f.artist = get(ItemKey::TrackArtist);
        f.album = get(ItemKey::AlbumTitle);
        f.album_artist = get(ItemKey::AlbumArtist);
        f.genre = get(ItemKey::Genre);
        f.year = get(ItemKey::RecordingDate);
        if f.year.is_empty() {
            f.year = get(ItemKey::Year);
        }
        f.track = get(ItemKey::TrackNumber);
        f.bpm = get(ItemKey::IntegerBpm);
        if f.bpm.is_empty() {
            f.bpm = get(ItemKey::Bpm);
        }
        f.comment = get(ItemKey::Comment);
    }
    Ok((f, info))
}

/// Schreibt die Felder in den Haupt-Tag der Datei (legt ihn an, wenn es keinen gibt).
/// Leere Felder werden entfernt. Der Ton der Datei bleibt unverändert.
pub fn write_tags(path: &Path, fields: &TagFields) -> anyhow::Result<()> {
    let mut file = lofty::read_from_path(path)?;
    if file.primary_tag().is_none() {
        let kind = file.primary_tag_type();
        file.insert_tag(Tag::new(kind));
    }
    let tag = file.primary_tag_mut().ok_or_else(|| anyhow::anyhow!("Kein Tag in dieser Datei möglich"))?;
    for (key, value) in fields.keyed() {
        let v = value.trim();
        if v.is_empty() {
            tag.remove_key(key);
        } else {
            tag.insert_text(key, v.to_owned());
        }
    }
    // BPM als ganze Zahl, wo das Format es so vorsieht (ID3v2 TBPM), sonst als Text
    tag.remove_key(ItemKey::IntegerBpm);
    tag.remove_key(ItemKey::Bpm);
    let bpm = fields.bpm.trim().replace(',', ".");
    if !bpm.is_empty() {
        let int = bpm.parse::<f32>().map(|b| format!("{:.0}", b)).unwrap_or_else(|_| bpm.clone());
        if !tag.insert_text(ItemKey::IntegerBpm, int) {
            tag.insert_text(ItemKey::Bpm, bpm);
        }
    }
    file.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_from_dates() {
        assert_eq!(year_of("2021"), "2021");
        assert_eq!(year_of("2021-05-03"), "2021");
        assert_eq!(year_of("03.05.2021"), "2021");
        assert_eq!(year_of(""), "");
    }

    /// Kleinste gültige WAV-Datei: 0,1 s Stille, 8 kHz, mono, 16 Bit.
    fn write_wav(path: &Path) {
        let samples = 800u32;
        let data_len = samples * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&16000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.resize(b.len() + data_len as usize, 0);
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn writes_and_reads_back_tags() {
        let path = std::env::temp_dir().join(format!("fw-tags-{}.wav", std::process::id()));
        write_wav(&path);
        let fields = TagFields {
            title: "Ice Blue".into(),
            artist: "FadeWerk".into(),
            album: "Demo".into(),
            genre: "House".into(),
            year: "2026".into(),
            track: "3".into(),
            bpm: "124".into(),
            ..Default::default()
        };
        write_tags(&path, &fields).unwrap();
        let (back, info) = read_tags(&path).unwrap();
        assert_eq!(back.title, "Ice Blue");
        assert_eq!(back.artist, "FadeWerk");
        assert_eq!(back.album, "Demo");
        assert_eq!(back.genre, "House");
        assert_eq!(back.track, "3");
        assert_eq!(back.bpm, "124");
        assert!((info.duration_secs - 0.1).abs() < 0.01);
        let meta = read_meta(&path).unwrap();
        assert_eq!((meta.genre.as_str(), meta.year.as_str()), ("House", "2026"));
        // Ein geleertes Feld verschwindet
        write_tags(&path, &TagFields { genre: String::new(), ..back }).unwrap();
        assert_eq!(read_tags(&path).unwrap().0.genre, "");
        assert_eq!(read_meta(&path).unwrap().title, "Ice Blue");
        std::fs::remove_file(&path).unwrap();
    }

    /// Hängt ein PNG als ID3v2-Cover an (WAV kann ID3v2 tragen).
    pub(crate) fn embed_cover(path: &Path, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb(pixel(x, y)));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut file = lofty::read_from_path(path).unwrap();
        let mut tag = Tag::new(lofty::tag::TagType::Id3v2);
        tag.push_picture(
            lofty::picture::Picture::unchecked(png)
                .pic_type(PictureType::CoverFront)
                .mime_type(lofty::picture::MimeType::Png)
                .build(),
        );
        file.insert_tag(tag);
        file.save_to_path(path, WriteOptions::default()).unwrap();
    }

    #[test]
    fn reads_embedded_cover_cropped_square() {
        let path = std::env::temp_dir().join(format!("fw-cover-{}.wav", std::process::id()));
        write_wav(&path);
        assert!(read_cover(&path).is_none());
        // Breites Bild: links rot, Mitte grün, rechts blau → mittig zugeschnitten bleibt grün
        embed_cover(&path, 900, 600, |x, _| if x < 150 { [255, 0, 0] } else if x < 750 { [0, 255, 0] } else { [0, 0, 255] });
        let cover = read_cover(&path).expect("Cover");
        assert_eq!(cover.size, COVER_SIZE);
        assert_eq!(cover.rgba.len(), (COVER_SIZE * COVER_SIZE * 4) as usize);
        assert_eq!(&cover.rgba[..4], &[0, 255, 0, 255]);
        // Ton und Tags bleiben lesbar
        assert!(read_meta(&path).is_ok());
        std::fs::remove_file(&path).unwrap();
    }

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
