// SPDX-License-Identifier: GPL-3.0-or-later
//! Zentrale App-Logik: verbindet Oberfläche, Engine und Bibliothek.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, pos2, vec2};
use egui_extras::{Size, StripBuilder};
use fw_engine::{AutoFade, Command, Controller, Curve, HeadMode, Motor, PREVIEW, TrackAudio, db_to_gain, eq};
use fw_library::{Library, Response, TrackMeta, Waveform};

use crate::audio::{self, AudioOut, HeadOut};
use crate::browser::{Browser, BrowserAction, BrowserCtx};
use crate::deck_ui::{self, DeckAction, DeckModel, PITCH_RANGE, Repeat};
use crate::devtools::DevTools;
use crate::i18n::{self, tr, trf};
use crate::history::{Entry, ExportFormat, History, PlayTracker};
use crate::history_ui::{DialogEvent, HistoryDialog};
use crate::settings::Settings;
use crate::settings_ui::{SettingsAction, SettingsCtx, SettingsUi};
use crate::tag_editor::{TagEditor, TagEditorEvent};
use crate::meter::Meter;
use crate::mixer_ui::{self, FadeAction, FadeModel, MixerAction, MixerModel};
use crate::playlist::{DragItem, Playlist};
use crate::playlist_ui::{self, PlaylistAction};
use crate::theme;

/// Höhe der Player-Zeile bei Vergrößerung 1,0.
const DECK_ROW_HEIGHT: f32 = 420.0;
/// Kleinste Höhe der Player-Zeile (auf kleinen Bildschirmen schrumpfen die Player bis hierhin).
const DECK_ROW_MIN: f32 = 325.0;
/// So viel Höhe behält eine Playlist mindestens (Kopfzeile und gut vier Titel).
const PLAYLIST_MIN: f32 = 150.0;
/// Bereich des Zooms (Strg +/−) und Schrittweite.
const ZOOM_MIN: f32 = 0.7;
const ZOOM_MAX: f32 = 1.5;
const ZOOM_STEP: f32 = 0.1;
/// So viele Sekunden vor Titelende beginnt der Rahmen eines laufenden Players zu blinken.
const END_WARNING_SECS: f64 = 15.0;
/// In den letzten Sekunden blinkt er doppelt so schnell.
const END_WARNING_FAST_SECS: f64 = 5.0;
/// So schnell (Prozent Pitch pro Sekunde) läuft Auto-Sync nach der Überblendung auf 0 % zurück, unhörbar langsam.
const AUTO_SYNC_RATE: f32 = 0.1;
/// Breite der Mittelspalte: wächst mit dem Fenster, damit die Player auf großen Monitoren nicht ausufern.
fn center_width(total: f32) -> f32 {
    (total * 0.2).clamp(250.0, 440.0)
}

struct Pending {
    token: u64,
    index: Option<usize>,
    autoplay: bool,
    keep_playing: bool,
}

struct DeckView {
    path: Option<PathBuf>,
    /// Der geladene Titel, damit er nach einem Gerätewechsel an die neue Engine geht.
    audio: Option<Arc<TrackAudio>>,
    meta: Option<TrackMeta>,
    waveform: Option<Arc<Waveform>>,
    bpm: Option<f32>,
    /// Eingebettetes Cover als Textur für den Plattenteller.
    cover: Option<egui::TextureHandle>,
    /// Kennung des Titels in der Engine, 0 = leer.
    loaded_id: u64,
    pending: Option<Pending>,
    pitch_pct: f32,
    /// Auto-Sync steuert den Pitch: Tempofaktor gegenüber dem anderen Titel (1, 2 oder ½).
    sync: Option<f32>,
    /// Lief im letzten Bild (für den Start-Zeitpunkt von Auto-Sync).
    was_playing: bool,
    pfl: bool,
    volume: f32,
    repeat: Repeat,
    loop_beats: Option<u32>,
    loop_range: Option<(f64, f64)>,
    scratching: bool,
    scratch_pos: f64,
    ended_seen: u64,
}

impl Default for DeckView {
    fn default() -> Self {
        Self {
            path: None,
            audio: None,
            meta: None,
            waveform: None,
            bpm: None,
            cover: None,
            loaded_id: 0,
            pending: None,
            pitch_pct: 0.0,
            sync: None,
            was_playing: false,
            pfl: false,
            volume: crate::settings::DEFAULT_CHANNEL_VOLUME,
            repeat: Repeat::Off,
            loop_beats: None,
            loop_range: None,
            scratching: false,
            scratch_pos: 0.0,
            ended_seen: 0,
        }
    }
}

struct PreviewView {
    path: PathBuf,
    token: u64,
    ended_seen: u64,
}

pub struct FadeWerkDjApp {
    audio: Option<AudioOut>,
    audio_error: Option<String>,
    head_audio: Option<HeadOut>,
    ctl: Controller,
    lib: Library,
    decks: [DeckView; 2],
    playlists: [Playlist; 2],
    browser: Browser,
    metas: HashMap<PathBuf, TrackMeta>,
    meta_requested: HashSet<PathBuf>,
    bpms: HashMap<PathBuf, f32>,
    /// Gemerkte BPM aus der Tempo-Erkennung (`bpm.json`).
    bpm_cache: crate::bpm_cache::BpmCache,
    /// Titel, deren BPM-Analyse noch läuft, und Gesamtzahl seit dem letzten Leerlauf (für die Fortschrittsanzeige).
    bpm_pending: HashSet<PathBuf>,
    bpm_total: usize,
    /// Ergebnis der Hintergrundprüfung des BPM-Caches beim Start (welche Dateien es noch gibt).
    bpm_check: Option<std::sync::mpsc::Receiver<Vec<(PathBuf, bool)>>>,
    autofade: AutoFade,
    crossfader: f32,
    curve: Curve,
    motor: Motor,
    head_mode: HeadMode,
    head_mix: f32,
    head_volume: f32,
    master_volume: f32,
    /// EQ je Player in dB und Kill-Tasten (Bässe, Mitten, Höhen).
    eq_db: [[f32; 3]; 2],
    eq_kill: [[bool; 3]; 2],
    /// Zuletzt an die Engine geschickte EQ-Faktoren; NaN erzwingt erneutes Senden.
    eq_sent: [[f32; 3]; 2],
    bass_swap: bool,
    auto_sync: bool,
    meters: [Meter; 3],
    /// Frequenzbänder des Masters für MAIN.
    bands: crate::meter::Bands,
    preview: Option<PreviewView>,
    next_token: u64,
    notice: Option<(String, f64)>,
    last_time: f64,
    dev: DevTools,
    settings: Settings,
    settings_ui: SettingsUi,
    tag_editor: Option<TagEditor>,
    save_due: Option<f64>,
    /// Vollbild beim nächsten Bild umschalten (Taste im Browser).
    toggle_fullscreen: bool,
    /// Einstellungen und Playlists speichern? Bei Entwicklerläufen nicht.
    persist: bool,
    session_saved: u64,
    session_checked: f64,
    /// Fensterhöhe und Browserhöhe im letzten Bild: ändert sich nur der Browser, hat der Nutzer ihn gezogen.
    last_heights: (f32, f32),
    update_rx: Option<std::sync::mpsc::Receiver<Option<crate::update::NewVersion>>>,
    /// Gefundene neue Version (Hinweisleiste), bis der Nutzer sie wegklickt.
    new_version: Option<crate::update::NewVersion>,
    /// Frage „nach Updates suchen?“ beim ersten Start.
    ask_update: bool,
    /// Gespielte Titel je Tag (`historie/`).
    history: History,
    /// Erkennt je Player, wann ein Titel als gespielt gilt.
    play_trackers: [PlayTracker; 2],
    /// Titelenden je Player, die die Historie schon gesehen hat (danach zählt ein Titel erneut).
    history_ended: [u64; 2],
    history_clock: std::time::Instant,
    /// Laufender Import einer Historie oder Playlist.
    import_rx: Option<std::sync::mpsc::Receiver<crate::history::ImportResult>>,
    history_dialog: Option<HistoryDialog>,
    /// Zum Neuzeichnen aus Hintergrund-Threads.
    repaint_ctx: egui::Context,
}

impl FadeWerkDjApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: Settings, start_dir: Option<PathBuf>) -> Self {
        i18n::set_lang(settings.language);
        theme::set_skin(settings.skin);
        theme::set_brushed(settings.brushed_steel);
        // Für die Fehlersuche: welche Grafikkarte und welcher Treiber tatsächlich zeichnen
        if let Some(rs) = &cc.wgpu_render_state {
            let a = rs.adapter.get_info();
            log::info!("wgpu: {} ({:?}, {:?}, Treiber {} {})", a.name, a.backend, a.device_type, a.driver, a.driver_info);
        }
        if let Some(gl) = &cc.gl {
            use eframe::glow::HasContext as _;
            // SAFETY: nur Abfragen, der Kontext ist während des Erzeugens der App aktiv
            let (renderer, version) = unsafe { (gl.get_parameter_string(eframe::glow::RENDERER), gl.get_parameter_string(eframe::glow::VERSION)) };
            log::info!("OpenGL: {renderer} ({version})");
        }
        crate::fonts::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        // Zoom selbst behandeln: begrenzt und gespeichert
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        cc.egui_ctx.set_zoom_factor(settings.ui_zoom.clamp(ZOOM_MIN, ZOOM_MAX));
        let (audio, ctl, audio_error) = audio::start(settings.output_device.as_deref(), settings.buffer_frames);
        let ctx = cc.egui_ctx.clone();
        let lib = Library::start(Arc::new(move || ctx.request_repaint()));
        let mut app = Self {
            audio,
            head_audio: None,
            audio_error: audio_error.clone(),
            ctl,
            lib,
            decks: Default::default(),
            playlists: Default::default(),
            browser: {
                let mut b = Browser::new(start_dir, &settings.folders, settings.tree_width, &settings.folder_recursion);
                b.set_columns(&settings.track_columns, &settings.column_widths);
                b.set_search_memory(settings.remember_search, &settings.advanced_search);
                b
            },
            metas: HashMap::new(),
            meta_requested: HashSet::new(),
            bpms: HashMap::new(),
            bpm_cache: Default::default(),
            bpm_pending: HashSet::new(),
            bpm_total: 0,
            bpm_check: None,
            autofade: { let mut a = AutoFade::default(); a.enabled = settings.auto_fade; a.fade_secs = settings.fade_secs; a },
            crossfader: -1.0,
            curve: curve_from(settings.curve, settings.curve_points),
            motor: Motor { spin_up_secs: settings.spin_up_secs, spin_down_secs: settings.spin_down_secs },
            head_mode: HeadMode::Off,
            head_mix: settings.head_mix,
            head_volume: settings.head_volume,
            master_volume: settings.master_volume,
            eq_db: settings.eq_db,
            eq_kill: [[false; 3]; 2],
            eq_sent: [[f32::NAN; 3]; 2],
            bass_swap: settings.bass_swap,
            auto_sync: settings.auto_sync || std::env::var_os("FW_AUTOSYNC").is_some(),
            meters: Default::default(),
            bands: Default::default(),
            preview: None,
            next_token: 1,
            notice: None,
            last_time: 0.0,
            dev: DevTools::from_env(),
            settings,
            settings_ui: SettingsUi::default(),
            tag_editor: None,
            save_due: None,
            toggle_fullscreen: false,
            persist: true,
            session_saved: 0,
            session_checked: 0.0,
            last_heights: (0.0, 0.0),
            update_rx: None,
            new_version: None,
            ask_update: false,
            history: History::default(),
            play_trackers: Default::default(),
            history_ended: [0; 2],
            history_clock: std::time::Instant::now(),
            import_rx: None,
            history_dialog: None,
            repaint_ctx: cc.egui_ctx.clone(),
        };
        app.persist = !app.dev.isolated();
        // Update-Prüfung: beim ersten Start fragen, danach nur wenn erlaubt (nicht bei Entwicklerläufen,
        // außer mit FW_UPDATE_URL; FW_UPDATE_ASK=1 zeigt die Frage)
        let dev_update = cfg!(debug_assertions) && std::env::var_os("FW_UPDATE_URL").is_some();
        let dev_ask = cfg!(debug_assertions) && std::env::var_os("FW_UPDATE_ASK").is_some();
        match app.settings.update_check {
            _ if dev_ask => app.ask_update = true,
            _ if dev_update => app.update_rx = Some(crate::update::check_in_background(cc.egui_ctx.clone())),
            None if app.persist => app.ask_update = true,
            Some(true) if app.persist => app.update_rx = Some(crate::update::check_in_background(cc.egui_ctx.clone())),
            _ => {}
        }
        if app.persist && app.settings.fullscreen {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        if app.persist {
            app.history = History::load();
        }
        if app.persist {
            app.bpm_cache = crate::bpm_cache::BpmCache::load();
            app.bpms.extend(app.bpm_cache.all().map(|(p, b)| (p.clone(), b)));
            app.bpm_check = Some(app.bpm_cache.start_check());
        }
        if app.persist {
            // Playlists vom letzten Mal wiederherstellen und die geladenen Titel wieder in die Player legen
            let (lists, missing) = crate::session::load();
            app.playlists = lists;
            app.session_saved = crate::session::fingerprint(&app.playlists);
            for d in 0..2 {
                // Zuletzt geladenen Titel wieder bereitlegen, sonst den ersten der Playlist
                let start = app.playlists[d].current.or((!app.playlists[d].entries.is_empty()).then_some(0));
                if let Some(i) = start {
                    app.load_index(d, i, false, false);
                }
            }
            if missing > 0 {
                // Bereinigte Liste gleich zurückschreiben
                app.session_saved = 0;
                app.notify(trf("{} Titel aus den Playlists gibt es nicht mehr", &[&missing]), 0.0);
            }
        }
        app.push_mixer_state();
        app.start_head_audio(0.0);
        if let Some(s) = app.dev.skin {
            theme::set_skin(s);
            theme::apply(&cc.egui_ctx);
        }
        if app.dev.steel {
            theme::set_brushed(true);
            theme::apply(&cc.egui_ctx);
        }
        if let Some(l) = app.dev.lang {
            i18n::set_lang(l);
        }
        if let Some(page) = app.dev.settings_tab.clone() {
            app.settings_ui.open_at(&page);
        }
        if let Some(e) = audio_error {
            app.notify(e, 0.0);
        }
        if app.dev.demo {
            // Für die Demo direkt suchen, die Hintergrundsuche läuft erst mit dem ersten Bild an
            let mut tracks = Vec::new();
            if let Some(dir) = start_dir_for_demo(&app) {
                fw_library::walk_audio(&dir, || true, |t| tracks.extend(t));
            }
            if let Some((first, rest)) = tracks.split_first() {
                app.playlists[0].push([first.clone()]);
                app.playlists[1].push(rest.iter().cloned());
                app.load_index(0, 0, true, false);
                app.load_index(1, 0, false, false);
            }
            // FW_SELECT=1: im Browser den 2. bis 4. Titel auswählen, um die Auswahl zu prüfen
            app.browser.demo_select = std::env::var_os("FW_SELECT").is_some();
            // FW_BPM=1: fehlende BPM des Startordners ermitteln
            app.browser.demo_bpm = std::env::var_os("FW_BPM").is_some();
            // FW_SEARCH="genre=Rock,year=1980-1989": erweiterte Suche vorbelegt öffnen
            if let Ok(spec) = std::env::var("FW_SEARCH") {
                app.browser.demo_search(&spec);
            }
            // FW_COLUMNS=album,genre,year: andere Spalten der Titelliste zeigen
            if let Ok(cols) = std::env::var("FW_COLUMNS") {
                let keys: Vec<String> = cols.split(',').map(|k| k.trim().to_owned()).collect();
                app.browser.set_columns(&keys, &[]);
            }
            // FW_TAGS=1: Fenster „Titelinfo & Tags“ für den ersten Titel öffnen
            if std::env::var_os("FW_TAGS").is_some()
                && let Some(t) = tracks.first()
            {
                app.tag_editor = Some(TagEditor::open(t.clone()));
            }
            // FW_HISTORY=1: Beispiel-Historie aus den Demo-Titeln (nicht gespeichert), Tag geöffnet
            if std::env::var_os("FW_HISTORY").is_some() {
                app.demo_history(&tracks);
            }
            // FW_PREVIEW=1: den zweiten Titel vorhören, um die Vorhör-Anzeige zu prüfen
            if std::env::var_os("FW_PREVIEW").is_some()
                && let Some(t) = tracks.get(1)
            {
                app.browser_action(BrowserAction::Preview(t.clone()), 0.0);
            }
        }
        app
    }

    /// Bringt die Engine auf den Stand der Oberfläche (Start und nach einem Gerätewechsel).
    fn push_mixer_state(&mut self) {
        self.ctl.send(Command::SetCrossfader(self.crossfader));
        self.ctl.send(Command::SetCurve(self.curve));
        self.ctl.send(Command::SetRepeatFade(self.autofade.fade_secs));
        self.ctl.send(Command::SetMotor(self.motor));
        self.ctl.send(Command::SetMasterVolume(self.master_volume));
        self.ctl.send(Command::SetHeadVolume(self.head_volume));
        self.ctl.send(Command::SetHeadMix(self.head_mix));
        self.ctl.send(Command::SetHeadMode(self.head_mode));
        for d in 0..2 {
            let v = &self.decks[d];
            self.ctl.send(Command::SetChannelVolume { deck: d, volume: v.volume });
            self.ctl.send(Command::SetPfl { deck: d, on: v.pfl });
            self.ctl.send(Command::SetPitch { deck: d, pitch: v.pitch_pct / 100.0 });
            self.ctl.send(Command::SetRepeatTrack { deck: d, on: v.repeat == Repeat::Track });
        }
        self.eq_sent = [[f32::NAN; 3]; 2];
        self.push_eq();
    }

    /// Schickt geänderte EQ-Werte an die Engine. Mit Bass-Swap sind während einer Überblendung erst
    /// die Bässe des neuen Titels aus, ab der Mitte die des alten.
    fn push_eq(&mut self) {
        let mut swap = [1.0f32; 2];
        if self.bass_swap
            && let Some(f) = self.autofade.fade()
            && f.is_handover()
        {
            let late = f.progress() >= 0.5;
            swap[f.to] = if late { 1.0 } else { 0.0 };
            swap[f.from] = if late { 0.0 } else { 1.0 };
        }
        for (d, swap) in swap.into_iter().enumerate() {
            for band in 0..3 {
                let mut gain = if self.eq_kill[d][band] { 0.0 } else { db_to_gain(self.eq_db[d][band]) };
                if band == eq::LOW {
                    gain *= swap;
                }
                if gain != self.eq_sent[d][band] {
                    self.eq_sent[d][band] = gain;
                    self.ctl.send(Command::SetEq { deck: d, band, gain });
                }
            }
        }
    }

    /// Öffnet das eigene Kopfhörergerät aus den Einstellungen (falls gewählt) und verbindet es mit der Engine.
    fn start_head_audio(&mut self, now: f64) {
        self.head_audio = None;
        self.ctl.send(Command::SetHeadSink(None));
        let Some(id) = self.settings.head_device.clone() else {
            if self.head_mode == HeadMode::Device {
                self.set_head_mode(HeadMode::Split);
            }
            return;
        };
        let Some(master) = self.audio.as_ref() else { return };
        if master.id == id {
            self.notify(tr("Das Kopfhörergerät ist dasselbe wie die Lautsprecher. Bitte ein anderes wählen oder im Mixer Split L/R nutzen."), now);
            return;
        }
        match audio::start_head(&id, master.sample_rate, self.settings.buffer_frames) {
            Ok((out, sink)) => {
                self.ctl.send(Command::SetHeadSink(Some(sink)));
                self.head_audio = Some(out);
                // Wer ein Kopfhörergerät wählt, will es auch hören
                if matches!(self.head_mode, HeadMode::Off | HeadMode::Split | HeadMode::Outputs34) {
                    self.set_head_mode(HeadMode::Device);
                }
            }
            Err(e) => {
                log::warn!("Kopfhörergerät: {e:#}");
                self.notify(trf("Kopfhörergerät nicht verfügbar: {}", &[&format!("{e:#}")]), now);
                if self.head_mode == HeadMode::Device {
                    self.set_head_mode(HeadMode::Split);
                }
            }
        }
    }

    /// Modus für Vorhören, wenn der Kopfhörer noch aus ist: eigenes Gerät, sonst Split.
    fn enable_headphones(&mut self, now: f64) {
        if self.head_mode != HeadMode::Off {
            return;
        }
        if self.head_audio.is_some() {
            self.set_head_mode(HeadMode::Device);
            self.notify(tr("Vorhören über das Kopfhörergerät"), now);
        } else {
            self.set_head_mode(HeadMode::Split);
            self.notify(tr("Kopfhörer: links Master, rechts Vorhören"), now);
        }
    }

    /// Startet die Audio-Ausgabe mit den aktuellen Einstellungen neu. Geladene Titel laufen an derselben Stelle weiter.
    fn restart_audio(&mut self, now: f64) {
        let st: Vec<(f64, bool)> = (0..2).map(|d| (self.ctl.shared.decks[d].position_secs(), self.ctl.shared.decks[d].is_playing())).collect();
        // Alte Geräte zuerst freigeben, manche Treiber erlauben nur einen Stream
        self.head_audio = None;
        self.audio = None;
        let (audio, ctl, err) = audio::start(self.settings.output_device.as_deref(), self.settings.buffer_frames);
        self.audio = audio;
        self.ctl = ctl;
        self.audio_error = err.clone();
        self.push_mixer_state();
        self.start_head_audio(now);
        for (d, &(pos, playing)) in st.iter().enumerate() {
            self.decks[d].ended_seen = 0;
            if let Some(track) = self.decks[d].audio.clone() {
                let id = self.decks[d].loaded_id;
                self.ctl.send(Command::Load { deck: d, id, track, keep_playing: false, start_secs: pos });
                let range = self.decks[d].loop_range;
                self.ctl.send(Command::SetLoop { deck: d, range });
                if playing {
                    self.ctl.send(Command::Play { deck: d });
                }
            }
        }
        self.preview = None;
        match err {
            Some(e) => self.notify(e, now),
            None => {
                let name = self.audio.as_ref().map_or(String::new(), |a| a.device.clone());
                self.notify(trf("Audiogerät gewechselt: {}", &[&name]), now);
            }
        }
    }

    fn settings_action(&mut self, a: SettingsAction, ctx: &egui::Context, now: f64) {
        match a {
            SettingsAction::Device(id) => {
                if self.settings.output_device != id {
                    self.settings.output_device = id;
                    self.restart_audio(now);
                }
            }
            SettingsAction::HeadDevice(id) => {
                if self.settings.head_device != id {
                    self.settings.head_device = id;
                    self.start_head_audio(now);
                }
            }
            SettingsAction::Buffer(b) => {
                if self.settings.buffer_frames != b {
                    self.settings.buffer_frames = b;
                    self.restart_audio(now);
                }
            }
            SettingsAction::Language(l) => {
                self.settings.language = l;
                i18n::set_lang(l);
            }
            SettingsAction::Skin(s) => {
                self.settings.skin = s;
                theme::set_skin(s);
                theme::apply(ctx);
            }
            SettingsAction::Brushed(on) => {
                self.settings.brushed_steel = on;
                theme::set_brushed(on);
                theme::apply(ctx);
            }
            SettingsAction::Zoom(z) => self.set_zoom(ctx, z),
            // Wirkt erst beim nächsten Start
            SettingsAction::OpenGl(on) => self.settings.opengl = on,
            SettingsAction::RecordHistory(on) => self.settings.record_history = on,
            SettingsAction::UpdateCheck(on) => {
                self.settings.update_check = Some(on);
                self.ask_update = false;
            }
            SettingsAction::RevealLog => {
                if let Some(path) = crate::diagnose::log_path()
                    && let Err(e) = reveal(&path)
                {
                    self.notify(trf("Logdatei nicht gefunden: {}", &[&e]), now);
                }
            }
        }
        self.save_settings();
    }

    /// Übernimmt geänderte Werte in die Einstellungen und speichert verzögert (Schieberegler ändern sich laufend).
    fn track_settings(&mut self, now: f64) {
        let mut s = self.settings.clone();
        s.spin_up_secs = self.motor.spin_up_secs;
        s.spin_down_secs = self.motor.spin_down_secs;
        s.fade_secs = self.autofade.fade_secs;
        s.auto_fade = self.autofade.enabled;
        s.curve = match self.curve {
            Curve::Soft => 0,
            Curve::Linear => 1,
            Curve::Cut => 2,
            Curve::Custom(pts) => {
                s.curve_points = pts;
                3
            }
        };
        s.master_volume = self.master_volume;
        s.head_volume = self.head_volume;
        s.head_mix = self.head_mix;
        s.eq_db = self.eq_db;
        s.bass_swap = self.bass_swap;
        s.auto_sync = self.auto_sync;
        if s != self.settings {
            self.settings = s;
            self.save_due = Some(now + 1.0);
        }
        if self.save_due.is_some_and(|t| now >= t) {
            self.save_settings();
            self.save_due = None;
        }
    }

    /// Zoom der ganzen Oberfläche setzen (begrenzt, auf 5 % gerundet) und merken.
    fn set_zoom(&mut self, ctx: &egui::Context, z: f32) {
        let z = ((z * 20.0).round() / 20.0).clamp(ZOOM_MIN, ZOOM_MAX);
        ctx.set_zoom_factor(z);
        self.settings.ui_zoom = z;
        self.save_due = Some(ctx.input(|i| i.time) + 1.0);
    }

    /// Schwebende Leiste oben in der Mitte: einmalige Frage, ob nach Updates gesucht werden darf,
    /// bzw. Hinweis auf eine neue Version. Schwebt über der Oberfläche, damit sich nichts verschiebt.
    fn update_ui(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.update_rx
            && let Ok(result) = rx.try_recv()
        {
            self.new_version = result;
            self.update_rx = None;
        }
        if !self.ask_update && self.new_version.is_none() {
            return;
        }
        let font = crate::fonts::bold(13.0);
        let mut answer = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("update")).anchor(Align2::CENTER_TOP, vec2(0.0, 12.0)).show(ctx, |ui| {
            Frame::new()
                .fill(theme::panel2())
                .stroke(Stroke::new(1.0, theme::line()))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if self.ask_update {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(tr("Beim Start nach neuen Versionen suchen?")).strong().color(theme::ink()));
                                ui.label(RichText::new(tr("Dabei fragt FadeWerk DJ nur bei GitHub nach der neuesten Version – ohne Daten über deine Musik.")).size(11.5).color(theme::dim()));
                            });
                            ui.add_space(10.0);
                            if crate::hw::button(ui, vec2(100.0, 28.0), tr("Ja, prüfen"), font.clone(), crate::hw::Look::Color(theme::play()), false, true).clicked() {
                                answer = Some(true);
                            }
                            if crate::hw::button(ui, vec2(70.0, 28.0), tr("Nein"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                                answer = Some(false);
                            }
                        } else if let Some(v) = &self.new_version {
                            ui.label(RichText::new(trf("FadeWerk DJ {} ist verfügbar (installiert: {}).", &[&v.version, &env!("CARGO_PKG_VERSION")])).strong().color(theme::ink()));
                            ui.add_space(10.0);
                            if crate::hw::button(ui, vec2(190.0, 28.0), tr("Download-Seite öffnen"), font.clone(), crate::hw::Look::Color(theme::play()), false, true).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(&v.url));
                                close = true;
                            }
                            if crate::hw::button(ui, vec2(80.0, 28.0), tr("Später"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                                close = true;
                            }
                        }
                    });
                });
        });
        if let Some(on) = answer {
            self.settings.update_check = Some(on);
            self.ask_update = false;
            self.save_settings();
            if on {
                self.update_rx = Some(crate::update::check_in_background(ctx.clone()));
            }
        }
        if close {
            self.new_version = None;
        }
    }

    fn save_settings(&self) {
        if self.persist {
            self.settings.save();
        }
    }

    /// Speichert die Playlists, wenn sie sich seit dem letzten Mal geändert haben.
    fn save_session_if_changed(&mut self) {
        if !self.persist {
            return;
        }
        let fp = crate::session::fingerprint(&self.playlists);
        if fp != self.session_saved {
            crate::session::save(&self.playlists);
            self.session_saved = fp;
        }
    }

    /// Gemerkte BPM für einen Titel prüfen: geänderte Datei → verwerfen, umgezogene Datei → übernehmen.
    fn resolve_bpm(&mut self, path: &Path) {
        match self.bpm_cache.resolve(path) {
            crate::bpm_cache::Resolved::Bpm(b) => {
                self.bpms.insert(path.to_path_buf(), b);
            }
            crate::bpm_cache::Resolved::Stale => {
                self.bpms.remove(path);
            }
            crate::bpm_cache::Resolved::Unknown => {}
        }
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.notice = Some((text.into(), now + 3.0));
    }

    fn has_track(&self, d: usize) -> bool {
        self.decks[d].loaded_id != 0
    }

    fn request_meta(&mut self, path: PathBuf) {
        if !self.metas.contains_key(&path) && self.meta_requested.insert(path.clone()) {
            self.lib.request_meta(path);
        }
    }

    /// Historie: Titel eintragen, sobald sie lange genug über Master zu hören waren.
    fn record_plays(&mut self) {
        let dt = self.history_clock.elapsed().as_secs_f64();
        self.history_clock = std::time::Instant::now();
        for d in 0..2 {
            let st = &self.ctl.shared.decks[d];
            // Zu Ende gelaufen: spielt der Titel noch einmal, zählt er erneut
            let ended = st.ended_count();
            if ended != self.history_ended[d] {
                self.history_ended[d] = ended;
                self.play_trackers[d].restart();
            }
            let id = if self.settings.record_history { self.decks[d].loaded_id } else { 0 };
            let audible = self.on_air(d) && st.is_playing() && self.master_volume > 0.02;
            let pos = st.position_secs();
            let Some(at) = self.play_trackers[d].update(id, audible, pos, dt, crate::history::now_local) else { continue };
            let v = &self.decks[d];
            let Some(path) = v.path.clone() else { continue };
            let meta = v.meta.clone().unwrap_or_else(|| TrackMeta::from_filename(&path));
            let entry = Entry {
                size: std::fs::metadata(&path).ok().map(|m| m.len()),
                path,
                title: meta.title,
                artist: meta.artist,
                album: meta.album,
                duration_secs: meta.duration_secs,
                played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                deck: Some(d as u8),
            };
            log::info!("Historie: {} – {}", entry.artist, entry.title);
            self.history.record(entry);
        }
    }

    /// Entwicklerschalter `FW_HISTORY`: zwei Beispieltage aus den Demo-Titeln, der neuere ist geöffnet.
    fn demo_history(&mut self, tracks: &[PathBuf]) {
        let today = crate::history::history_date(crate::history::now_local());
        // Ein Titel, dessen Datei fehlt (wird ausgegraut gezeigt)
        // Namen passend zur Sprache, damit englische Screenshots nichts Deutsches zeigen
        let en = i18n::lang() == i18n::Lang::En;
        let (artist, title, name) = if en { ("Unknown", "Lost Track", "Grandma's birthday") } else { ("Unbekannt", "Verschollen", "Omas Geburtstag") };
        let gone = PathBuf::from(format!("/gibt/es/nicht/{artist} - {title}.mp3"));
        let at = today.at(19, 56, 0, 0);
        self.history.record(Entry { path: gone, title: title.into(), artist: artist.into(), played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()), deck: Some(0), ..Default::default() });
        for (k, p) in tracks.iter().enumerate() {
            let meta = TrackMeta::from_filename(p);
            let day = if k < 3 { today.yesterday().unwrap_or(today) } else { today };
            let at = day.at(20, 0, 0, 0) + jiff::Span::new().minutes(4 * k as i64);
            self.history.record(Entry {
                path: p.clone(),
                title: meta.title,
                artist: meta.artist,
                duration_secs: 200.0,
                played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                deck: Some((k % 2) as u8),
                ..Default::default()
            });
        }
        self.history.rename(&today.to_string(), name);
        self.browser.open_history_day(today.to_string());
    }

    /// Füllt die Playlists wieder so, wie an diesem Tag gespielt wurde: Titel von Player A nach A, von B
    /// nach B, in der Reihenfolge von damals. Titel ohne Player-Angabe (importierte Playlists) werden
    /// abwechselnd verteilt, fehlende Dateien übersprungen. Ein laufender Titel bleibt, der Rest folgt dahinter.
    fn restore_history(&mut self, date: &str, now: f64) {
        let Some(day) = self.history.day(date) else { return };
        let mut lists: [Vec<PathBuf>; 2] = Default::default();
        let mut next = 0;
        for e in day.entries.iter().filter(|e| e.path.is_file()) {
            let d = e.deck.map(usize::from).filter(|d| *d < 2).unwrap_or_else(|| {
                next += 1;
                (next - 1) % 2
            });
            lists[d].push(e.path.clone());
        }
        let counts = [lists[0].len(), lists[1].len()];
        if counts == [0, 0] {
            self.notify(tr("Keine der Dateien dieses Tages ist zu finden"), now);
            return;
        }
        for (d, paths) in lists.into_iter().enumerate() {
            let playing = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
            self.playlists[d].clear_keep(playing);
            let first = self.playlists[d].entries.len();
            let any = !paths.is_empty();
            self.playlists[d].push(paths);
            // Steht der Player, gleich den ersten Titel bereitlegen
            if !playing && any {
                self.load_index(d, first, false, false);
            }
        }
        self.notify(trf("Wiederhergestellt: {} Titel in Playlist A, {} in B", &[&counts[0], &counts[1]]), now);
    }

    /// Speichert einen Tag in einem der Exportformate (Speicherort wählt der Nutzer).
    fn export_history(&mut self, date: &str, format: ExportFormat, now: f64) {
        let Some(day) = self.history.day(date) else { return };
        let (filter, text) = match format {
            ExportFormat::M3u => (tr("Playlist"), crate::history::to_m3u(day)),
            ExportFormat::Csv => {
                let sep = if i18n::lang() == i18n::Lang::De { ';' } else { ',' };
                let headers = [tr("Nr."), tr("Uhrzeit"), tr("Interpret"), tr("Titel"), tr("Album"), tr("Länge"), tr("Player"), tr("Datei")];
                (tr("Tabelle"), crate::history::to_csv(day, sep, headers))
            }
            ExportFormat::Json => (tr("FadeWerk-DJ-Historie"), crate::history::to_json(day)),
        };
        let ext = format.extension();
        let Some(file) = rfd::FileDialog::new()
            .set_title(tr("Historie exportieren"))
            .set_file_name(format!("{}.{ext}", day.file_stem()))
            .add_filter(filter, &[ext])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&file, text) {
            Ok(()) => self.notify(trf("Exportiert: {}", &[&file.file_name().unwrap_or_default().to_string_lossy()]), now),
            Err(e) => self.notify(trf("Export fehlgeschlagen: {}", &[&e]), now),
        }
    }

    /// Ordner, in denen beim Import fehlende Titel gesucht werden: eigene Ordner und der Musikordner.
    fn music_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.settings.folders.clone();
        if let Some(m) = dirs::audio_dir()
            && !roots.iter().any(|r| m.starts_with(r))
        {
            roots.push(m);
        }
        roots
    }

    fn poll_import(&mut self, now: f64) {
        let Some(rx) = &self.import_rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.import_rx = None;
        match result {
            Ok((crate::history::Imported::Day(day), found)) => {
                let date = day.date.clone();
                let n = day.entries.len();
                let added = self.history.merge(day);
                self.browser.open_history_day(date.clone());
                let mut text = if added == n { trf("Historie {} importiert: {} Titel", &[&date, &n]) } else { trf("Historie {}: {} neue Titel", &[&date, &added]) };
                if found > 0 {
                    text.push_str(&trf(" ({} wiedergefunden)", &[&found]));
                }
                self.notify(text, now);
            }
            Ok((crate::history::Imported::List { name, entries }, found)) => {
                let date = crate::history::history_date(crate::history::now_local()).to_string();
                self.history_dialog = Some(HistoryDialog::Import { entries, found, date, name });
            }
            Err(e) => self.notify(trf("Import fehlgeschlagen: {}", &[&e]), now),
        }
    }

    /// Ist der Player gerade über Master zu hören?
    fn on_air(&self, d: usize) -> bool {
        let st = &self.ctl.shared.decks[d];
        let (ga, gb) = self.curve.gains(self.crossfader);
        let g = if d == 0 { ga } else { gb };
        self.has_track(d) && (st.is_playing() || st.speed() > 0.0) && g * self.decks[d].volume > 0.05
    }

    fn set_crossfader(&mut self, x: f32) {
        self.crossfader = x.clamp(-1.0, 1.0);
        self.ctl.send(Command::SetCrossfader(self.crossfader));
    }

    fn set_head_mode(&mut self, mode: HeadMode) {
        self.head_mode = mode;
        self.ctl.send(Command::SetHeadMode(mode));
    }

    // ---------- Laden ----------

    fn load_index(&mut self, d: usize, i: usize, autoplay: bool, keep_playing: bool) -> bool {
        let Some(path) = self.playlists[d].entries.get(i).cloned() else { return false };
        let token = self.token();
        self.decks[d].pending = Some(Pending { token, index: Some(i), autoplay, keep_playing });
        self.lib.load(d, token, path);
        true
    }

    fn load_next(&mut self, d: usize, autoplay: bool, keep_playing: bool) -> bool {
        let wrap = self.decks[d].repeat == Repeat::List;
        match self.playlists[d].next_index(wrap) {
            Some(i) => self.load_index(d, i, autoplay, keep_playing),
            None => false,
        }
    }

    /// Lädt den ersten neuen Titel, wenn der Player leer ist.
    fn fill_if_empty(&mut self, d: usize, index: usize) {
        if !self.has_track(d) && self.decks[d].pending.is_none() {
            self.load_index(d, index, false, false);
        }
    }

    fn poll_library(&mut self, ctx: &egui::Context, now: f64) {
        let responses: Vec<Response> = self.lib.poll().collect();
        for resp in responses {
            match resp {
                Response::Meta(path, meta) => {
                    // Geladene Titel zeigen geänderte Tags sofort
                    for v in self.decks.iter_mut().filter(|v| v.path.as_ref() == Some(&path)) {
                        v.meta = Some(meta.clone());
                    }
                    self.resolve_bpm(&path);
                    self.metas.insert(path, meta);
                }
                Response::Bpm { path, bpm, analyzed } => {
                    self.bpm_pending.remove(&path);
                    if self.bpm_pending.is_empty() {
                        self.bpm_total = 0;
                    }
                    if let Some(bpm) = bpm {
                        if analyzed {
                            self.bpm_cache.insert(&path, bpm);
                        }
                        self.bpms.insert(path, bpm);
                    }
                }
                Response::Scan { token, tracks, done, limited } => self.browser.receive_scan(token, tracks, done, limited),
                Response::Loaded(l) => {
                    let l = *l;
                    self.metas.insert(l.path.clone(), l.meta.clone());
                    if let Some(bpm) = l.bpm {
                        self.bpms.insert(l.path.clone(), bpm);
                        if l.bpm_analyzed {
                            self.bpm_cache.insert(&l.path, bpm);
                        }
                    }
                    if l.deck == PREVIEW {
                        if self.preview.as_ref().is_some_and(|p| p.token == l.token) {
                            let start = l.audio.duration_secs() * 0.3;
                            self.ctl.send(Command::Load { deck: PREVIEW, id: l.token, track: l.audio, keep_playing: false, start_secs: start });
                            self.ctl.send(Command::Play { deck: PREVIEW });
                        }
                        continue;
                    }
                    let d = l.deck;
                    let Some(p) = self.decks[d].pending.take_if(|p| p.token == l.token) else { continue };
                    let audio = l.audio.clone();
                    self.ctl.send(Command::Load { deck: d, id: l.token, track: l.audio, keep_playing: p.keep_playing, start_secs: 0.0 });
                    if p.autoplay {
                        self.ctl.send(Command::Play { deck: d });
                    }
                    let v = &mut self.decks[d];
                    v.path = Some(l.path);
                    v.meta = Some(l.meta);
                    v.waveform = Some(Arc::new(l.waveform));
                    v.bpm = l.bpm;
                    v.cover = l.cover.map(|c| {
                        let img = egui::ColorImage::from_rgba_unmultiplied([c.size as usize; 2], &c.rgba);
                        ctx.load_texture(format!("cover-{d}"), img, egui::TextureOptions::LINEAR)
                    });
                    v.loaded_id = l.token;
                    v.audio = Some(audio);
                    v.loop_beats = None;
                    v.loop_range = None;
                    if let Some(i) = p.index {
                        self.playlists[d].current = Some(i);
                    }
                }
                Response::Failed { path, deck, error } => {
                    log::warn!("Laden fehlgeschlagen: {error}");
                    if let Some(d) = deck.filter(|d| *d < 2) {
                        self.decks[d].pending = None;
                    }
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.notify(trf("„{}“ lässt sich nicht abspielen", &[&name]), now);
                }
            }
        }
    }

    // ---------- Titelende und Auto-Fade ----------

    fn handle_track_ends(&mut self) {
        for d in 0..2 {
            let c = self.ctl.shared.decks[d].ended_count();
            if c != self.decks[d].ended_seen {
                self.decks[d].ended_seen = c;
                if self.autofade.fade().is_some_and(|f| f.from == d || f.to == d) {
                    continue;
                }
                let continuous = !self.autofade.enabled && self.autofade.active == d;
                self.load_next(d, continuous, false);
            }
        }
        if let Some(p) = &mut self.preview {
            let c = self.ctl.shared.decks[PREVIEW].ended_count();
            if p.ended_seen == 0 {
                p.ended_seen = c;
            } else if c != p.ended_seen {
                self.preview = None;
            }
        }
    }

    fn step_autofade(&mut self, dt: f32) {
        if let Some(step) = self.autofade.step(dt) {
            self.set_crossfader(step.crossfader);
            if let Some(old) = step.finished {
                self.ctl.send(Command::Pause { deck: old });
                self.load_next(old, false, false);
            }
            return;
        }
        let a = self.autofade.active;
        let st = &self.ctl.shared.decks[a];
        if !self.has_track(a) || self.decks[a].repeat == Repeat::Track || self.decks[a].loop_range.is_some() {
            return;
        }
        let remaining = (st.duration_secs() - st.position_secs()) / (1.0 + self.decks[a].pitch_pct as f64 / 100.0);
        if self.autofade.should_start(st.is_playing(), remaining) {
            self.start_fade(false, 0.0);
        }
    }

    fn start_fade(&mut self, manual: bool, now: f64) {
        if self.autofade.fade().is_some() {
            return;
        }
        let to = self.autofade.other();
        if self.has_track(to) && self.decks[to].pending.is_none() {
            self.ctl.send(Command::Play { deck: to });
            self.autofade.start(self.crossfader);
        } else if self.decks[to].pending.is_none() && !self.load_next(to, false, false) && manual {
            self.notify(trf("Playlist {} hat keinen nächsten Titel", &[&theme::deck_letter(to)]), now);
        }
    }

    /// Pitch, mit dem Player `d` im Tempo des anderen läuft, und der Tempofaktor dazu. Ohne `factor` wird der
    /// nächstliegende aus 1, 2 und ½ gewählt (70 und 140 BPM passen zusammen). `None` ohne BPM.
    fn sync_pitch(&self, d: usize, factor: Option<f32>) -> Option<(f32, f32)> {
        let o = 1 - d;
        let mine = self.decks[d].bpm?;
        let theirs = self.decks[o].bpm.filter(|_| self.has_track(o))? * (1.0 + self.decks[o].pitch_pct / 100.0);
        let pct = |f: f32| (theirs / (mine * f) - 1.0) * 100.0;
        let f = factor.unwrap_or_else(|| [1.0, 2.0, 0.5].into_iter().min_by(|a, b| pct(*a).abs().total_cmp(&pct(*b).abs())).unwrap_or(1.0));
        Some((pct(f), f))
    }

    fn send_pitch(&mut self, d: usize, pct: f32) {
        self.decks[d].pitch_pct = pct;
        self.ctl.send(Command::SetPitch { deck: d, pitch: pct / 100.0 });
    }

    /// Auto-Sync: startet ein Player, während der andere zu hören ist, übernimmt er dessen Tempo und hält es,
    /// solange der andere zu hören ist (also auch während der Überblendung). Ist er allein auf Sendung, läuft sein
    /// Pitch mit `AUTO_SYNC_RATE` auf 0 % zurück. Steht er still und ist nicht zu hören, geht der Pitch sofort auf 0.
    fn step_auto_sync(&mut self, dt: f32, now: f64) {
        for d in 0..2 {
            let o = 1 - d;
            let playing = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
            let started = playing && !self.decks[d].was_playing;
            self.decks[d].was_playing = playing;
            if !self.auto_sync {
                continue;
            }
            if started && self.on_air(o) {
                self.decks[d].sync = None;
                if let Some((pct, factor)) = self.sync_pitch(d, None) {
                    if pct.abs() <= PITCH_RANGE {
                        self.decks[d].sync = Some(factor);
                        self.send_pitch(d, pct);
                    } else {
                        let theirs = self.decks[o].bpm.unwrap_or(0.0) * (1.0 + self.decks[o].pitch_pct / 100.0);
                        let mine = self.decks[d].bpm.unwrap_or(0.0);
                        self.notify(trf("Auto-Sync: {} ↔ {} BPM liegen zu weit auseinander", &[&format!("{mine:.1}"), &format!("{theirs:.1}")]), now);
                    }
                }
            }
            let Some(factor) = self.decks[d].sync else { continue };
            if !playing {
                if !self.on_air(d) {
                    self.decks[d].sync = None;
                    self.send_pitch(d, 0.0);
                }
            } else if self.on_air(o) {
                // Dem anderen folgen, auch wenn dessen Pitch sich ändert
                if let Some((pct, _)) = self.sync_pitch(d, Some(factor))
                    && pct.abs() <= PITCH_RANGE
                    && pct != self.decks[d].pitch_pct
                {
                    self.send_pitch(d, pct);
                }
            } else {
                let p = self.decks[d].pitch_pct;
                let step = AUTO_SYNC_RATE * dt;
                if p.abs() <= step {
                    self.decks[d].sync = None;
                    self.send_pitch(d, 0.0);
                } else {
                    self.send_pitch(d, p - step * p.signum());
                }
            }
        }
    }

    fn fade_status(&self) -> String {
        if let Some(f) = self.autofade.fade() {
            return trf("Blende von {} auf {}, noch {} s", &[&theme::deck_letter(f.from), &theme::deck_letter(f.to), &format!("{:.1}", f.remaining_secs())]);
        }
        if !self.autofade.enabled {
            return tr("Auto-Fade aus, Crossfader per Hand").into();
        }
        let a = self.autofade.active;
        let st = &self.ctl.shared.decks[a];
        if !self.has_track(a) || !st.is_playing() {
            return trf("Player {} starten, dann blendet Auto-Fade selbst", &[&theme::deck_letter(a)]);
        }
        if self.decks[a].repeat == Repeat::Track || self.decks[a].loop_range.is_some() {
            return trf("Repeat oder Loop in {}: Auto-Fade wartet", &[&theme::deck_letter(a)]);
        }
        let rem = (st.duration_secs() - st.position_secs()) / (1.0 + self.decks[a].pitch_pct as f64 / 100.0) - self.autofade.fade_secs as f64;
        trf("Auto-Fade zu {} in {}", &[&theme::deck_letter(1 - a), &deck_ui::fmt_time(rem)])
    }

    // ---------- Aktionen ----------

    fn deck_action(&mut self, d: usize, a: DeckAction, now: f64) {
        let st_pos = self.ctl.shared.decks[d].position_secs();
        let st_dur = self.ctl.shared.decks[d].duration_secs();
        match a {
            DeckAction::TogglePlay => {
                let starting = !self.ctl.shared.decks[d].is_playing();
                self.ctl.send(Command::TogglePlay { deck: d });
                if starting && self.autofade.fade().is_none() {
                    let (ga, gb) = self.curve.gains(self.crossfader);
                    let gain = if d == 0 { ga } else { gb };
                    let other_idle = !self.ctl.shared.decks[1 - d].is_playing();
                    if self.autofade.enabled && other_idle && self.has_track(d) && self.crossfader != fw_engine::automix::side(d) {
                        // Der andere Player steht: Crossfader zu diesem hinüberblenden
                        self.autofade.glide_to(d, self.crossfader);
                    } else if gain > 0.5 {
                        self.autofade.active = d;
                    }
                }
            }
            DeckAction::Cue => self.ctl.send(Command::Cue { deck: d }),
            DeckAction::Stop => self.ctl.send(Command::Stop { deck: d }),
            DeckAction::Prev => match self.playlists[d].prev_index() {
                Some(i) if st_pos <= 3.0 && self.has_track(d) => {
                    self.load_index(d, i, false, true);
                }
                _ => self.ctl.send(Command::Seek { deck: d, secs: 0.0 }),
            },
            DeckAction::Next => {
                let wrap = self.decks[d].repeat == Repeat::List;
                match self.playlists[d].next_index(wrap) {
                    Some(i) => {
                        self.load_index(d, i, false, true);
                    }
                    None => self.notify(trf("Playlist {}: kein weiterer Titel", &[&theme::deck_letter(d)]), now),
                }
            }
            DeckAction::Eject => {
                if self.on_air(d) {
                    self.notify(trf("Player {} ist gerade zu hören. Erst stoppen oder wegblenden.", &[&theme::deck_letter(d)]), now);
                    return;
                }
                self.ctl.send(Command::Eject { deck: d });
                let v = &mut self.decks[d];
                v.path = None;
                v.meta = None;
                v.waveform = None;
                v.audio = None;
                v.bpm = None;
                v.cover = None;
                v.loaded_id = 0;
                v.pending = None;
                v.loop_beats = None;
                v.loop_range = None;
            }
            DeckAction::CycleRepeat => {
                let r = self.decks[d].repeat.next();
                self.decks[d].repeat = r;
                self.ctl.send(Command::SetRepeatTrack { deck: d, on: r == Repeat::Track });
            }
            DeckAction::Pfl(on) => {
                self.decks[d].pfl = on;
                self.ctl.send(Command::SetPfl { deck: d, on });
                if on {
                    self.enable_headphones(now);
                }
            }
            DeckAction::Sync => {
                self.decks[d].sync = None;
                let o = 1 - d;
                match (self.decks[d].bpm, self.decks[o].bpm.filter(|_| self.has_track(o))) {
                    (Some(mine), Some(theirs)) => {
                        let target = theirs * (1.0 + self.decks[o].pitch_pct / 100.0);
                        let pct = (target / mine - 1.0) * 100.0;
                        if pct.abs() > PITCH_RANGE {
                            self.notify(trf("Zu weit auseinander ({} ↔ {} BPM), Pitch reicht nur ±{} %", &[&format!("{mine:.1}"), &format!("{target:.1}"), &format!("{PITCH_RANGE:.0}")]), now);
                        } else {
                            let pct = (pct * 10.0).round() / 10.0;
                            self.decks[d].pitch_pct = pct;
                            self.ctl.send(Command::SetPitch { deck: d, pitch: pct / 100.0 });
                        }
                    }
                    _ => self.notify(tr("Für Sync brauchen beide Player einen Titel mit BPM"), now),
                }
            }
            DeckAction::Loop(beats) => {
                let v = &mut self.decks[d];
                if v.loop_beats == Some(beats) {
                    v.loop_beats = None;
                    v.loop_range = None;
                } else if let Some(bpm) = v.bpm {
                    let beat = 60.0 / (bpm as f64);
                    let range = (st_pos, (st_pos + beats as f64 * beat).min(st_dur));
                    v.loop_beats = Some(beats);
                    v.loop_range = Some(range);
                }
                let range = v.loop_range;
                self.ctl.send(Command::SetLoop { deck: d, range });
            }
            DeckAction::Pitch(p) => {
                self.decks[d].sync = None;
                self.decks[d].pitch_pct = p;
                self.ctl.send(Command::SetPitch { deck: d, pitch: p / 100.0 });
            }
            DeckAction::Seek(s) => self.ctl.send(Command::Seek { deck: d, secs: s }),
            DeckAction::ScratchBegin => {
                self.decks[d].scratching = true;
                self.decks[d].scratch_pos = st_pos;
                self.ctl.send(Command::ScratchBegin { deck: d });
            }
            DeckAction::ScratchBy(ds) => {
                if self.decks[d].scratching {
                    let p = (self.decks[d].scratch_pos + ds).clamp(0.0, st_dur);
                    self.decks[d].scratch_pos = p;
                    self.ctl.send(Command::ScratchTo { deck: d, secs: p });
                }
            }
            DeckAction::ScratchEnd => {
                self.decks[d].scratching = false;
                self.ctl.send(Command::ScratchEnd { deck: d });
            }
        }
    }

    fn mixer_action(&mut self, a: MixerAction, now: f64) {
        match a {
            MixerAction::Volume(d, v) => {
                self.decks[d].volume = v;
                self.ctl.send(Command::SetChannelVolume { deck: d, volume: v });
            }
            MixerAction::HeadMode(m) => {
                if m == HeadMode::Outputs34 && self.ctl.shared.channels < 4 {
                    self.notify(trf("Das Ausgabegerät hat nur {} Kanäle", &[&self.ctl.shared.channels]), now);
                    return;
                }
                self.set_head_mode(m);
            }
            MixerAction::HeadMix(v) => {
                self.head_mix = v;
                self.ctl.send(Command::SetHeadMix(v));
            }
            MixerAction::HeadVolume(v) => {
                self.head_volume = v;
                self.ctl.send(Command::SetHeadVolume(v));
            }
            MixerAction::MasterVolume(v) => {
                self.master_volume = v;
                self.ctl.send(Command::SetMasterVolume(v));
            }
            MixerAction::Eq { deck, band, db } => self.eq_db[deck][band] = db,
            MixerAction::EqKill { deck, band, on } => self.eq_kill[deck][band] = on,
        }
    }

    fn fade_action(&mut self, a: FadeAction, now: f64) {
        match a {
            FadeAction::Auto(on) => self.autofade.enabled = on,
            FadeAction::FadeSecs(s) => {
                self.autofade.fade_secs = s;
                self.ctl.send(Command::SetRepeatFade(s));
            }
            FadeAction::Curve(c) => {
                self.curve = c;
                self.ctl.send(Command::SetCurve(c));
            }
            FadeAction::Crossfader(x) => {
                self.autofade.cancel();
                self.set_crossfader(x);
                self.autofade.follow_crossfader(x);
            }
            FadeAction::FadeNow => self.start_fade(true, now),
            FadeAction::Motor(m) => {
                self.motor = m;
                self.ctl.send(Command::SetMotor(m));
            }
            FadeAction::BassSwap(on) => self.bass_swap = on,
            FadeAction::AutoSync(on) => {
                self.auto_sync = on;
                for v in &mut self.decks {
                    v.sync = None;
                }
            }
        }
    }

    fn playlist_action(&mut self, d: usize, a: PlaylistAction, now: f64) {
        match a {
            PlaylistAction::Load(i) => {
                if self.ctl.shared.decks[d].is_playing() {
                    self.notify(trf("Player {} spielt gerade. Erst stoppen.", &[&theme::deck_letter(d)]), now);
                } else {
                    self.load_index(d, i, false, false);
                }
            }
            PlaylistAction::Remove(i) => {
                self.playlists[d].remove(i);
            }
            PlaylistAction::Shuffle => {
                self.playlists[d].shuffle_upcoming(shuffle_seed());
            }
            PlaylistAction::Clear => {
                let keep = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
                self.playlists[d].clear_keep(keep);
            }
            PlaylistAction::NeedMeta(p) => self.request_meta(p),
            PlaylistAction::Preview(p) => self.browser_action(BrowserAction::Preview(p), now),
            PlaylistAction::StopPreview => self.browser_action(BrowserAction::StopPreview, now),
            PlaylistAction::PlayNext(i) => {
                let pl = &mut self.playlists[d];
                let before = pl.current.map_or(0, |c| c + 1);
                pl.move_entry(i, before);
            }
            PlaylistAction::EditTags(p) => self.browser_action(BrowserAction::EditTags(p), now),
            // Wie Ziehen ans Ende der anderen Playlist
            PlaylistAction::MoveToOther(i) => self.playlist_action(1 - d, PlaylistAction::Drop { item: DragItem::Row { list: d, index: i }, before: usize::MAX }, now),
            PlaylistAction::AnalyzeBpm(p) => self.browser_action(BrowserAction::AnalyzeBpm { paths: vec![p], force: true }, now),
            PlaylistAction::Drop { item, before } => match item {
                DragItem::Files(paths) => {
                    let at = before.min(self.playlists[d].entries.len());
                    self.playlists[d].insert(at, paths);
                    self.fill_if_empty(d, at);
                }
                DragItem::Row { list, index } if list == d => self.playlists[d].move_entry(index, before),
                DragItem::Row { list, index } => {
                    if self.playlists[list].current == Some(index) && self.has_track(list) {
                        self.notify(trf("Der Titel ist in Player {} geladen und bleibt in dessen Playlist", &[&theme::deck_letter(list)]), now);
                        return;
                    }
                    if let Some(p) = self.playlists[list].remove(index) {
                        let at = before.min(self.playlists[d].entries.len());
                        self.playlists[d].insert(at, [p]);
                        self.fill_if_empty(d, at);
                    }
                }
            },
        }
    }

    /// Titel per Drag & Drop auf einen Player gezogen. Aus der eigenen Playlist wird direkt geladen;
    /// sonst kommt der Titel hinter den aktuellen in die Playlist dieses Players, damit Vor/Zurück
    /// und Auto-Fade danach stimmig weiterlaufen.
    fn drop_on_deck(&mut self, d: usize, item: DragItem, now: f64) {
        if self.ctl.shared.decks[d].is_playing() {
            self.notify(trf("Player {} spielt gerade. Erst stoppen.", &[&theme::deck_letter(d)]), now);
            return;
        }
        let paths = match item {
            DragItem::Row { list, index } if list == d => {
                self.load_index(d, index, false, false);
                return;
            }
            DragItem::Row { list, index } => match self.playlists[list].entries.get(index) {
                Some(p) => vec![p.clone()],
                None => return,
            },
            DragItem::Files(paths) => paths,
        };
        if paths.is_empty() {
            return;
        }
        let pl = &mut self.playlists[d];
        let at = pl.current.map_or(pl.entries.len(), |c| c + 1);
        pl.insert(at, paths);
        self.load_index(d, at, false, false);
    }

    fn browser_action(&mut self, a: BrowserAction, now: f64) {
        match a {
            BrowserAction::Add { list, paths } => {
                let n = paths.len();
                let first = self.playlists[list].entries.len();
                self.playlists[list].push(paths);
                self.fill_if_empty(list, first);
                self.notify(trf("{} Titel in Playlist {}", &[&n, &theme::deck_letter(list)]), now);
            }
            BrowserAction::AddShuffled { list, mut paths } => {
                shuffle(&mut paths);
                self.browser_action(BrowserAction::Add { list, paths }, now);
            }
            BrowserAction::AddNext { list, paths } => {
                let n = paths.len();
                let pl = &mut self.playlists[list];
                let at = pl.current.map_or(0, |c| c + 1).min(pl.entries.len());
                pl.insert(at, paths);
                self.fill_if_empty(list, at);
                self.notify(trf("{} Titel als Nächstes in Playlist {}", &[&n, &theme::deck_letter(list)]), now);
            }
            BrowserAction::Distribute(paths) => {
                // Mit dem Player beginnen, der als Nächstes einblendet: Spielt A, ist B dran
                let st = &self.ctl.shared.decks;
                let start = usize::from(st[0].is_playing() && !st[1].is_playing());
                let n = paths.len();
                let mut split: [Vec<PathBuf>; 2] = Default::default();
                for (k, p) in paths.into_iter().enumerate() {
                    split[(start + k) % 2].push(p);
                }
                for (list, paths) in split.into_iter().enumerate() {
                    let first = self.playlists[list].entries.len();
                    self.playlists[list].push(paths);
                    self.fill_if_empty(list, first);
                }
                self.notify(trf("{} Titel auf Playlist A und B verteilt", &[&n]), now);
            }
            BrowserAction::LoadDeck { deck, path } => self.drop_on_deck(deck, DragItem::Files(vec![path]), now),
            BrowserAction::Reveal(path) => {
                if let Err(e) = reveal(&path) {
                    log::warn!("Anzeigen im Dateimanager: {e}");
                    self.notify(trf("Lässt sich nicht anzeigen: {}", &[&e]), now);
                }
            }
            BrowserAction::StopScan => self.lib.cancel_scan(),
            BrowserAction::FolderRecursion(list) => {
                self.settings.folder_recursion = list;
                self.save_settings();
            }
            BrowserAction::EditTags(path) => self.tag_editor = Some(TagEditor::open(path)),
            BrowserAction::SearchMemory(remember, search) => {
                self.settings.remember_search = remember;
                self.settings.advanced_search = search;
                self.save_settings();
            }
            BrowserAction::TrackColumns(visible, widths) => {
                self.settings.track_columns = visible;
                self.settings.column_widths = widths;
                self.save_settings();
            }
            BrowserAction::TreeWidth(w) => {
                self.settings.tree_width = w;
                self.save_settings();
            }
            BrowserAction::Preview(path) => {
                let token = self.token();
                self.preview = Some(PreviewView { path: path.clone(), token, ended_seen: 0 });
                self.lib.load(PREVIEW, token, path);
                self.enable_headphones(now);
            }
            BrowserAction::StopPreview => {
                self.ctl.send(Command::Eject { deck: PREVIEW });
                self.preview = None;
            }
            BrowserAction::PreviewSeek(s) => self.ctl.send(Command::Seek { deck: PREVIEW, secs: s }),
            BrowserAction::NeedMeta(p) => self.request_meta(p),
            BrowserAction::AnalyzeBpm { paths, force } => {
                for p in paths {
                    // Vielleicht nur umgezogen (Ordner umbenannt): dann ist die BPM schon bekannt
                    if !force {
                        self.resolve_bpm(&p);
                    }
                    // Ohne `force` nur Titel ohne bekannte BPM
                    if (force || !self.bpms.contains_key(&p)) && self.bpm_pending.insert(p.clone()) {
                        self.bpm_total += 1;
                        self.lib.analyze_bpm(p, force);
                    }
                }
            }
            BrowserAction::StopBpm => {
                self.lib.cancel_bpm();
                self.bpm_pending.clear();
                self.bpm_total = 0;
            }
            BrowserAction::AddRoot => {
                if let Some(dir) = rfd::FileDialog::new().set_title(tr("Musikordner hinzufügen")).pick_folder()
                    && self.browser.add_root(dir.clone())
                {
                    self.settings.folders.push(dir);
                    self.save_settings();
                }
            }
            BrowserAction::RemoveRoot(dir) => {
                self.browser.remove_root(&dir);
                self.settings.folders.retain(|p| p != &dir);
                self.save_settings();
            }
            BrowserAction::HistoryRename { date, name } => self.history.rename(&date, &name),
            BrowserAction::HistoryRestore(date) => self.restore_history(&date, now),
            BrowserAction::HistoryRemove { date, slots } => {
                self.history.remove_entries(&date, &slots);
                self.notify(trf("{} Einträge aus der Historie entfernt", &[&slots.len()]), now);
            }
            BrowserAction::HistoryDelete(date) => {
                if let Some(day) = self.history.day(&date) {
                    self.history_dialog = Some(HistoryDialog::Delete { date, label: day.label(), count: day.entries.len() });
                }
            }
            BrowserAction::HistoryExport { date, format } => self.export_history(&date, format, now),
            BrowserAction::HistoryImport => {
                if self.import_rx.is_none()
                    && let Some(file) = rfd::FileDialog::new()
                        .set_title(tr("Historie importieren"))
                        .add_filter(tr("Historie oder Playlist"), &["json", "m3u", "m3u8"])
                        .pick_file()
                {
                    let ctx = self.repaint_ctx.clone();
                    self.import_rx = Some(crate::history::import_in_background(file, self.music_roots(), move || ctx.request_repaint()));
                }
            }
            BrowserAction::OpenSettings => self.settings_ui.toggle(),
            BrowserAction::ToggleFullscreen => self.toggle_fullscreen = true,
        }
    }
}

/// Plastisches Gehäuse-Panel hinter dem Inhalt der Zelle.
/// Rahmen eines laufenden Players blinkt rot, wenn der Titel bald zu Ende ist.
fn end_warning(ui: &egui::Ui, rect: egui::Rect, m: &DeckModel<'_>) {
    let left = m.duration - m.position;
    if !m.playing || m.duration <= 0.0 || left <= 0.0 || left > END_WARNING_SECS {
        return;
    }
    let period = if left <= END_WARNING_FAST_SECS { 0.4 } else { 0.8 };
    let t = ui.input(|i| i.time);
    // Etwas länger an als aus, damit das Blinken deutlich wirkt
    if t % period < period * 0.6 {
        crate::hw::alert_border(ui.painter(), rect, theme::cue());
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}

fn panel_frame(ui: &egui::Ui) -> Frame {
    crate::hw::panel(ui.painter(), ui.max_rect().shrink(3.0));
    Frame::new().inner_margin(Margin::same(15))
}

impl eframe::App for FadeWerkDjApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.dev.raw_input(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Die ersten Bilder sind gezeichnet: Der Grafikstart hat geklappt
        if ctx.cumulative_frame_nr() >= 2 {
            crate::diagnose::graphics_running();
        }
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = now;

        self.poll_library(&ctx, now);
        self.poll_import(now);
        self.record_plays();
        self.handle_track_ends();
        self.step_autofade(dt);
        self.step_auto_sync(dt, now);
        self.push_eq();
        let levels = self.ctl.shared.take_levels();
        for (m, l) in self.meters.iter_mut().zip(levels) {
            m.update(l, dt, now);
        }
        self.bands.update(self.ctl.shared.take_bands(), dt, now);

        let mut deck_actions: [Vec<DeckAction>; 2] = Default::default();
        let mut mixer_actions = Vec::new();
        let mut fade_actions = Vec::new();
        let mut pl_actions: [Vec<PlaylistAction>; 2] = Default::default();
        let mut browser_actions = Vec::new();
        let mut deck_drops: Vec<(usize, DragItem)> = Vec::new();

        // Neu geöffneter Ordner: Titel samt Unterordnern im Hintergrund suchen
        if let Some(dirs) = self.browser.take_pending_scan() {
            let token = self.token();
            self.browser.begin_scan(token);
            self.lib.scan(token, dirs);
        }

        // Ordnerbrowser unten
        let preview_st = &self.ctl.shared.decks[PREVIEW];
        let bctx = BrowserCtx {
            metas: &self.metas,
            bpms: &self.bpms,
            bpm_progress: (self.bpm_total > 0).then(|| (self.bpm_total - self.bpm_pending.len(), self.bpm_total)),
            preview: self.preview.as_ref().map(|p| p.path.as_path()),
            preview_pos: preview_st.position_secs(),
            preview_dur: preview_st.duration_secs(),
            deck_playing: [self.ctl.shared.decks[0].is_playing(), self.ctl.shared.decks[1].is_playing()],
            fullscreen: ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)),
            history: &self.history,
            importing: self.import_rx.is_some(),
        };
        // Höhe merken; ohne gespeicherten Wert 240 pt, auf hohen Monitoren mehr (bei 1400 pt Höhe etwa 450),
        // auf niedrigen weniger (ein gutes Viertel der Höhe)
        let total_h = ui.available_height();
        let browser_h = if self.settings.browser_height > 0.0 {
            self.settings.browser_height
        } else if total_h >= 920.0 {
            240.0 + (total_h - 920.0) * 0.44
        } else {
            (total_h * 0.26).max(160.0)
        };
        let browser_resp = egui::Panel::bottom("browser")
            .resizable(true)
            .default_size(browser_h)
            .min_size(160.0)
            .max_size((total_h - DECK_ROW_MIN - PLAYLIST_MIN - 30.0).max(160.0))
            .frame(Frame::new().fill(theme::panel()).stroke(Stroke::new(1.0, theme::line())).inner_margin(Margin::same(10)))
            .show(ui, |ui| {
                if theme::brushed() {
                    let p = ui.ctx().layer_painter(ui.layer_id());
                    let face = crate::hw::steel_face(theme::panel(), theme::skin().dark);
                    crate::hw::surface(&p, ui.max_rect().expand(10.0), 0.0, crate::hw::lighten(face, 0.04), crate::hw::darken(face, 0.18));
                }
                self.browser.show(ui, &bctx, &mut browser_actions)
            });
        let heights = (ctx.content_rect().height().round(), browser_resp.response.rect.height().round());
        // Nur bei gedrückter Maustaste: dann zieht der Nutzer die Kante, sonst hat sich der Inhalt geändert
        let dragging = ctx.input(|i| i.pointer.primary_down());
        if dragging && self.last_heights.0 == heights.0 && self.last_heights.1 > 0.0 && (self.last_heights.1 - heights.1).abs() >= 1.0 {
            self.settings.browser_height = heights.1;
            self.save_due = Some(now + 1.0);
        }
        self.last_heights = heights;

        let decks_models: Vec<DeckModel<'_>> = (0..2)
            .map(|d| {
                let v = &self.decks[d];
                let st = &self.ctl.shared.decks[d];
                let in_sync = v.loaded_id != 0 && st.track_id() == v.loaded_id;
                DeckModel {
                    deck: d,
                    title: v.meta.as_ref().map_or("", |m| m.title.as_str()),
                    artist: v.meta.as_ref().map_or("", |m| m.artist.as_str()),
                    bpm: v.bpm,
                    pitch_pct: v.pitch_pct,
                    auto_synced: v.sync.is_some(),
                    position: if in_sync { st.position_secs() } else { 0.0 },
                    duration: if in_sync { st.duration_secs() } else { v.meta.as_ref().map_or(0.0, |m| m.duration_secs) },
                    playing: in_sync && st.is_playing(),
                    speed: if in_sync { st.speed() } else { 0.0 },
                    has_track: v.loaded_id != 0,
                    loading: v.pending.is_some(),
                    waveform: v.waveform.as_deref(),
                    cover: v.cover.as_ref().map(|t| t.id()),
                    pfl: v.pfl,
                    repeat: v.repeat,
                    loop_beats: v.loop_beats,
                    loop_range: v.loop_range,
                    on_air: self.on_air(d),
                    scratching: v.scratching,
                }
            })
            .collect();
        let mixer_model = MixerModel {
            meters: &self.meters,
            bands: &self.bands,
            volume: [self.decks[0].volume, self.decks[1].volume],
            head_mode: self.head_mode,
            head_mix: self.head_mix,
            head_volume: self.head_volume,
            master_volume: self.master_volume,
            audio_ok: self.audio_error.is_none(),
            head_device: self.head_audio.is_some(),
            eq_db: self.eq_db,
            eq_kill: self.eq_kill,
        };
        let status = self.fade_status();
        let fade_model = FadeModel {
            auto: self.autofade.enabled,
            fade_secs: self.autofade.fade_secs,
            curve: self.curve,
            crossfader: self.crossfader,
            status: &status,
            motor: self.motor,
            bass_swap: self.bass_swap,
            auto_sync: self.auto_sync,
        };
        let playlists = &self.playlists;
        let metas = &self.metas;
        let bpms = &self.bpms;
        let has = [self.decks[0].loaded_id != 0, self.decks[1].loaded_id != 0];
        let playing = [self.ctl.shared.decks[0].is_playing(), self.ctl.shared.decks[1].is_playing()];
        let preview_path = self.preview.as_ref().map(|p| p.path.as_path());
        let deck_height_setting = self.settings.deck_height;
        // Je Seite: Zelle des Players und Oberkante der Playlist, für den Griff dazwischen
        let mut split_bounds = [(Rect::NOTHING, 0.0f32); 2];
        let mut split_change: Option<f32> = None;

        egui::CentralPanel::default().frame(Frame::new().fill(theme::ground()).inner_margin(Margin::same(8))).show(ui, |ui| {
            if theme::brushed() {
                crate::hw::brushed_overlay(ui.painter(), ui.max_rect().expand(8.0), 0.0, theme::skin().dark);
            }
            // Links Player A über Playlist A, rechts B über B. Die Mitte (Mixer und Auto-Fade)
            // läuft über die ganze Höhe, damit keine Zeile die andere in die Länge zieht.
            let center_w = center_width(ui.available_width());
            // Player wachsen mit ihrer Breite auch in der Höhe, damit sie nicht zu flachen Streifen werden
            let deck_inner = (ui.available_width() - center_w) / 2.0 - 36.0;
            // Höchstens gut die Hälfte der Höhe, damit die Playlist darunter Platz behält; auf kleinen
            // Bildschirmen schrumpfen die Player. Hat der Nutzer die Kante gezogen, gilt seine Höhe.
            let avail_h = ui.available_height();
            let deck_max = DECK_ROW_HEIGHT * deck_ui::deck_scale(deck_inner);
            let wanted = if deck_height_setting > 0.0 { deck_height_setting } else { deck_max.min(avail_h * 0.58) };
            let deck_h = wanted.min(deck_max).min(avail_h - PLAYLIST_MIN).max(DECK_ROW_MIN);
            StripBuilder::new(ui).size(Size::remainder()).size(Size::exact(center_w)).size(Size::remainder()).horizontal(|mut cols| {
                for side in [0usize, 2] {
                    let d = side / 2;
                    if side == 2 {
                        cols.cell(|ui| {
                            panel_frame(ui).show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                egui::ScrollArea::vertical().id_salt("center").auto_shrink([false, false]).show(ui, |ui| {
                                    mixer_ui::show_mixer(ui, &mixer_model, &mut mixer_actions);
                                    ui.separator();
                                    mixer_ui::show_fade(ui, &fade_model, &mut fade_actions);
                                });
                            });
                        });
                    }
                    cols.strip(|b| {
                        b.size(Size::exact(deck_h)).size(Size::remainder().at_least(120.0)).vertical(|mut rows| {
                            rows.cell(|ui| {
                                let cell = ui.max_rect();
                                split_bounds[d].0 = cell;
                                panel_frame(ui).show(ui, |ui| {
                                    ui.set_min_size(ui.available_size());
                                    deck_ui::show(ui, &decks_models[d], &mut deck_actions[d]);
                                });
                                end_warning(ui, cell.shrink(3.0), &decks_models[d]);
                                if let Some(item) = deck_drop_zone(ui, cell.shrink(3.0), d) {
                                    deck_drops.push((d, item));
                                }
                            });
                            rows.cell(|ui| {
                                split_bounds[d].1 = ui.max_rect().top();
                                panel_frame(ui).show(ui, |ui| {
                                    ui.set_min_size(ui.available_size());
                                    playlist_ui::show(ui, d, &playlists[d], has[d], playing[d], preview_path, metas, bpms, &mut pl_actions[d]);
                                });
                            });
                        });
                    });
                }
            });
            // Griff zwischen Player und Playlist: ziehen ändert die Höhe der Player, Doppelklick = automatisch
            for (d, (cell, list_top)) in split_bounds.iter().enumerate() {
                let y = (cell.bottom() + list_top) / 2.0;
                let rect = Rect::from_min_max(pos2(cell.left() + 6.0, y - 4.0), pos2(cell.right() - 6.0, y + 4.0));
                let resp = ui.interact(rect, egui::Id::new(("deck_split", d)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeRow);
                if resp.hovered() || resp.dragged() {
                    let line = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.3, 3.0));
                    ui.painter().rect_filled(line, 1.5, theme::dim());
                }
                if resp.double_clicked() {
                    split_change = Some(0.0);
                } else if resp.dragged() && resp.drag_delta().y != 0.0 {
                    split_change = Some((deck_h + resp.drag_delta().y).max(DECK_ROW_MIN));
                }
                resp.on_hover_text(tr("Ziehen ändert die Höhe der Player, Doppelklick stellt sie wieder automatisch ein"));
            }
        });
        drop(decks_models);
        if let Some(h) = split_change {
            self.settings.deck_height = h;
            self.save_due = Some(now + 1.0);
        }

        for (d, acts) in deck_actions.into_iter().enumerate() {
            for a in acts {
                self.deck_action(d, a, now);
            }
        }
        for (d, item) in deck_drops {
            self.drop_on_deck(d, item, now);
        }
        for a in mixer_actions {
            self.mixer_action(a, now);
        }
        for a in fade_actions {
            self.fade_action(a, now);
        }
        for (d, acts) in pl_actions.into_iter().enumerate() {
            for a in acts {
                self.playlist_action(d, a, now);
            }
        }
        for a in browser_actions {
            self.browser_action(a, now);
        }

        self.update_ui(&ctx);

        // Hinweis unten in der Mitte
        if let Some((text, until)) = self.notice.clone() {
            if until > 0.0 && now > until {
                self.notice = None;
            } else {
                if until == 0.0 {
                    self.notice = Some((text.clone(), now + 6.0));
                }
                egui::Area::new(egui::Id::new("notice")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0)).show(&ctx, |ui| {
                    Frame::new().fill(theme::ink()).corner_radius(CornerRadius::same(5)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
                        ui.label(RichText::new(text).color(theme::ground()).strong());
                    });
                });
            }
        }

        // Vollbild: F11 überall, auf macOS zusätzlich ⌃⌘F (wie die grüne Fenstertaste)
        let fs_key = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::F11)
                || (cfg!(target_os = "macos") && i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::MAC_CMD, egui::Key::F))
        });
        let fullscreen = ctx.input(|i| i.viewport().fullscreen);
        if fs_key || std::mem::take(&mut self.toggle_fullscreen) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen.unwrap_or(false)));
        }
        // Zustand merken, auch wenn er über die Fenstertaste des Systems wechselt
        if let Some(fs) = fullscreen
            && fs != self.settings.fullscreen
        {
            self.settings.fullscreen = fs;
            self.save_due = Some(now + 1.0);
        }

        // Zoom wie im Browser: ⌘/Strg + Plus, Minus, 0
        let zoom_key = ctx.input_mut(|i| {
            let mut key = |k| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, k));
            if key(egui::Key::Plus) || key(egui::Key::Equals) {
                Some(ZOOM_STEP)
            } else if key(egui::Key::Minus) {
                Some(-ZOOM_STEP)
            } else if key(egui::Key::Num0) {
                Some(0.0)
            } else {
                None
            }
        });
        if let Some(step) = zoom_key {
            let z = if step == 0.0 { 1.0 } else { ctx.zoom_factor() + step };
            self.set_zoom(&ctx, z);
        }

        // Einstellungen: ⌘, (macOS) bzw. Strg+,
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Comma))) {
            self.settings_ui.toggle();
        }
        let mut settings_actions = Vec::new();
        let sctx = SettingsCtx {
            settings: &self.settings,
            active_device: self.audio.as_ref().map(|a| a.device.as_str()),
            active_device_id: self.audio.as_ref().map(|a| a.id.as_str()),
            active_head: self.head_audio.as_ref().map(|h| h.device.as_str()),
            sample_rate: self.ctl.shared.sample_rate,
            channels: self.ctl.shared.channels,
        };
        self.settings_ui.show(&ctx, &sctx, &mut settings_actions);
        if let Some(event) = self.tag_editor.as_mut().and_then(|ed| ed.show(&ctx)) {
            match event {
                TagEditorEvent::Close => self.tag_editor = None,
                TagEditorEvent::Saved(path) => {
                    // Tags neu einlesen, damit Browser, Playlists und Player die neuen Angaben zeigen
                    self.metas.remove(&path);
                    self.meta_requested.remove(&path);
                    self.request_meta(path);
                    self.notify(tr("Tags gespeichert"), now);
                }
                TagEditorEvent::Reveal(path) => self.browser_action(BrowserAction::Reveal(path), now),
            }
        }
        if let Some(event) = self.history_dialog.as_mut().and_then(|d| d.show(&ctx)) {
            self.history_dialog = None;
            match event {
                DialogEvent::Import(day) => {
                    let (date, n) = (day.date.clone(), day.entries.len());
                    self.history.merge(day);
                    self.browser.open_history_day(date.clone());
                    self.notify(trf("Historie {} importiert: {} Titel", &[&date, &n]), now);
                }
                DialogEvent::Delete(date) => {
                    self.history.delete_day(&date);
                    self.notify(trf("Historie {} gelöscht", &[&date]), now);
                }
                DialogEvent::Close => {}
            }
        }
        for a in settings_actions {
            self.settings_action(a, &ctx, now);
        }
        self.track_settings(now);
        // Playlists alle zwei Sekunden sichern, falls sich etwas geändert hat (auch für Abstürze)
        if now - self.session_checked > 2.0 {
            self.session_checked = now;
            self.save_session_if_changed();
            if self.persist {
                self.bpm_cache.save_if_dirty();
                self.history.save_dirty();
            }
        }
        if let Some(rx) = &self.bpm_check
            && let Ok(results) = rx.try_recv()
        {
            self.bpm_check = None;
            let removed = self.bpm_cache.apply_check(results);
            if removed > 0 {
                log::info!("BPM-Cache: {removed} Einträge ohne Datei entfernt");
            }
        }

        self.dev.on_frame(&ctx);
        self.ctl.collect_garbage();
        // DJ-Software: Teller, Wellenform und Pegel laufen ständig
        ctx.request_repaint();
    }
}

fn shuffle_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |t| t.as_nanos() as u64)
}

fn shuffle(paths: &mut Vec<PathBuf>) {
    let mut pl = Playlist { entries: std::mem::take(paths), current: None };
    pl.shuffle_upcoming(shuffle_seed());
    *paths = pl.entries;
}

/// Zeigt eine Datei oder einen Ordner im Dateimanager des Systems.
fn reveal(path: &Path) -> std::io::Result<()> {
    use std::process::Command as Cmd;
    if cfg!(target_os = "macos") {
        Cmd::new("open").arg("-R").arg(path).spawn()?;
    } else if cfg!(windows) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path);
        Cmd::new("explorer").arg(arg).spawn()?;
    } else {
        // Ohne einheitliches „Datei markieren“ unter Linux: den Ordner öffnen
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        Cmd::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}

fn curve_from(i: u8, points: [f32; 4]) -> Curve {
    match i {
        1 => Curve::Linear,
        2 => Curve::Cut,
        3 => Curve::Custom(points),
        _ => Curve::Soft,
    }
}

impl Drop for FadeWerkDjApp {
    fn drop(&mut self) {
        self.save_settings();
        self.save_session_if_changed();
        if self.persist {
            self.bpm_cache.save_if_dirty();
            self.history.save_dirty();
        }
    }
}

fn start_dir_for_demo(app: &FadeWerkDjApp) -> Option<PathBuf> {
    std::env::args_os().nth(1).map(PathBuf::from).filter(|p| p.is_dir()).or_else(|| app.browser.tracks().first().and_then(|p| p.parent().map(Path::to_path_buf)))
}

/// Ablagefläche über einem Player: leuchtet beim Ziehen eines Titels auf und liefert ihn beim Loslassen.
fn deck_drop_zone(ui: &egui::Ui, rect: egui::Rect, d: usize) -> Option<DragItem> {
    let ctx = ui.ctx();
    if !egui::DragAndDrop::has_payload_of_type::<DragItem>(ctx) {
        return None;
    }
    let over = ctx.pointer_latest_pos().is_some_and(|p| rect.contains(p));
    if !over {
        return None;
    }
    let color = theme::deck_color(d);
    let p = ui.painter();
    p.rect_filled(rect, 8.0, color.gamma_multiply(0.14));
    p.rect_stroke(rect, 8.0, Stroke::new(3.0, color), egui::StrokeKind::Inside);
    let label = trf("In Player {} laden", &[&theme::deck_letter(d)]);
    let pill = egui::Rect::from_center_size(rect.center(), vec2(240.0, 44.0));
    crate::hw::paint_button(p, pill, &label, crate::fonts::bold(18.0), crate::hw::Look::Color(color), true, false, false, true);
    if ctx.input(|i| i.pointer.any_released()) {
        return egui::DragAndDrop::take_payload::<DragItem>(ctx).map(|a| (*a).clone());
    }
    None
}
