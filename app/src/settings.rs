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
pub const DEFAULT_FX_VOLUME: f32 = 0.8;
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
    /// Plattenteller samt Tonarm in den Playern zeigen.
    pub platters: bool,
    /// Höhe des Ordnerbrowsers in Punkten, 0 = automatisch (ein knappes Drittel des Fensters).
    pub browser_height: f32,
    /// Ordnerbrowser rechts neben Playern und Playlists statt darunter (für breite Monitore).
    pub browser_side: bool,
    /// Breite des Ordnerbrowsers rechts in Punkten, 0 = automatisch.
    pub browser_width: f32,
    /// Breite des Ordnerbaums im Browser in Punkten, 0 = Standard.
    pub tree_width: f32,
    /// Höhe des Ordnerbaums über der Titelliste (Browser rechts) in Punkten, 0 = automatisch.
    pub tree_height: f32,
    /// Ordner in Spalten statt als Baum zeigen.
    pub folder_columns: bool,
    /// Anteil der Spaltenansicht an der Browserbreite, wenn der Browser unten steht, 0 = Standard (40 %).
    pub columns_share: f32,
    /// Sichtbare Spalten der Titelliste im Browser (Schlüssel aus `browser::TrackColumn`), Titel ist immer da.
    pub track_columns: Vec<String>,
    /// Vom Nutzer gezogene Spaltenbreiten der Titelliste in Punkten, je Spaltenschlüssel.
    pub column_widths: Vec<(String, f32)>,
    /// Sortierung der Titelliste: (Spaltenschlüssel, absteigend), `None` = Reihenfolge der Ordner.
    pub track_sort: Option<(String, bool)>,
    /// Effekt-Pads: eigene Namen und Dateien (fehlende Einträge = mitgeliefert).
    pub fx_slots: Vec<crate::fx::FxSlot>,
    /// Lautstärke der Effekt-Pads.
    pub fx_volume: f32,
    /// Offener Reiter unter dem Crossfader (`mixer_ui::CenterTab::key`), `None` = alle zugeklappt.
    pub center_tab: Option<String>,
    /// Erweiterte Suche beim Schließen und Beenden behalten.
    pub remember_search: bool,
    /// Beim Suchen Kopien desselben Titels ausblenden.
    pub hide_duplicates: bool,
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
    /// Auto-Sync: neuen Titel ans Tempo des laufenden angleichen, danach langsam auf 0 % Pitch.
    pub auto_sync: bool,
    /// Vom Nutzer hinzugefügte Musikordner.
    pub folders: Vec<PathBuf>,
    /// Fenster beim Start im Vollbild öffnen (zuletzt so verlassen).
    pub fullscreen: bool,
    /// Sparsame Darstellung für ältere oder langsame Rechner (ohne Effekte, weniger Bilder pro Sekunde).
    pub lite_ui: bool,
    /// Grafik über OpenGL statt wgpu (für Rechner, auf denen wgpu nicht zuverlässig startet).
    pub opengl: bool,
    /// Zoom der ganzen Oberfläche (⌘/Strg + Plus/Minus), 1 = normal.
    pub ui_zoom: f32,
    /// Höhe der Player-Zeile in Punkten, vom Nutzer gezogen; 0 = automatisch.
    pub deck_height: f32,
    /// Beim Start bei GitHub nach einer neuen Version fragen; `None` = noch nicht gefragt.
    pub update_check: Option<bool>,
    /// Gespielte Titel in der Historie aufzeichnen.
    pub record_history: bool,
    /// Bildschirm und Rechner nicht in den Ruhezustand gehen lassen, solange Musik spielt.
    pub keep_awake: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Lang::De,
            skin: SkinId::Nacht,
            brushed_steel: false,
            platters: true,
            browser_height: 0.0,
            browser_side: false,
            browser_width: 0.0,
            tree_width: 0.0,
            tree_height: 0.0,
            folder_columns: false,
            columns_share: 0.0,
            track_columns: crate::browser::TrackColumn::DEFAULT.iter().map(|c| c.key().to_owned()).collect(),
            column_widths: Vec::new(),
            track_sort: None,
            center_tab: Some("pads".into()),
            fx_slots: Vec::new(),
            fx_volume: DEFAULT_FX_VOLUME,
            remember_search: true,
            hide_duplicates: true,
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
            auto_sync: false,
            folders: Vec::new(),
            fullscreen: false,
            lite_ui: false,
            opengl: false,
            ui_zoom: 1.0,
            deck_height: 0.0,
            update_check: None,
            record_history: true,
            keep_awake: true,
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
    /// Einstellung eines Effekt-Pads (Standard, wenn noch nichts eingetragen ist).
    pub fn fx_slot(&self, slot: usize) -> crate::fx::FxSlot {
        self.fx_slots.get(slot).cloned().unwrap_or_default()
    }

    /// Effekt-Pad ändern; die Liste wächst bei Bedarf mit Standard-Einträgen.
    pub fn fx_slot_mut(&mut self, slot: usize) -> &mut crate::fx::FxSlot {
        if self.fx_slots.len() <= slot {
            self.fx_slots.resize(slot + 1, Default::default());
        }
        &mut self.fx_slots[slot]
    }

    pub fn load() -> Self {
        migrate_old_dir();
        let Some(p) = path() else { return Self::default() };
        match std::fs::read_to_string(&p) {
            Ok(s) => Self::from_json(&s).unwrap_or_else(|e| {
                log::warn!("Einstellungen in {} unlesbar, nehme Standardwerte: {e}", p.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Liest Einstellungen nachsichtig: Ein Eintrag, den diese Version nicht versteht (etwa ein Skin aus einer
    /// neueren Version oder ein kaputter Wert), fällt einzeln auf den Standard zurück, statt alle zurückzusetzen.
    /// Danach werden Zahlen auf sinnvolle Bereiche begrenzt.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let serde_json::Value::Object(fields) = file else { return Err("kein JSON-Objekt".into()) };
        let mut base = serde_json::to_value(Self::default()).map_err(|e| e.to_string())?;
        let mut parsed: Self = match serde_json::from_value(serde_json::Value::Object(fields.clone())) {
            Ok(s) => s,
            Err(_) => {
                for (key, value) in fields {
                    let mut candidate = base.clone();
                    candidate[&key] = value.clone();
                    if serde_json::from_value::<Self>(candidate).is_ok() {
                        base[&key] = value;
                    } else {
                        log::warn!("Einstellung „{key}“ unlesbar, nehme den Standardwert");
                    }
                }
                serde_json::from_value(base).map_err(|e| e.to_string())?
            }
        };
        parsed.sanitize();
        Ok(parsed)
    }

    /// Begrenzt Zahlen auf die Bereiche der Regler; Unsinn (negativ, riesig, unendlich) wird zum Standard.
    fn sanitize(&mut self) {
        let d = Self::default();
        let fit = |v: f32, lo: f32, hi: f32, def: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.browser_height = fit(self.browser_height, 0.0, 10_000.0, 0.0);
        self.browser_width = fit(self.browser_width, 0.0, 10_000.0, 0.0);
        self.tree_width = fit(self.tree_width, 0.0, 5_000.0, 0.0);
        self.tree_height = fit(self.tree_height, 0.0, 5_000.0, 0.0);
        self.columns_share = fit(self.columns_share, 0.0, 0.8, 0.0);
        self.deck_height = fit(self.deck_height, 0.0, 10_000.0, 0.0);
        self.column_widths.retain(|(_, w)| w.is_finite() && *w > 0.0 && *w < 5_000.0);
        self.fx_slots.truncate(fw_engine::FX_SLOTS);
        self.fx_volume = fit(self.fx_volume, 0.0, 1.0, d.fx_volume);
        self.spin_up_secs = fit(self.spin_up_secs, 0.1, 2.0, d.spin_up_secs);
        self.spin_down_secs = fit(self.spin_down_secs, 0.2, 4.0, d.spin_down_secs);
        self.fade_secs = fit(self.fade_secs, 1.0, 20.0, d.fade_secs);
        if self.curve > 3 {
            self.curve = d.curve;
        }
        for p in &mut self.curve_points {
            *p = fit(*p, 0.0, 1.0, 0.5);
        }
        self.master_volume = fit(self.master_volume, 0.0, 1.0, d.master_volume);
        self.head_volume = fit(self.head_volume, 0.0, 1.0, d.head_volume);
        self.head_mix = fit(self.head_mix, 0.0, 1.0, d.head_mix);
        for db in self.eq_db.iter_mut().flatten() {
            *db = fit(*db, EQ_MIN_DB, EQ_MAX_DB, 0.0);
        }
        self.ui_zoom = fit(self.ui_zoom, 0.7, 1.5, 1.0);
        if self.buffer_frames.is_some_and(|b| !(16..=16_384).contains(&b)) {
            self.buffer_frames = None;
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
    fn garbage_files_give_errors_not_panics() {
        for text in ["", "null", "[]", "{", "\"text\"", "42", "{\"language\":", "\u{0}\u{1}"] {
            assert!(Settings::from_json(text).is_err(), "{text:?} sollte ein Fehler sein");
        }
    }

    #[test]
    fn one_unknown_value_does_not_reset_everything() {
        // Etwa eine ältere Version, die Einstellungen einer neueren liest
        let s = Settings::from_json(r#"{"skin":"Plasma","fx_volume":0.3,"curve_points":"kaputt","folders":["/Musik"]}"#).unwrap();
        assert_eq!(s.skin, Settings::default().skin);
        assert_eq!(s.fx_volume, 0.3);
        assert_eq!(s.curve_points, Settings::default().curve_points);
        assert_eq!(s.folders, [PathBuf::from("/Musik")]);
    }

    #[test]
    fn nonsense_numbers_are_limited() {
        let s = Settings::from_json(
            r#"{"ui_zoom":40,"fade_secs":-3,"master_volume":7,"eq_db":[[1e30,-99,0],[5,5,5]],"curve":9,
                "buffer_frames":0,"tree_width":-20,"column_widths":[["artist",-5],["album",120]],"spin_up_secs":1e39}"#,
        )
        .unwrap();
        assert_eq!(s.ui_zoom, 1.5);
        assert_eq!(s.fade_secs, 1.0);
        assert_eq!(s.master_volume, 1.0);
        assert_eq!(s.eq_db[0], [EQ_MAX_DB, EQ_MIN_DB, 0.0]);
        assert_eq!(s.curve, 0);
        assert_eq!(s.buffer_frames, None);
        assert_eq!(s.tree_width, 0.0);
        assert_eq!(s.column_widths, [("album".to_owned(), 120.0)]);
        assert!(s.spin_up_secs > 0.0 && s.spin_up_secs <= 2.0);
    }

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"language":"En","neu_in_spaeterer_version":1}"#).unwrap();
        assert_eq!(s.language, Lang::En);
        assert_eq!(s.fade_secs, 8.0);
        let round: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(round, s);
    }
}
