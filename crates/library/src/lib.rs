// SPDX-License-Identifier: GPL-3.0-or-later
//! Musikbibliothek ohne Datenbank: Ordner werden beim Öffnen direkt gelesen,
//! Tags und Analysen laufen im Hintergrund.

pub mod bpm;
pub mod decode;
pub mod meta;
pub mod scan;
pub mod waveform;
pub mod worker;

pub use meta::TrackMeta;
pub use scan::{list_dir, walk_audio, DirListing};
pub use waveform::Waveform;
pub use worker::{Library, Loaded, Response, SCAN_LIMIT};
