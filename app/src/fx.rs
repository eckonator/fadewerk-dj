// SPDX-License-Identifier: GPL-3.0-or-later
//! Effekt-Pads: mitgelieferte Klänge oder eigene Dateien aus den Einstellungen, im Hintergrund geladen.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use fw_engine::{FX_SLOTS, TrackAudio};
use serde::{Deserialize, Serialize};

/// Namen der mitgelieferten Effekte (deutscher Schlüssel für `tr`), Reihenfolge wie `fw_library::fx::builtin`.
pub const BUILTIN_NAMES: [&str; FX_SLOTS] = ["HORN", "SIRENE", "LASER", "BOOM", "RISER", "DROP", "REWIND", "BREMSE"];

/// Ein Pad in den Einstellungen: eigener Name (leer = Name des Klangs) und eigene Datei (`None` = mitgeliefert).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxSlot {
    pub name: String,
    pub file: Option<PathBuf>,
    /// Symbol (`FxIcon::key`), `None` = das des mitgelieferten Effekts auf diesem Pad.
    pub icon: Option<String>,
}

impl FxSlot {
    pub fn icon(&self, slot: usize) -> crate::fx_icons::FxIcon {
        self.icon.as_deref().and_then(crate::fx_icons::FxIcon::from_key).unwrap_or(crate::fx_icons::FxIcon::DEFAULTS[slot])
    }

    /// Beschriftung des Pads: eigener Name, sonst Dateiname, sonst Name des mitgelieferten Klangs.
    pub fn label(&self, slot: usize) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_owned();
        }
        match &self.file {
            Some(f) => f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            None => crate::i18n::tr(BUILTIN_NAMES[slot]).to_owned(),
        }
    }
}

/// Ergebnis eines Ladevorgangs: Pad, Klang, Fehlermeldung (dann ist der Klang der mitgelieferte).
type Loaded = (usize, Arc<TrackAudio>, Option<String>);

/// Lädt Klänge für die Pads in einem eigenen Thread und hält die geladenen fest (zum erneuten Senden,
/// wenn die Audio-Ausgabe neu startet).
pub struct FxBank {
    pub sounds: [Option<Arc<TrackAudio>>; FX_SLOTS],
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl FxBank {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { sounds: Default::default(), tx, rx }
    }

    /// Lädt den Klang eines Pads neu: eigene Datei oder (ohne Datei bzw. wenn sie nicht lesbar ist) mitgeliefert.
    pub fn load(&self, slot: usize, file: Option<PathBuf>) {
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("fw-fx".into()).spawn(move || {
            let (sound, err) = match file.as_deref().map(decode) {
                Some(Ok(a)) => (a, None),
                Some(Err(e)) => (fw_library::fx::builtin(slot), Some(e)),
                None => (fw_library::fx::builtin(slot), None),
            };
            let _ = tx.send((slot, Arc::new(sound), err));
        });
        if let Err(e) = spawned {
            log::warn!("Effekt-Thread nicht gestartet: {e}");
        }
    }

    /// Fertig geladene Klänge seit dem letzten Aufruf.
    pub fn poll(&mut self) -> Vec<Loaded> {
        let done: Vec<Loaded> = self.rx.try_iter().collect();
        for (slot, sound, _) in &done {
            self.sounds[*slot] = Some(sound.clone());
        }
        done
    }
}

fn decode(path: &Path) -> Result<TrackAudio, String> {
    fw_library::decode::decode_file(path).map_err(|e| {
        log::warn!("Effekt {} nicht lesbar: {e:#}", path.display());
        format!("{e:#}")
    })
}
