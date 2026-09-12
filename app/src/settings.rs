// SPDX-License-Identifier: GPL-3.0-or-later
//! Gespeicherte Einstellungen: Sprache, Skin, Audiogerät, Mixer- und Auto-Fade-Werte, eigene Ordner.
//!
//! Liegt als JSON im Konfigurationsordner des Systems
//! (macOS: ~/Library/Application Support/FadeWerk DJ/einstellungen.json).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::theme::SkinId;

/// Grundwerte der Regler: Startwerte und Ziel von Rechts- bzw. Doppelklick auf einen Fader.
pub const DEFAULT_CHANNEL_VOLUME: f32 = 0.85;
pub const DEFAULT_MASTER_VOLUME: f32 = 0.9;
pub const DEFAULT_HEAD_VOLUME: f32 = 0.8;
pub const DEFAULT_HEAD_MIX: f32 = 0.0;
pub const DEFAULT_FADE_SECS: f32 = 8.0;
/// Regelbereich des EQ in dB. Ganz zu ist nicht ganz stumm, dafür gibt es die Kill-Tasten.
pub const EQ_MIN_DB: f32 = -26.0;
pub const EQ_MAX_DB: f32 = 6.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Lang,
    pub skin: SkinId,
    /// Flächen als gebürsteter Stahl.
    pub brushed_steel: bool,
    /// Höhe des Ordnerbrowsers in Punkten, 0 = automatisch (ein knappes Drittel des Fensters).
    pub browser_height: f32,
    /// Breite des Ordnerbaums im Browser in Punkten, 0 = Standard.
    pub tree_width: f32,
    /// Sichtbare Spalten der Titelliste im Browser (Schlüssel aus `browser::TrackColumn`), Titel ist immer da.
    pub track_columns: Vec<String>,
    /// Vom Nutzer gezogene Spaltenbreiten der Titelliste in Punkten, je Spaltenschlüssel.
    pub column_widths: Vec<(String, f32)>,
    /// Erweiterte Suche beim Schließen und Beenden behalten.
    pub remember_search: bool,
    /// Zuletzt benutzte erweiterte Suche (nur mit `remember_search`).
    pub advanced_search: crate::search::AdvancedSearch,
    /// „Mit Unterordnern“ je Ordner, nur wo der Nutzer vom Standard abweicht.
    pub folder_recursion: Vec<(PathBuf, bool)>,
    /// Kennung des Ausgabegeräts (cpal `DeviceId`), `None` = Systemstandard.
    pub output_device: Option<String>,
    /// Eigenes Kopfhörergerät, `None` = Kopfhörer über Split L/R oder Ausgang 3/4 des Master-Geräts.
    pub head_device: Option<String>,
    /// Puffergröße in Samples, `None` = Standard des Geräts.
    pub buffer_frames: Option<u32>,
    pub spin_up_secs: f32,
    pub spin_down_secs: f32,
    pub fade_secs: f32,
    pub auto_fade: bool,
    /// 0 = weich, 1 = linear, 2 = hart, 3 = eigene Kurve aus `curve_points`
    pub curve: u8,
    /// Pegel von A an den Stützstellen der eigenen Blendkurve.
    pub curve_points: [f32; 4],
    pub master_volume: f32,
    pub head_volume: f32,
    pub head_mix: f32,
    /// EQ je Player in dB, Reihenfolge Bässe, Mitten, Höhen.
    pub eq_db: [[f32; 3]; 2],
    /// Beim Auto-Fade die Bässe in der Mitte der Überblendung tauschen.
    pub bass_swap: bool,
    /// Vom Nutzer hinzugefügte Musikordner.
    pub folders: Vec<PathBuf>,
    /// Fenster beim Start im Vollbild öffnen (zuletzt so verlassen).
    pub fullscreen: bool,
    /// Grafik über OpenGL statt wgpu (für Rechner, auf denen wgpu nicht zuverlässig startet).
    pub opengl: bool,
    /// Zoom der ganzen Oberfläche (⌘/Strg + Plus/Minus), 1 = normal.
    pub ui_zoom: f32,
    /// Höhe der Player-Zeile in Punkten, vom Nutzer gezogen; 0 = automatisch.
    pub deck_height: f32,
    /// Beim Start bei GitHub nach einer neuen Version fragen; `None` = noch nicht gefragt.
    pub update_check: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Lang::De,
            skin: SkinId::Nacht,
            brushed_steel: false,
            browser_height: 0.0,
            tree_width: 0.0,
            track_columns: crate::browser::TrackColumn::DEFAULT.iter().map(|c| c.key().to_owned()).collect(),
            column_widths: Vec::new(),
            remember_search: true,
            advanced_search: Default::default(),
            folder_recursion: Vec::new(),
            output_device: None,
            head_device: None,
            buffer_frames: None,
            spin_up_secs: 0.5,
            spin_down_secs: 1.6,
            fade_secs: DEFAULT_FADE_SECS,
            auto_fade: true,
            curve: 0,
            curve_points: fw_engine::Curve::Soft.points(),
            master_volume: DEFAULT_MASTER_VOLUME,
            head_volume: DEFAULT_HEAD_VOLUME,
            head_mix: DEFAULT_HEAD_MIX,
            eq_db: [[0.0; 3]; 2],
            bass_swap: false,
            folders: Vec::new(),
            fullscreen: false,
            opengl: false,
            ui_zoom: 1.0,
            deck_height: 0.0,
            update_check: None,
        }
    }
}

/// Schreibt erst in eine Zwischendatei und benennt sie dann um. So bleibt bei einem Absturz
/// mitten im Schreiben die alte Datei heil.
pub fn write_atomic(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("FadeWerk DJ"))
}

fn path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("einstellungen.json"))
}

/// Frühere Namen des Programms, der neueste zuerst.
const OLD_DIRS: [&str; 2] = ["FadeWerk", "Zweiklang"];

/// Das Programm hieß früher „Zweiklang“ und kurz „FadeWerk“. Gibt es den neuen Ordner noch nicht,
/// werden Einstellungen und Playlists aus dem neuesten alten Ordner einmalig übernommen
/// (kopiert, der alte Ordner bleibt).
pub fn migrate_old_dir() {
    let (Some(base), Some(new)) = (dirs::config_dir(), config_dir()) else { return };
    let Some(old) = OLD_DIRS.iter().map(|n| base.join(n)).find(|d| d.is_dir()) else { return };
    if new.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&new) {
        log::warn!("Ordner {} nicht angelegt: {e}", new.display());
        return;
    }
    for name in ["einstellungen.json", "playlists.json"] {
        let from = old.join(name);
        if from.is_file()
            && let Err(e) = std::fs::copy(&from, new.join(name))
        {
            log::warn!("{} nicht übernommen: {e}", from.display());
        }
    }
    log::info!("Einstellungen aus {} übernommen", old.display());
}

impl Settings {
    pub fn load() -> Self {
        migrate_old_dir();
        let Some(p) = path() else { return Self::default() };
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("Einstellungen in {} unlesbar, nehme Standardwerte: {e}", p.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Err(e) = write_atomic(&p, &serde_json::to_string_pretty(self).unwrap_or_default()) {
            log::warn!("Einstellungen nicht gespeichert ({}): {e}", p.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"language":"En","neu_in_spaeterer_version":1}"#).unwrap();
        assert_eq!(s.language, Lang::En);
        assert_eq!(s.fade_secs, 8.0);
        let round: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(round, s);
    }
}
