// SPDX-License-Identifier: GPL-3.0-or-later
//! Musikbibliothek ohne Datenbank: Ordner werden beim Öffnen direkt gelesen,
//! Tags und Analysen laufen im Hintergrund.

pub mod bpm;
pub mod decode;
pub mod fx;
pub mod identify;
pub mod key;
pub mod meta;
pub mod scan;
pub mod sketch;
pub mod waveform;
pub mod worker;

pub use key::{Camelot, estimate_key, parse_key};
pub use meta::{Cover, FileInfo, TagFields, TrackMeta, read_cover, read_tags, write_tags, write_tags_with_cover};
pub use scan::{DirListing, is_system_dir, list_dir, walk_audio, walk_audio_progress};
pub use sketch::WaveSketch;
pub use waveform::Waveform;
pub use worker::{Library, Loaded, Response, SCAN_LIMIT, Stamp, file_stamp};
