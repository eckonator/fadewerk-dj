// SPDX-License-Identifier: GPL-3.0-or-later
//! Echtzeit-Audio-Engine von FadeWerk DJ.
//!
//! Die Engine läuft im Audio-Thread und bekommt Befehle über einen lock-freien
//! Ringpuffer. Zustand für die Oberfläche (Positionen, Pegel) wird über Atomics
//! veröffentlicht. Im Audio-Thread wird weder gesperrt noch Speicher angelegt
//! oder freigegeben: ausgeworfene Titel gehen zum Aufräumen an die Oberfläche zurück.

mod atomic;
pub mod automix;
pub mod deck;
pub mod engine;
pub mod eq;
pub mod filter;
pub mod headlink;
pub mod mixer;
pub mod motor;
pub mod sampler;
pub mod spectrum;
pub mod testtone;
pub mod track;

pub use automix::AutoFade;
pub use deck::Deck;
pub use eq::{Eq, db_to_gain};
pub use filter::Filter;
pub use engine::{Command, Controller, DeckStatus, Engine, Shared, DECKS, PREVIEW, VOICES};
pub use headlink::{HeadReceiver, HeadSink, head_link};
pub use mixer::{CURVE_KNOTS, Curve, HeadMode};
pub use motor::Motor;
pub use sampler::FX_SLOTS;
pub use spectrum::{BAND_HZ, BANDS};
pub use track::TrackAudio;
