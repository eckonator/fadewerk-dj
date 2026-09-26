// SPDX-License-Identifier: GPL-3.0-or-later
//! Musikbibliothek ohne Datenbank: Ordner werden beim Öffnen direkt gelesen,
//! Tags und Analysen laufen im Hintergrund.

pub mod bpm;
pub mod decode;
pub mod fx;
pub mod meta;
pub mod scan;
pub mod waveform;
pub mod worker;

pub use meta::{Cover, FileInfo, TagFields, TrackMeta, read_cover, read_tags, write_tags};
pub use scan::{DirListing, is_system_dir, list_dir, walk_audio};
pub use waveform::Waveform;
pub use worker::{Library, Loaded, Response, SCAN_LIMIT};
