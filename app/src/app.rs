// SPDX-License-Identifier: GPL-3.0-or-later
//! Zentrale App-Logik: verbindet Oberfläche, Engine und Bibliothek.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, pos2, vec2};
use egui_extras::{Size, StripBuilder};
use fw_engine::{AutoFade, Command, Controller, Curve, HeadMode, Motor, PREVIEW, TrackAudio, db_to_gain, eq};
use fw_library::{Camelot, Library, Response, TrackMeta, WaveSketch, Waveform};

use crate::audio::{self, AudioOut, HeadOut};
use crate::fade_preview;
use crate::browser::{Browser, BrowserAction, BrowserCtx};
use crate::deck_ui::{self, DeckAction, DeckModel, PITCH_RANGE, Repeat};
use crate::devtools::DevTools;
use crate::i18n::{self, tr, trf};
use crate::history::{Entry, ExportFormat, History, PlayTracker};
use crate::history_ui::{DialogEvent, HistoryDialog};
use crate::settings::{Settings, ViewMode};
use crate::statusbar::{OnAir, StatusAction, StatusModel};
use crate::settings_ui::{SettingsAction, SettingsCtx, SettingsUi};
use crate::tag_editor::{TagEditor, TagEditorEvent};
use crate::meter::Meter;
use crate::mixer_ui::{self, FadeAction, FadeModel, MixerAction, MixerModel};
use crate::playlist::{DragItem, Playlist};
use crate::playlist_ui::{self, PlaylistAction};
use crate::theme;

mod actions;
mod commands;
mod fade;
mod library;
mod output;
mod played;
mod prefs;
mod views;

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
/// Kleinstes Fenster bei Zoom 1 in Punkten; auf kleineren Bildschirmen höchstens so groß wie der Bildschirm.
const MIN_WINDOW: egui::Vec2 = egui::vec2(1100.0, 620.0);
/// Kleinstes Fenster, solange die kompakte Ansicht möglich ist (Netbook 1024×600 ohne Zoom).
const MIN_WINDOW_COMPACT: egui::Vec2 = egui::vec2(1000.0, 480.0);
/// Ist der Bildschirm niedriger (logische Pixel, wie Netbooks mit 1024×600), schaltet „Kompakte Ansicht:
/// Automatisch“ ein. Bewusst nach dem Bildschirm, nicht nach dem Fenster: Ein klein gezogenes Fenster auf einem
/// normalen Bildschirm behält die gewohnte Ansicht (Nutzerwunsch).
const COMPACT_AUTO_H: f32 = 720.0;
/// Breite der Leiste „ORDNER“ am linken Rand und Dauer des Hereingleitens (kompakte Ansicht).
const SLIDE_STRIP_W: f32 = 30.0;
const SLIDE_SECS: f32 = 0.25;
/// So viel Bildschirmhöhe bleibt für Fensterleiste und Taskleiste frei (logische Pixel).
const SCREEN_RESERVE_H: f32 = 60.0;
/// So viele Sekunden vor Titelende beginnt der Rahmen eines laufenden Players zu blinken.
const END_WARNING_SECS: f64 = 15.0;
/// In den letzten Sekunden blinkt er doppelt so schnell.
const END_WARNING_FAST_SECS: f64 = 5.0;
/// So schnell (Prozent Pitch pro Sekunde) läuft Auto-Sync nach der Überblendung auf 0 % zurück, unhörbar langsam.
const AUTO_SYNC_RATE: f32 = 0.1;
/// So stark tritt ein Player zurück, solange nur der andere zu hören ist (Deckkraft der Grundfarbe darüber).
const OFF_AIR_DIM: f32 = 0.38;

/// Bildabstand der sparsamen Darstellung (30 Bilder pro Sekunde), solange sich etwas bewegt.
const LITE_FRAME: std::time::Duration = std::time::Duration::from_millis(33);
/// Bildabstand der vollen Optik, solange sich etwas bewegt (60 Bilder pro Sekunde).
const FULL_FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);
/// Im Ruhezustand nur so oft neu zeichnen (ohne Eingaben), damit Zustände aus dem Audio-Thread ankommen.
const IDLE_FRAME: std::time::Duration = std::time::Duration::from_millis(250);
/// Dasselbe in der sparsamen Darstellung: Eingaben und Antworten aus dem Hintergrund zeichnen ohnehin sofort.
const IDLE_FRAME_LITE: std::time::Duration = std::time::Duration::from_millis(1000);
/// Takt der Abspiel-Logik bei verdecktem oder minimiertem Fenster, solange etwas läuft (sonst `IDLE_FRAME`).
const HIDDEN_TICK: std::time::Duration = std::time::Duration::from_millis(100);
/// Ab so vielen Fehlern pro Sekunde gilt die Audio-Ausgabe als hängend und wird neu geöffnet.
const AUDIO_ERROR_STORM: f64 = 20.0;
/// Größter Zeitschritt für Auto-Fade und Auto-Sync; bei verdecktem Fenster kommen die Aufrufe seltener.
const LOGIC_MAX_DT: f64 = 1.0;
/// So lange nach dem letzten Ton bleibt der Ruhezustand noch gesperrt (kein Flattern beim Titelwechsel).
const AWAKE_HOLD_SECS: f64 = 10.0;
/// Breite der Mittelspalte: wächst mit dem Fenster, damit die Player auf großen Monitoren nicht ausufern.
/// Ordnerbrowser rechts: So breit bleibt die Fläche für Player und Playlists mindestens (wie das kleinste
/// Fenster), und so schmal wird der Browser höchstens. Ist das Fenster schmaler als beides, steht er unten.
const SIDE_MAIN_MIN: f32 = 1100.0;
const SIDE_BROWSER_MIN: f32 = 420.0;
fn center_width(total: f32) -> f32 {
    (total * 0.2).clamp(250.0, 440.0)
}

struct Pending {
    token: u64,
    index: Option<usize>,
    autoplay: bool,
    keep_playing: bool,
    /// Der Anfang spielt schon (`Response::Partial`); der vollständige Titel wird nur noch eingesetzt.
    started: bool,
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
    /// Ausgeworfene Platte, solange sie noch aus dem Player gleitet: (Titel, Interpret, Cover, Zeitpunkt).
    ejected: Option<(String, String, Option<egui::TextureHandle>, f64)>,
    /// Kennung des Titels in der Engine, 0 = leer.
    loaded_id: u64,
    /// Der geladene Titel wurde schon weggeblendet und die Playlist hat keinen weiteren: Auto-Fade blendet
    /// nicht mehr hierher (sonst liefe der Rest des alten Titels noch einmal), der andere Player spielt allein weiter.
    spent: bool,
    pending: Option<Pending>,
    pitch_pct: f32,
    /// Auto-Sync steuert den Pitch: Tempofaktor gegenüber dem anderen Titel (1, 2 oder ½).
    sync: Option<f32>,
    /// Lief im letzten Bild (für den Start-Zeitpunkt von Auto-Sync).
    was_playing: bool,
    pfl: bool,
    volume: f32,
    repeat: Repeat,
    reverse: bool,
    /// Zuletzt gesehener Stand von `reverse_ended_count`.
    reverse_ended_seen: u64,
    loop_beats: Option<u32>,
    loop_range: Option<(f64, f64)>,
    scratching: bool,
    scratch_pos: f64,
    ended_seen: u64,
    /// Geglättete Position für die Anzeige (Teller, Wellenform, Tonarm), siehe [`DeckView::smooth_position`].
    shown_pos: f64,
    /// Zuletzt von der Engine gemeldete Position und wann sie sich zuletzt geändert hat.
    raw_pos: (f64, f64),
}

/// Weicht die angezeigte Position mehr ab (Sprung, Spulen), springt sie direkt mit.
const POS_SNAP_SECS: f64 = 0.05;
/// So weit wird über die letzte Meldung der Engine hinaus höchstens vorausgerechnet (Sekunden).
const POS_PREDICT_MAX: f64 = 0.06;

impl DeckView {
    /// Die Engine meldet die Position nur einmal je Audiopuffer (etwa alle 10 ms, je nach Gerät auch
    /// seltener). Bei 60–120 Bildern/s liefe der Teller damit stoßweise. Darum läuft die Anzeige mit der
    /// gemeldeten Geschwindigkeit weiter und wird sanft auf die Meldungen nachgezogen.
    fn smooth_position(&mut self, raw: f64, rate: f64, now: f64, dt: f64) -> f64 {
        if raw != self.raw_pos.0 {
            self.raw_pos = (raw, now);
        }
        let predicted = raw + rate * (now - self.raw_pos.1).min(POS_PREDICT_MAX);
        self.shown_pos += rate * dt;
        let err = predicted - self.shown_pos;
        if err.abs() > POS_SNAP_SECS {
            self.shown_pos = predicted;
        } else {
            self.shown_pos += err * (dt * 10.0).min(1.0);
        }
        self.shown_pos
    }
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
            ejected: None,
            loaded_id: 0,
            spent: false,
            pending: None,
            pitch_pct: 0.0,
            sync: None,
            was_playing: false,
            pfl: false,
            volume: crate::settings::DEFAULT_CHANNEL_VOLUME,
            repeat: Repeat::Off,
            reverse: false,
            reverse_ended_seen: 0,
            loop_beats: None,
            loop_range: None,
            scratching: false,
            scratch_pos: 0.0,
            ended_seen: 0,
            shown_pos: 0.0,
            raw_pos: (0.0, 0.0),
        }
    }
}

struct PreviewView {
    path: PathBuf,
    token: u64,
    ended_seen: u64,
    /// Vorhören im Fade-Editor: Eintrag, Art und Versatz zum Titel. Dann zeigen Browser und Playlists es
    /// nicht als normales Vorhören des Titels.
    clip: Option<ClipView>,
}

struct ClipView {
    list: usize,
    index: usize,
    kind: fade_preview::Kind,
    /// Noch im Mischen; danach Stelle im Titel = Stelle im Ausschnitt + `offset`.
    rendering: Option<std::sync::mpsc::Receiver<Result<fade_preview::Clip, String>>>,
    offset: f64,
}

/// Caches für große Sammlungen und Netzlaufwerke (siehe `tag_cache`, `folder_cache`, `analysis_cache`).
#[derive(Default)]
struct Caches {
    tag: crate::tag_cache::TagCache,
    folder: crate::folder_cache::FolderCache,
    /// Ermittelte BPM bzw. Tonarten (`bpm.json`, `tonart.json`).
    bpm: crate::analysis_cache::BpmCache,
    key: crate::analysis_cache::KeyCache,
    /// Gemessene Größen für Einstellungen → Musik und die laufende Messung.
    sizes: Option<crate::caches::CacheSizes>,
    sizes_rx: Option<std::sync::mpsc::Receiver<crate::caches::CacheSizes>>,
}

/// Update-Prüfung bei GitHub und Selbst-Update.
#[derive(Default)]
struct UpdateState {
    /// Laufende Prüfung.
    rx: Option<std::sync::mpsc::Receiver<Result<Option<crate::update::NewVersion>, crate::update::CheckError>>>,
    /// Von Hand angestoßen: dann auch „aktuell“ und Fehler melden.
    manual: bool,
    /// Laufendes Herunterladen und Installieren, mit letztem Stand.
    installing: Option<std::sync::mpsc::Receiver<crate::self_update::Progress>>,
    progress: Option<crate::self_update::Progress>,
    /// Gefundene neue Version (Leiste oben).
    new_version: Option<crate::update::NewVersion>,
    /// Beim ersten Start fragen, ob geprüft werden soll.
    ask: bool,
}

/// Ruhezustand verhindern, solange ein Player oder das Vorhören spielt.
#[derive(Default)]
struct WakeLock {
    lock: Option<keepawake::KeepAwake>,
    /// Zuletzt etwas zu hören (für `AWAKE_HOLD_SECS`).
    last_sound: f64,
    /// Sperre scheiterte: erst nach dem nächsten Stillstand erneut versuchen.
    failed: bool,
}

/// Wellenform-Skizzen der Playlist-Titel für den FadeWerk-Modus.
#[derive(Default)]
struct Sketches {
    map: HashMap<PathBuf, Option<Arc<WaveSketch>>>,
    requested: HashSet<PathBuf>,
    /// Anfragen dieses Bildes je Playlist, abwechselnd abgeschickt.
    batch: [Vec<PathBuf>; 2],
}

pub struct FadeWerkDjApp {
    /// Tag-, Ordner-, BPM- und Tonart-Cache und die gemessenen Größen (Einstellungen → Musik).
    caches: Caches,
    /// Update-Prüfung, Selbst-Update und die Leiste dazu.
    update: UpdateState,
    /// Ruhezustand verhindern, solange etwas spielt.
    wake: WakeLock,
    /// Wellenform-Skizzen für den FadeWerk-Modus.
    sketch: Sketches,
    audio: Option<AudioOut>,
    audio_error: Option<String>,
    /// Zuletzt gesetztes Mindestfenster in Punkten (`fit_screen`).
    min_window: Option<egui::Vec2>,
    /// Kleiner Bildschirm (niedriger als `COMPACT_AUTO_H`): dort ist die kompakte Ansicht automatisch an.
    small_screen: bool,
    /// Die Bildschirmgröße ist bekannt (`fit_screen` lief). Vorher weiß die App nicht, ob die kompakte Ansicht
    /// gilt, und damit nicht, ob ein Cover gebraucht wird; Ladeaufträge warten bis dahin in `deferred_loads`.
    screen_known: bool,
    deferred_loads: Vec<(usize, usize, bool, bool)>,
    /// Kompakte Ansicht: Ordnerbereich eingeschoben (gleitet von links über die ganze Oberfläche).
    browser_slid: bool,
    /// Beim Ziehen eines Titels zurückgeglitten; nach dem Ablegen kommt er wieder herein.
    browser_slid_for_drag: bool,
    head_audio: Option<HeadOut>,
    ctl: Controller,
    lib: Library,
    decks: [DeckView; 2],
    playlists: [Playlist; 2],
    browser: Browser,
    metas: HashMap<PathBuf, TrackMeta>,
    /// Zählt Änderungen an `metas` und `bpms` (die Titelliste filtert dann neu).
    data_gen: u64,
    meta_requested: HashSet<PathBuf>,
    /// Davon mit Vorrang angefordert (sichtbare Zeilen), damit jeder Pfad nur einmal vorgezogen wird.
    meta_urgent: HashSet<PathBuf>,
    bpms: HashMap<PathBuf, f32>,
    /// Ermittelte Tonarten, `None` = Analyse war unsicher (nicht erneut analysieren). Tonarten aus Tags stehen
    /// in `metas`; [`Self::key_of`] fragt beide ab.
    keys: HashMap<PathBuf, Option<Camelot>>,
    /// Laufende Ordnersuche: Kennung, gesuchte Ordner, Start (zum Merken langsamer Ordner).
    scan_job: Option<(u64, crate::folder_cache::Roots, std::time::Instant)>,
    /// Titel, deren BPM-Analyse noch läuft, und Gesamtzahl seit dem letzten Leerlauf (für die Fortschrittsanzeige).
    bpm_pending: HashSet<PathBuf>,
    bpm_total: usize,
    /// Ergebnis der Hintergrundprüfung des BPM-Caches beim Start (welche Dateien es noch gibt).
    bpm_check: Option<std::sync::mpsc::Receiver<Vec<(PathBuf, bool)>>>,
    key_check: Option<std::sync::mpsc::Receiver<Vec<(PathBuf, bool)>>>,
    /// „Harmonisch“ wartet auf die Analyse fehlender Tonarten.
    harmonic_pending: bool,
    /// Playlists vor dem letzten Sortieren (harmonisch oder zufällig), Fingerabdruck danach und Zeitpunkt (für „Rückgängig“).
    sort_undo: Option<([Playlist; 2], u64, f64)>,
    /// FadeWerk-Modus: Playlists zeigen Wellenformen und Fade-Bereiche.
    fadewerk: bool,
    autofade: AutoFade,
    /// Laufende FadeWerk-Überblendung, die sich nach dem alten Titel richtet: (Player, Beginn und Ende seines
    /// Ausblendbereichs). Bei FADE per Hand `None`, dann laufen die Phasen nach Zeit.
    fade_track: Option<(usize, f64, f64)>,
    /// Zuletzt an die Engine geschickt: Crossfader zweiphasig auswerten (`None` = unbekannt, neu schicken).
    split_sent: Option<bool>,
    crossfader: f32,
    curve: Curve,
    motor: Motor,
    head_mode: HeadMode,
    head_mix: f32,
    head_volume: f32,
    master_volume: f32,
    /// Klänge der Effekt-Pads.
    fx: crate::fx::FxBank,
    fx_volume: f32,
    /// EQ je Player in dB und Kill-Tasten (Bässe, Mitten, Höhen).
    eq_db: [[f32; 3]; 2],
    eq_kill: [[bool; 3]; 2],
    /// Filter je Player, −1 Tiefpass bis +1 Hochpass (wird nicht gespeichert).
    filter: [f32; 2],
    /// Ausgang der Aufnahme aus der Engine, solange keine Aufnahme läuft.
    record_rx: Option<crate::recorder::FrameRx>,
    recording: Option<crate::recorder::Recording>,
    /// Verlorene Frames beim Start der Aufnahme (Zähler der Engine), für den Hinweis am Ende.
    record_dropped_at_start: u64,
    /// Zählt hoch, wenn die Audio-Ausgabe neu startet (dann gehört ein alter Aufnahme-Puffer zur alten Engine).
    audio_gen: u64,
    /// Fehlerzähler der Audio-Ausgabe beim letzten Blick und Zeitpunkt des letzten Neustarts (`check_audio_errors`).
    audio_errors_seen: (u64, f64),
    audio_restarted: f64,
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
    /// Uhr für [`eframe::App::logic`]: bei verdecktem Fenster bleibt die egui-Zeit stehen.
    epoch: std::time::Instant,
    /// Abstand der egui-Zeit zu `epoch`, damit beide Uhren gleich gehen.
    clock_offset: f64,
    last_logic: f64,
    dev: DevTools,
    settings: Settings,
    settings_ui: SettingsUi,
    /// Befehlssuche (⌘K).
    palette: crate::palette::Palette,
    /// Rückfrage zu „Alles leeren“ ist offen.
    confirm_clear_all: bool,
    tag_editor: Option<TagEditor>,
    tag_batch: Option<crate::tag_batch::TagBatch>,
    save_due: Option<f64>,
    /// Vollbild beim nächsten Bild umschalten (Taste im Browser).
    toggle_fullscreen: bool,
    /// Einstellungen und Playlists speichern? Bei Entwicklerläufen nicht.
    persist: bool,
    /// Stummschalten anderer Geräusche (`mute_others`), läuft in einem eigenen Thread.
    quiet: Option<crate::quiet::Quiet>,
    session_saved: u64,
    session_checked: f64,
    /// Ordnerbrowser im letzten Bild (ändert sich nur der Browser, hat der Nutzer ihn gezogen): (rechts, Fensterhöhe bzw. -breite, Höhe bzw. Breite des Browsers).
    last_sizes: (bool, f32, f32),
    /// Titel, die gerade aus dem ausgeklinkten Ordnerfenster gezogen werden.
    cross: crate::crossdrag::CrossDrag,
    /// Lage des Ordnerfensters beim Öffnen (danach gehört sie dem Nutzer).
    browser_window_init: Option<[f32; 4]>,
    /// Wie viele Bilder das Ordnerfenster schon auf keinem Bildschirm liegt.
    browser_window_lost: u32,
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
    pub fn new(cc: &eframe::CreationContext<'_>, mut settings: Settings, start_dir: Option<PathBuf>) -> Self {
        i18n::set_lang(settings.language);
        theme::set_skin(settings.skin);
        theme::set_brushed(settings.brushed_steel);
        theme::set_lite(settings.lite);
        // Für die Fehlersuche: welche Grafikkarte und welcher Treiber tatsächlich zeichnen
        let mut software_wgpu = false;
        let mut lean_for_software_gl = false;
        if let Some(rs) = &cc.wgpu_render_state {
            let a = rs.adapter.get_info();
            log::info!("wgpu: {} ({:?}, {:?}, Treiber {} {})", a.name, a.backend, a.device_type, a.driver, a.driver_info);
            software_wgpu = a.device_type == eframe::egui_wgpu::wgpu::DeviceType::Cpu;
        }
        if let Some(gl) = &cc.gl {
            use eframe::glow::HasContext as _;
            // SAFETY: nur Abfragen, der Kontext ist während des Erzeugens der App aktiv
            let (renderer, version) = unsafe { (gl.get_parameter_string(eframe::glow::RENDERER), gl.get_parameter_string(eframe::glow::VERSION)) };
            log::info!("OpenGL: {renderer} ({version})");
            // Mac ohne Metal, und auch OpenGL zeichnet nur der Prozessor (keine beschleunigte Grafik): Dort braucht
            // ein Bild der vollen Optik über eine Sekunde (MacBook Pro 2008 im Software-Renderer), sparsam einstellen.
            // Mit Grafikkarte (dort OpenGL 3.3) läuft die volle Optik flüssig und bleibt
            if crate::diagnose::NO_METAL_START.load(std::sync::atomic::Ordering::Relaxed) && renderer.contains("Software") {
                settings.lite = theme::Lite::ALL;
                settings.platters = false;
                theme::set_lite(settings.lite);
                lean_for_software_gl = true;
            }
        }
        crate::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.add_plugin(crate::ctrl_click::CtrlClick::default());
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
            min_window: None,
            small_screen: false,
            screen_known: false,
            deferred_loads: Vec::new(),
            // FW_SLIDE=1 (nur Debug): Ordnerbereich der kompakten Ansicht gleich eingeschoben
            browser_slid: cfg!(debug_assertions) && std::env::var_os("FW_SLIDE").is_some(),
            browser_slid_for_drag: false,
            ctl,
            lib,
            decks: Default::default(),
            playlists: Default::default(),
            browser: {
                let mut b = Browser::new(start_dir, &settings.folders, settings.tree_width, &settings.folder_recursion);
                b.set_columns(&settings.track_columns, &settings.column_widths);
                b.set_sort(settings.track_sort.as_ref());
                b.tree_height = settings.tree_height;
                b.folder_columns = settings.folder_columns;
                b.columns_share = settings.columns_share;
                b.set_search_memory(settings.remember_search, settings.hide_duplicates, settings.search_whole_words, &settings.advanced_search);
                b
            },
            metas: HashMap::new(),
            data_gen: 0,
            meta_requested: HashSet::new(),
            meta_urgent: HashSet::new(),
            bpms: HashMap::new(),
            keys: HashMap::new(),
            scan_job: None,
            fadewerk: false,
            bpm_pending: HashSet::new(),
            bpm_total: 0,
            bpm_check: None,
            key_check: None,
            harmonic_pending: false,
            sort_undo: None,
            fade_track: None,
            split_sent: None,
            autofade: { let mut a = AutoFade::default(); a.enabled = settings.auto_fade; a.fade_secs = settings.fade_secs; a },
            crossfader: -1.0,
            curve: curve_from(settings.curve, settings.curve_points),
            motor: Motor { spin_up_secs: settings.spin_up_secs, spin_down_secs: settings.spin_down_secs },
            head_mode: HeadMode::Off,
            head_mix: settings.head_mix,
            head_volume: settings.head_volume,
            master_volume: settings.master_volume,
            fx: {
                let bank = crate::fx::FxBank::new();
                for slot in 0..fw_engine::FX_SLOTS {
                    bank.load(slot, settings.fx_slot(slot).file);
                }
                bank
            },
            fx_volume: settings.fx_volume,
            eq_db: settings.eq_db,
            eq_kill: [[false; 3]; 2],
            filter: [0.0; 2],
            record_rx: None,
            recording: None,
            record_dropped_at_start: 0,
            audio_gen: 0,
            audio_errors_seen: (0, 0.0),
            audio_restarted: f64::NEG_INFINITY,
            eq_sent: [[f32::NAN; 3]; 2],
            bass_swap: settings.bass_swap,
            auto_sync: settings.auto_sync || std::env::var_os("FW_AUTOSYNC").is_some(),
            meters: Default::default(),
            bands: Default::default(),
            preview: None,
            next_token: 1,
            notice: None,
            last_time: 0.0,
            epoch: std::time::Instant::now(),
            clock_offset: 0.0,
            last_logic: 0.0,
            dev: DevTools::from_env(),
            settings,
            caches: Caches::default(),
            update: UpdateState::default(),
            wake: WakeLock::default(),
            sketch: Sketches::default(),
            settings_ui: SettingsUi::default(),
            palette: Default::default(),
            confirm_clear_all: false,
            tag_editor: None,
            tag_batch: None,
            save_due: None,
            toggle_fullscreen: false,
            persist: true,
            quiet: None,
            session_saved: 0,
            session_checked: 0.0,
            last_sizes: (false, 0.0, 0.0),
            cross: Default::default(),
            browser_window_init: None,
            browser_window_lost: 0,
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
            _ if dev_ask => app.update.ask = true,
            _ if dev_update => app.update.rx = Some(crate::update::check_in_background(cc.egui_ctx.clone())),
            None if app.persist => app.update.ask = true,
            Some(true) if app.persist => app.update.rx = Some(crate::update::check_in_background(cc.egui_ctx.clone())),
            _ => {}
        }
        // Linux: Findet wgpu nur einen Software-Renderer (llvmpipe, etwa ältere Intel-Grafik ohne Vulkan), ist
        // OpenGL über den echten Treiber meist viel schneller (Netbook mit GMA 950: ~150 % statt ~100 % CPU).
        // Einmalig umstellen und neu starten. Nicht unter Windows: dort ist der Software-Renderer (WARP in VMs,
        // Remote Desktop) oft noch besser als OpenGL ohne Treiber. Nicht bei FW_RENDERER (bewusst gewählt).
        if cfg!(target_os = "linux") && software_wgpu && app.persist && std::env::var_os("FW_RENDERER").is_none() {
            log::warn!("wgpu zeichnet nur in Software, stelle auf OpenGL um und starte neu");
            app.settings.opengl = true;
            app.save_settings();
            match crate::self_update::restart_after_exit(crate::self_update::AFTER_OPENGL_FLAG) {
                Ok(()) => cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => log::warn!("Neustart für OpenGL nicht möglich: {e}"),
            }
        }
        if std::env::args().any(|a| a == crate::self_update::AFTER_OPENGL_FLAG) {
            app.notice = Some((tr("Die Grafik zeichnete nur in Software. FadeWerk DJ nutzt jetzt OpenGL (abschaltbar unter Einstellungen → Leistung).").to_owned(), 0.0));
        }
        if lean_for_software_gl {
            app.save_settings();
            app.notice = Some((tr("Auf diesem Mac zeichnet nur der Prozessor. FadeWerk DJ zeichnet deshalb sparsam (umstellbar unter Einstellungen → Leistung).").to_owned(), 0.0));
        }
        if std::env::args().any(|a| a == crate::self_update::AFTER_RESET_FLAG) {
            app.notice = Some((tr("FadeWerk DJ wurde zurückgesetzt.").to_owned(), 0.0));
        }
        if std::env::args().any(|a| a == crate::self_update::AFTER_UPDATE_FLAG) {
            app.notice = Some((trf("FadeWerk DJ wurde auf Version {} aktualisiert.", &[&env!("CARGO_PKG_VERSION")]), 0.0));
        }
        if app.persist && app.settings.fullscreen {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        if app.persist {
            app.history = History::load();
        }
        if app.persist {
            app.caches.bpm = crate::analysis_cache::BpmCache::load();
            app.bpms.extend(app.caches.bpm.all().map(|(p, b)| (p.clone(), b.bpm)));
            app.bpm_check = Some(app.caches.bpm.start_check());
            app.caches.key = crate::analysis_cache::KeyCache::load();
            app.keys.extend(app.caches.key.all().map(|(p, k)| (p.clone(), k.camelot())));
            app.key_check = Some(app.caches.key.start_check());
            app.caches.tag = crate::tag_cache::TagCache::load();
            app.caches.folder = crate::folder_cache::FolderCache::load();
            log::info!("Tag-Cache: {} Titel", app.caches.tag.len());
            let dir = crate::settings::config_dir().map(|d| d.join("wellenformen"));
            if let Some(dir) = dir.clone() {
                let _ = std::thread::Builder::new().name("fw-wave-clean".into()).spawn(move || {
                    let n = fw_library::sketch::clean_cache(&dir);
                    if n > 0 {
                        log::info!("Wellenform-Cache: {n} alte Einträge entfernt");
                    }
                });
            }
            app.lib.set_sketch_cache(dir);
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
        app.start_head_audio(0.0, true);
        // Entwicklerläufe schalten nichts am System stumm, außer mit FW_QUIET=1
        let quiet_test = std::env::var_os("FW_QUIET").is_some();
        if app.persist || quiet_test {
            app.quiet = Some(crate::quiet::Quiet::start(crate::settings::config_dir(), app.settings.mute_others));
            app.apply_exclusive(0.0);
        }
        if let Some(s) = app.dev.skin {
            theme::set_skin(s);
            theme::apply(&cc.egui_ctx);
        }
        if app.dev.steel {
            theme::set_brushed(true);
            theme::apply(&cc.egui_ctx);
        }
        // FW_CLEAR_ALL=1 (nur Debug): Rückfrage „Alles leeren“ gleich offen
        app.confirm_clear_all = cfg!(debug_assertions) && std::env::var_os("FW_CLEAR_ALL").is_some();
        if let Some(q) = app.dev.palette.clone() {
            app.palette.open_with(&q);
        }
        if app.dev.view.is_some() {
            app.settings.view = app.dev.view;
        }
        if app.dev.lite {
            app.settings.lite = theme::Lite::ALL;
            theme::set_lite(theme::Lite::ALL);
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
            // FW_PLATTERS=0: Player ohne Plattenteller
            if std::env::var("FW_PLATTERS").is_ok_and(|v| v == "0") {
                app.settings.platters = false;
            }
            // FW_FADEWERK=1: Playlists im FadeWerk-Modus; FW_FADEWERK=edit setzt Beispielbereiche in
            // Playlist B und öffnet den Editor für deren zweiten Titel
            if let Ok(v) = std::env::var("FW_FADEWERK") {
                app.fadewerk = true;
                if v == "edit" && app.playlists[1].entries.len() > 1 {
                    use crate::fadewerk::{Region, TrackFade};
                    app.playlists[1].set_fade(0, TrackFade { fade_in: Some(Region { start: 1.5, end: 7.0 }), fade_out: Some(Region { start: 186.0, end: 196.0 }) });
                    app.playlists[1].set_fade(1, TrackFade { fade_in: Some(Region { start: 2.5, end: 9.0 }), fade_out: Some(Region { start: 192.0, end: 192.0 }) });
                    // FW_EDIT_INDEX wählt den Eintrag (Standard 1)
                    let k = std::env::var("FW_EDIT_INDEX").ok().and_then(|v| v.parse().ok()).filter(|&k: &usize| k < app.playlists[1].entries.len()).unwrap_or(1);
                    playlist_ui::demo_open_editor(&cc.egui_ctx, 1, k, app.playlists[1].entries[k].clone());
                }
                // FW_FADEWERK=auto: kurze Bereiche, damit die Überblendung von A nach B nach 3 s beginnt
                if v == "auto" && !app.playlists[0].entries.is_empty() && !app.playlists[1].entries.is_empty() {
                    use crate::fadewerk::{Region, TrackFade};
                    app.playlists[0].set_fade(0, TrackFade { fade_in: None, fade_out: Some(Region { start: 6.0, end: 10.0 }) });
                    app.playlists[1].set_fade(0, TrackFade { fade_in: Some(Region { start: 3.0, end: 6.0 }), fade_out: None });
                }
            }
            // FW_BROWSER=rechts: Ordnerbrowser rechts neben den Playern, FW_BROWSER=fenster: in eigenem Fenster
            if std::env::var("FW_BROWSER").is_ok_and(|v| v == "rechts") {
                app.settings.browser_side = true;
            }
            if std::env::var("FW_BROWSER").is_ok_and(|v| v == "fenster") {
                app.settings.browser_detached = true;
            }
            // FW_FOLDERS=spalten: Ordner in Spalten statt als Baum
            if std::env::var("FW_FOLDERS").is_ok_and(|v| v == "spalten") {
                app.browser.folder_columns = true;
            }
            // FW_TAB=kopfhoerer|fade|motor|zu: Reiter unter dem Crossfader
            if let Ok(tab) = std::env::var("FW_TAB") {
                app.settings.center_tab = (tab != "zu").then_some(tab);
            }
            // FW_COLUMNS=album,genre,year: andere Spalten der Titelliste zeigen
            if let Ok(cols) = std::env::var("FW_COLUMNS") {
                let keys: Vec<String> = cols.split(',').map(|k| k.trim().to_owned()).collect();
                app.browser.set_columns(&keys, &[]);
            }
            // FW_SORT=artist oder FW_SORT=bpm:desc: Titelliste sortiert zeigen
            if let Ok(spec) = std::env::var("FW_SORT") {
                let (key, desc) = spec.split_once(':').map_or((spec.as_str(), false), |(k, d)| (k, d == "desc"));
                app.browser.set_sort(Some(&(key.to_owned(), desc)));
            }
            // FW_TAGS=1: Fenster „Titelinfo & Tags“ für den ersten Titel öffnen
            if std::env::var_os("FW_TAGS").is_some()
                && let Some(t) = tracks.first()
            {
                let mut ed = TagEditor::open(t.clone());
                // FW_ONLINE=1: gleich online suchen (ohne Rückfrage)
                if std::env::var_os("FW_ONLINE").is_some() {
                    ed.start_search(&cc.egui_ctx);
                }
                app.tag_editor = Some(ed);
            }
            // FW_BATCH=1: Ordner-Durchlauf „Tags online ergänzen“ für die Demo-Titel (mit FW_ONLINE=1 ohne Rückfrage)
            if std::env::var_os("FW_BATCH").is_some() {
                let mut batch = crate::tag_batch::TagBatch::new(tracks.clone());
                // FW_BATCH=hintergrund: gleich im Hintergrund (Stand in der Browser-Kopfzeile)
                if std::env::var("FW_BATCH").is_ok_and(|v| v == "hintergrund") {
                    batch.send_to_background();
                }
                app.tag_batch = Some(batch);
                if std::env::var_os("FW_ONLINE").is_some() {
                    app.settings.online_lookup = true;
                }
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
        app.record_rx = app.ctl.take_recording();
        app
    }

    fn notify(&mut self, text: impl Into<String>, now: f64) {
        self.notice = Some((text.into(), now + 3.0));
    }

    fn has_track(&self, d: usize) -> bool {
        self.decks[d].loaded_id != 0
    }

    /// Ist der Player gerade über Master zu hören?
    /// Bewegt sich gerade etwas auf dem Bildschirm (Teller, Wellenform, Pegel, Überblendung)?
    /// Sonst braucht die Oberfläche nicht ständig neu gezeichnet zu werden.
    fn is_moving(&self) -> bool {
        self.ctl.shared.decks.iter().any(|s| s.is_playing() || s.speed() != 0.0)
            || self.decks.iter().any(|v| v.scratching)
            || self.autofade.fade().is_some()
            || !self.meters.iter().all(Meter::is_dark)
            || !self.bands.is_dark()
            || self.ctl.shared.fx_playing() != 0
    }

    /// Hält Bildschirm und Rechner wach, solange ein Player oder das Vorhören spielt (und kurz danach).
    fn keep_awake(&mut self, now: f64) {
        if self.ctl.shared.decks.iter().any(|s| s.is_playing()) || self.recording.is_some() {
            self.wake.last_sound = now;
        }
        let want = self.settings.keep_awake && now - self.wake.last_sound < AWAKE_HOLD_SECS && self.wake.last_sound > 0.0;
        if !want {
            // Freigeben gibt die Sperre beim System zurück
            self.wake.lock = None;
            self.wake.failed = false;
        } else if self.wake.lock.is_none() && !self.wake.failed {
            let created = keepawake::Builder::default()
                .display(true)
                .idle(true)
                .reason(tr("Musik spielt"))
                .app_name("FadeWerk DJ")
                .app_reverse_domain("de.eckonator.fadewerkdj")
                .create();
            match created {
                Ok(a) => self.wake.lock = Some(a),
                Err(e) => {
                    log::warn!("Ruhezustand ließ sich nicht verhindern: {e}");
                    self.wake.failed = true;
                }
            }
        }
    }

    // ---------- Laden ----------

    // ---------- Titelende und Auto-Fade ----------

    // ---------- Aktionen ----------

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

/// Leiste „ORDNER“ am linken Rand der kompakten Ansicht: senkrechte Aufschrift, Klick schiebt den Ordnerbereich
/// ein oder aus.
fn folder_strip(ui: &mut egui::Ui, rect: Rect, open: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with("browser_strip"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 0.0, theme::row_hover());
    }
    let color = if open { theme::select_accent() } else if resp.hovered() { theme::ink() } else { theme::dim() };
    // Von unten nach oben lesbar, mittig in der Leiste
    let galley = p.layout_no_wrap(tr("ORDNER").to_owned(), crate::fonts::bold(13.0), color);
    let size = galley.size();
    let pos = pos2(rect.center().x - size.y / 2.0, rect.center().y + size.x / 2.0);
    p.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(-std::f32::consts::FRAC_PI_2));
    // Winkel über und unter der Schrift zeigen, wohin der Bereich beim Klick gleitet: zu nach rechts, offen nach links
    let dir = if open { -1.0 } else { 1.0 };
    let (w, h) = (3.5, 5.0);
    for y in [rect.center().y - size.x / 2.0 - 14.0, rect.center().y + size.x / 2.0 + 14.0] {
        let tip = pos2(rect.center().x + dir * w, y);
        let back = rect.center().x - dir * w;
        p.line(vec![pos2(back, y - h), tip, pos2(back, y + h)], Stroke::new(1.8, color));
    }
    let tip = if cfg!(target_os = "macos") { tr("Ordner einblenden oder ausblenden (⌘B)") } else { tr("Ordner einblenden oder ausblenden (Strg+B)") };
    resp.on_hover_text(tip)
}

/// Textur für das Cover auf dem Plattenteller von Player `d`.
fn cover_texture(ctx: &egui::Context, d: usize, c: &fw_library::Cover) -> egui::TextureHandle {
    let img = egui::ColorImage::from_rgba_unmultiplied([c.size as usize; 2], &c.rgba);
    ctx.load_texture(
        format!("cover-{d}"),
        img,
        // Mit Mipmaps flimmert das verkleinerte, sich drehende Bild nicht
        egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
    )
}

fn panel_frame(ui: &egui::Ui) -> Frame {
    crate::hw::panel(ui.painter(), ui.max_rect().shrink(3.0));
    Frame::new().inner_margin(Margin::same(15))
}

impl eframe::App for FadeWerkDjApp {
    /// Alles, was auch ohne sichtbares Fenster weiterlaufen muss: Titelende, Auto-Fade, Laden, Historie,
    /// Speichern. eframe ruft `ui` bei verdecktem oder minimiertem Fenster nicht auf, `logic` aber schon
    /// (davor blieb die Musik nach dem laufenden Titel stehen, wenn eine andere App das Fenster überdeckte).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let hidden = ctx.input(|i| i.viewport().visible()) == Some(false);
        let clock = self.epoch.elapsed().as_secs_f64();
        // Bei sichtbarem Fenster gilt die egui-Zeit; verdeckt bleibt sie stehen, dann läuft die eigene Uhr weiter
        if !hidden {
            self.clock_offset = ctx.input(|i| i.time) - clock;
        }
        let now = clock + self.clock_offset;
        let dt = (now - self.last_logic).clamp(0.0, LOGIC_MAX_DT) as f32;
        self.last_logic = now;

        self.check_audio_errors(now);
        self.poll_library(ctx, now);
        self.poll_fx(now);
        self.poll_fade_preview(now);
        self.poll_import(now);
        self.poll_recording(now);
        self.record_plays();
        self.keep_awake(now);
        self.handle_track_ends();
        // „Harmonisch“ wartet, bis die Analyse durch ist (oder abgebrochen wurde)
        if self.harmonic_pending && self.bpm_pending.is_empty() {
            self.harmonic_pending = false;
            self.harmonic_sort(now);
        }
        self.step_autofade(dt);
        self.step_auto_sync(dt, now);
        self.push_eq();
        // Playlists alle zwei Sekunden sichern, falls sich etwas geändert hat (auch für Abstürze)
        if now - self.session_checked > 2.0 {
            self.session_checked = now;
            self.save_session_if_changed();
            if self.persist {
                self.caches.bpm.save_if_dirty();
                self.caches.key.save_if_dirty();
                self.caches.tag.save_if_dirty(false);
                self.caches.folder.save_if_dirty();
                self.history.save_dirty();
            }
        }
        if let Some(rx) = &self.bpm_check
            && let Ok(results) = rx.try_recv()
        {
            self.bpm_check = None;
            let removed = self.caches.bpm.apply_check(results);
            if removed > 0 {
                log::info!("BPM-Cache: {removed} Einträge ohne Datei entfernt");
            }
        }
        if let Some(rx) = &self.key_check
            && let Ok(results) = rx.try_recv()
        {
            self.key_check = None;
            let removed = self.caches.key.apply_check(results);
            if removed > 0 {
                log::info!("Tonart-Cache: {removed} Einträge ohne Datei entfernt");
            }
        }
        self.ctl.collect_garbage();
        if hidden {
            ctx.request_repaint_after(if self.is_moving() { HIDDEN_TICK } else { IDLE_FRAME });
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.dev.raw_input(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Die ersten Bilder sind gezeichnet: Der Grafikstart hat geklappt
        if ctx.cumulative_frame_nr() >= 2 {
            crate::diagnose::graphics_running();
        }
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = now;

        self.fit_screen(&ctx);
        self.fx_keys(&ctx, now);
        let levels = self.ctl.shared.take_levels();
        for (m, l) in self.meters.iter_mut().zip(levels) {
            m.update(l, dt, now);
        }
        // Ohne Anzeige der Bänder rechnet der Audio-Thread sie auch nicht
        self.ctl.shared.set_spectrum(!theme::lite().no_bands);
        self.bands.update(self.ctl.shared.take_bands(), dt, now);

        // Aktionen aus Playern, Mixer und Playlists (Hauptbereich, siehe `views`)
        let mut vout = views::ViewOut::default();
        let mut browser_actions = Vec::new();

        // Neu geöffneter Ordner: Titel samt Unterordnern im Hintergrund suchen
        if let Some(dirs) = self.browser.take_pending_scan() {
            let token = self.token();
            self.browser.begin_scan(token);
            if let Some(cached) = self.caches.folder.get(&dirs) {
                self.browser.prefill(cached);
            }
            self.scan_job = Some((token, dirs.clone(), std::time::Instant::now()));
            self.lib.scan(token, dirs);
        }

        // Statuszeile über allem: was läuft, was kommt, Arbeitsweise, Befehlssuche, Einstellungen
        let transition = self.transition_view(self.fade_status());
        let status_model = self.status_model(&transition, &ctx);
        let mut status_actions = Vec::new();
        let status_rect = egui::Panel::top("statusbar")
            .exact_size(crate::statusbar::STATUS_H)
            .show_separator_line(false)
            .frame(Frame::new().fill(theme::panel()).stroke(Stroke::new(1.0, theme::line())).inner_margin(Margin::symmetric(10, 0)))
            .show(ui, |ui| crate::statusbar::show(ui, &status_model, &mut status_actions))
            .response
            .rect;

        // Ordnerbrowser unten oder rechts; in der kompakten Ansicht gleitet er von links über die Oberfläche
        let compact = self.compact();
        // Automatik: der Ordnerbereich gleitet wie in der kompakten Ansicht von links herein
        let auto_view = self.view() == ViewMode::Auto;
        let slide = (compact || auto_view) && !self.settings.browser_detached;
        if slide {
            // Ein Titel wird gezogen: Player und Playlists freigeben, nach dem Ablegen wieder herein
            let dragging_item = egui::DragAndDrop::has_any_payload(&ctx);
            if self.browser_slid && dragging_item {
                self.browser_slid = false;
                self.browser_slid_for_drag = true;
            } else if self.browser_slid_for_drag && !dragging_item {
                self.browser_slid_for_drag = false;
                self.browser_slid = true;
            }
            // ⌘B / Strg+B schiebt ein und aus, Esc schiebt zurück (nicht beim Tippen)
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
                self.browser_slid = !self.browser_slid;
            }
            if self.browser_slid && ctx.memory(|m| m.focused().is_none()) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.browser_slid = false;
            }
        } else {
            self.browser_slid = false;
            self.browser_slid_for_drag = false;
        }
        let mut slid = self.browser_slid;
        let preview_st = &self.ctl.shared.decks[PREVIEW];
        let bctx = BrowserCtx {
            metas: &self.metas,
            bpms: &self.bpms,
            keys: &self.keys,
            bpm_progress: (self.bpm_total > 0).then(|| (self.bpm_total - self.bpm_pending.len(), self.bpm_total)),
            tag_batch: self.tag_batch.as_ref().and_then(|b| b.status()),
            preview: self.preview.as_ref().filter(|p| p.clip.is_none()).map(|p| p.path.as_path()),
            preview_pos: preview_st.position_secs(),
            preview_dur: preview_st.duration_secs(),
            deck_playing: [self.ctl.shared.decks[0].is_playing(), self.ctl.shared.decks[1].is_playing()],
            side: false,
            side_possible: false,
            compact: slide,
            detached: self.settings.browser_detached,
            history: &self.history,
            importing: self.import_rx.is_some(),
            data_gen: self.data_gen,
        };
        // Höhe merken; ohne gespeicherten Wert 240 pt, auf hohen Monitoren mehr (bei 1400 pt Höhe etwa 450),
        // auf niedrigen weniger (ein gutes Viertel der Höhe)
        let total_h = ui.available_height();
        let total_w = ui.available_width();
        let side = self.settings.browser_side && total_w >= SIDE_MAIN_MIN + SIDE_BROWSER_MIN;
        // Im eigenen Fenster wählt der Nutzer: Ordner über den Titeln (wie rechts) oder daneben (wie unten)
        let side = if self.settings.browser_detached { !self.settings.browser_window_tree_left } else { side };
        let bctx = BrowserCtx { side, side_possible: total_w >= SIDE_MAIN_MIN + SIDE_BROWSER_MIN, ..bctx };
        let browser_frame = Frame::new().fill(theme::panel()).stroke(Stroke::new(1.0, theme::line())).inner_margin(Margin::same(10));
        let browser = &mut self.browser;
        let mut show_browser = |ui: &mut egui::Ui| {
            if theme::brushed() {
                let p = ui.ctx().layer_painter(ui.layer_id());
                let face = crate::hw::steel_face(theme::panel(), theme::skin().dark);
                crate::hw::surface(&p, ui.max_rect().expand(10.0), 0.0, crate::hw::lighten(face, 0.04), crate::hw::darken(face, 0.18));
            } else if theme::alu() {
                // Aluminium: gestrahltes Korn, leicht heller oben
                let p = ui.ctx().layer_painter(ui.layer_id());
                let face = theme::panel();
                crate::hw::surface(&p, ui.max_rect().expand(10.0), 0.0, crate::hw::lighten(face, if theme::skin().dark { 0.03 } else { 0.15 }), crate::hw::darken(face, 0.06));
            }
            browser.show(ui, &bctx, &mut browser_actions)
        };
        // Rechts: ohne gespeicherten Wert gut ein Viertel der Breite (bei 2560 pt etwa 680)
        let mut detach_closed = false;
        let mut detached_pads: Vec<usize> = Vec::new();
        let size = if self.settings.browser_detached {
        // Eigenes Fenster: läuft im Takt des Hauptfensters mit (egui-„immediate viewport“), Eingaben dort
        // zeichnen beide neu. Ziehen ins Hauptfenster über `crossdrag`.
            let zoom = ctx.zoom_factor();
            // Lage nur beim Öffnen vorgeben; danach verschiebt und vergrößert der Nutzer das Fenster
            let geo = *self.browser_window_init.get_or_insert(self.settings.browser_window);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(tr("FadeWerk DJ – Ordner"))
                .with_min_inner_size([SIDE_BROWSER_MIN, 300.0])
                .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).unwrap_or_default());
            builder = if geo[2] > 0.0 {
                builder.with_position([geo[0] / zoom, geo[1] / zoom]).with_inner_size([geo[2] / zoom, geo[3] / zoom])
            } else {
                builder.with_inner_size([DETACHED_DEFAULT_W, 640.0])
            };
            let cross = &mut self.cross;
            let window_geo = &mut self.settings.browser_window;
            let save_due = &mut self.save_due;
            let lost = &mut self.browser_window_lost;
            let pads = &mut detached_pads;
            let main_pos = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min);
                        let closed = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("fw-ordner"), builder, |ui, _class| {
                let wctx = ui.ctx().clone();
                egui::CentralPanel::default().frame(browser_frame).show(ui, |ui| {
                    show_browser(ui);
                });
                cross.track_source(&wctx);
                // Effekt-Pads auch, wenn das Ordnerfenster vorn ist
                *pads = pad_keys(&wctx);
                // Pfeil- und Eingabetasten für die Titelliste kommen in diesem Fenster an
                crate::keynav::collect(&wctx);
                let info = wctx.input(|i| i.viewport().clone());
                // Lage merken, sobald sie sich ändert (in Punkten des Systems, damit der Zoom nicht mitspielt)
                if let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) {
                    let now_geo = [outer.min.x * zoom, outer.min.y * zoom, inner.width() * zoom, inner.height() * zoom].map(f32::round);
                    if now_geo != *window_geo {
                        *window_geo = now_geo;
                        *save_due = Some(now + 1.0);
                    }
                }
                // Liegt das Fenster auf keinem Bildschirm mehr (zweiter Monitor fehlt), zum Hauptfenster holen
                if info.monitor_size.is_none() && info.inner_rect.is_some() {
                    *lost += 1;
                    if *lost == 10
                        && let Some(p) = main_pos
                    {
                        wctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p + vec2(60.0, 60.0)));
                    }
                } else {
                    *lost = 0;
                }
                info.close_requested()
            });
            detach_closed = closed;
            0.0
        } else if slide {
            // Leiste „ORDNER“ am linken Rand; der Bereich gleitet darüber hinaus über die ganze Oberfläche
            egui::Panel::left("browser_strip")
                .resizable(false)
                .exact_size(SLIDE_STRIP_W)
                .show_separator_line(false)
                .frame(Frame::new().fill(theme::panel()).stroke(Stroke::new(1.0, theme::line())))
                .show(ui, |ui| {
                    if folder_strip(ui, ui.max_rect(), slid).clicked() {
                        slid = !slid;
                    }
                });
            let t = ctx.animate_bool_with_time(egui::Id::new("browser_slide"), slid, if theme::lite().frame_limit { 0.0 } else { SLIDE_SECS });
            if t > 0.0 {
                // Unter der Statuszeile, die zeigt weiter, was läuft
                let full = ctx.content_rect();
                let area = Rect::from_min_max(pos2(full.left() + SLIDE_STRIP_W, status_rect.bottom()), full.max);
                let x = area.left() - (1.0 - t) * area.width();
                egui::Area::new(egui::Id::new("browser_slide_area")).fixed_pos(pos2(x, area.top())).order(egui::Order::Middle).constrain(false).show(&ctx, |ui| {
                    ui.set_clip_rect(area);
                    let rect = Rect::from_min_size(pos2(x, area.top()), area.size());
                    // Fängt Klicks ab, damit nichts darunter reagiert
                    ui.allocate_rect(rect, Sense::click_and_drag());
                    ui.painter().rect_filled(rect, 0.0, theme::panel());
                    // Kante beim Hereingleiten sichtbar machen
                    ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, theme::line()));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(10.0)), |ui| show_browser(ui));
                });
            }
            0.0
        } else if side {
            let browser_w = if self.settings.browser_width > 0.0 { self.settings.browser_width } else { (total_w * 0.27).max(SIDE_BROWSER_MIN) };
            egui::Panel::right("browser_side")
                .resizable(true)
                .default_size(browser_w)
                .min_size(SIDE_BROWSER_MIN)
                .max_size(total_w - SIDE_MAIN_MIN)
                .frame(browser_frame)
                .show(ui, show_browser)
                .response
                .rect
                .width()
        } else {
            let browser_h = if self.settings.browser_height > 0.0 {
                self.settings.browser_height
            } else if total_h >= 920.0 {
                240.0 + (total_h - 920.0) * 0.44
            } else {
                (total_h * 0.26).max(160.0)
            };
            egui::Panel::bottom("browser")
                .resizable(true)
                .default_size(browser_h)
                .min_size(160.0)
                .max_size((total_h - DECK_ROW_MIN - PLAYLIST_MIN - 30.0).max(160.0))
                .frame(browser_frame)
                .show(ui, show_browser)
                .response
                .rect
                .height()
        };
        if detach_closed {
            browser_actions.push(BrowserAction::Detach(false));
        }
        for slot in detached_pads {
            self.trigger_fx(slot, true, now);
        }
        self.cross.publish(&ctx);
        let content = ctx.content_rect();
        let sizes = (side, if side { content.width() } else { content.height() }.round(), size.round());
        let dragging = dragging_edge(&ctx) && !self.settings.browser_detached && !slide;
        let last = self.last_sizes;
        if dragging && last.0 == sizes.0 && last.1 == sizes.1 && last.2 > 0.0 && (last.2 - sizes.2).abs() >= 1.0 {
            if side {
                self.settings.browser_width = sizes.2;
            } else {
                self.settings.browser_height = sizes.2;
            }
            self.save_due = Some(now + 1.0);
        }
        self.last_sizes = sizes;

        // Ausgeworfene Platte ist hinausgeglitten (oder schon eine neue geladen)
        for v in &mut self.decks {
            if v.ejected.as_ref().is_some_and(|e| now - e.3 > 1.0) || v.loaded_id != 0 {
                v.ejected = None;
            }
        }
        let shown: Vec<f64> = (0..2)
            .map(|d| {
                let st = &self.ctl.shared.decks[d];
                let duration = st.duration_secs();
                self.decks[d].smooth_position(st.position_secs(), st.rate() as f64, now, dt as f64).clamp(0.0, duration.max(0.0))
            })
            .collect();
        let deck_keys = [self.deck_key(0), self.deck_key(1)];
        let key_ref = self.key_reference(&deck_keys);
        let harmonic = self.harmonic_state(now);
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
                    filter: self.filter[d],
                    auto_synced: v.sync.is_some(),
                    position: if in_sync { shown[d] } else { 0.0 },
                    duration: if in_sync { st.duration_secs() } else { v.meta.as_ref().map_or(0.0, |m| m.duration_secs) },
                    playing: in_sync && st.is_playing(),
                    speed: if in_sync { st.speed() } else { 0.0 },
                    has_track: v.loaded_id != 0,
                    // Spielt schon der Anfang, gilt der Player nicht mehr als ladend (Name und Zeiten stehen da)
                    loading: v.pending.as_ref().is_some_and(|p| !p.started),
                    waveform: v.waveform.as_deref(),
                    cover: v.cover.as_ref().map(|t| t.id()),
                    leaving: v.ejected.as_ref().map(|(title, artist, cover, _)| deck_ui::Leaving { title, artist, cover: cover.as_ref().map(|t| t.id()) }),
                    pfl: v.pfl,
                    reverse: v.reverse,
                    repeat: v.repeat,
                    loop_beats: v.loop_beats,
                    loop_range: v.loop_range,
                    on_air: self.on_air(d),
                    key: deck_keys[d].0,
                    key_shift: deck_keys[d].1,
                    key_fits: key_ref.is_some_and(|(r, rk)| r != d && deck_keys[d].0.is_some_and(|k| k.compatible(rk))),
                    scratching: v.scratching,
                    platter: self.settings.platters,
                    compact,
                }
            })
            .collect();
        let fx_labels: [String; fw_engine::FX_SLOTS] = std::array::from_fn(|i| self.settings.fx_slot(i).label(i));
        let mixer_model = MixerModel {
            meters: &self.meters,
            bands: &self.bands,
            volume: [self.decks[0].volume, self.decks[1].volume],
            head_mode: self.head_mode,
            head_mix: self.head_mix,
            head_volume: self.head_volume,
            master_volume: self.master_volume,
            audio_ok: self.audio_error.is_none(),
            compact,
            head_device: self.head_audio.is_some(),
            eq_db: self.eq_db,
            eq_kill: self.eq_kill,
            recording: self.recording.as_ref().map(|r| (r.secs(), r.is_stopping(), r.bytes())),
            fx_labels: &fx_labels,
            fx_icons: std::array::from_fn(|i| self.settings.fx_slot(i).icon(i)),
            fx_playing: self.ctl.shared.fx_playing(),
            fx_volume: self.fx_volume,
        };
        let fade_model = FadeModel {
            auto: self.autofade.enabled,
            fade_secs: self.autofade.fade_secs,
            curve: self.curve,
            crossfader: self.crossfader,
            transition: &transition,
            show_transition: !auto_view,
            motor: self.motor,
            bass_swap: self.bass_swap,
            auto_sync: self.auto_sync,
        };
        let key_ref_track = key_ref.and_then(|(r, k)| Some((k, self.decks[r].path.as_deref()?)));
        let editor_audio = self.editor_audio(&ctx);
        let views = views::Views {
            decks: &decks_models,
            mixer: &mixer_model,
            fade: &fade_model,
            transition: &transition,
            playlists: &self.playlists,
            info: playlist_ui::TrackInfo { metas: &self.metas, bpms: &self.bpms, keys: &self.keys, reference: key_ref_track, harmonic, compact },
            has: [self.decks[0].loaded_id != 0, self.decks[1].loaded_id != 0],
            playing: [self.ctl.shared.decks[0].is_playing(), self.ctl.shared.decks[1].is_playing()],
            preview: self.preview.as_ref().filter(|p| p.clip.is_none()).map(|p| p.path.as_path()),
            sketches: self.fadewerk.then_some(&self.sketch.map),
            editor_audio: &editor_audio,
            compact,
            air_deck: status_model.on_air.as_ref().map_or(self.autofade.active, |o| o.deck),
            off_air: std::array::from_fn(|d| self.on_air(1 - d) && !self.on_air(d)),
            deck_height: self.settings.deck_height,
            center_tab: self.settings.center_tab.as_deref().and_then(mixer_ui::CenterTab::from_key),
        };
        egui::CentralPanel::default().frame(Frame::new().fill(theme::ground()).inner_margin(Margin::same(8))).show(ui, |ui| {
            if theme::brushed() {
                crate::hw::brushed_overlay(ui.painter(), ui.max_rect().expand(8.0), 0.0, theme::skin().dark);
            } else if theme::alu() {
                crate::hw::grain_overlay(ui.painter(), ui.max_rect().expand(8.0), 0.0, theme::skin().dark);
            }
            if auto_view {
                views::automatic(ui, &views, &mut vout);
            } else {
                views::live(ui, &views, &mut vout);
            }
        });
        drop(decks_models);
        self.browser_slid = slid;
        for a in status_actions {
            self.status_action(a, now);
        }
        if let Some(h) = vout.deck_height {
            self.settings.deck_height = h;
            self.save_due = Some(now + 1.0);
        }

        for (d, acts) in vout.deck.into_iter().enumerate() {
            for a in acts {
                self.deck_action(d, a, now);
            }
        }
        for (d, item) in vout.drops {
            self.drop_on_deck(d, item, now);
        }
        for a in vout.mixer {
            self.mixer_action(a, now);
        }
        if let Some(t) = vout.center_tab {
            self.settings.center_tab = t.map(|t| t.key().to_owned());
            self.save_settings();
        }
        for a in vout.fade {
            self.fade_action(a, now);
        }
        for (d, acts) in vout.playlist.into_iter().enumerate() {
            for a in acts {
                self.playlist_action(d, a, now);
            }
        }
        // Titel aus dem Ordnerfenster ist abgelegt (oder ins Leere gefallen)
        self.cross.finish(&ctx);
        for a in browser_actions {
            self.browser_action(a, now);
        }

        self.update_ui(&ctx, now);

        // Hinweise stehen in der Statuszeile; hier nur, wie lange sie gelten
        if let Some((text, until)) = self.notice.clone() {
            if until > 0.0 && now > until {
                self.notice = None;
            } else if until == 0.0 {
                self.notice = Some((text, now + 6.0));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((until - now).max(0.05)));
            }
        }

        // Befehlssuche: ⌘K bzw. Strg+K
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::K))) {
            self.palette.open();
        }
        self.show_palette(&ctx, now);
        self.clear_all_dialog(&ctx, now);

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
        // Cache-Größen nur messen, solange die Einstellungen offen sind; beim nächsten Öffnen frisch
        if let Some(rx) = &self.caches.sizes_rx
            && let Ok(sizes) = rx.try_recv()
        {
            self.caches.sizes = Some(sizes);
            self.caches.sizes_rx = None;
        }
        if !self.settings_ui.open {
            self.caches.sizes = None;
        }
        let sctx = SettingsCtx {
            settings: &self.settings,
            active_device: self.audio.as_ref().map(|a| a.device.as_str()),
            active_device_id: self.audio.as_ref().map(|a| a.id.as_str()),
            active_head: self.head_audio.as_ref().map(|h| h.device.as_str()),
            sample_rate: self.ctl.shared.sample_rate,
            channels: self.ctl.shared.channels,
            cache_sizes: self.caches.sizes,
            playing: self.ctl.shared.decks.iter().any(|s| s.is_playing()),
            compact: self.compact(),
        };
        self.settings_ui.show(&ctx, &sctx, &mut settings_actions);
        if let Some(event) = self.tag_editor.as_mut().and_then(|ed| ed.show(&ctx, self.settings.online_lookup)) {
            match event {
                TagEditorEvent::Close => self.tag_editor = None,
                TagEditorEvent::Saved(path) => {
                    // Tags neu einlesen, damit Browser, Playlists und Player die neuen Angaben zeigen
                    self.metas.remove(&path);
                    self.meta_requested.remove(&path);
                    self.meta_urgent.remove(&path);
                    if let Some(b) = &mut self.tag_batch {
                        b.saved(&path);
                    }
                    self.request_meta_now(path);
                    self.notify(tr("Tags gespeichert"), now);
                }
                TagEditorEvent::Reveal(path) => self.browser_action(BrowserAction::Reveal(path), now),
                TagEditorEvent::AllowOnline => {
                    self.settings.online_lookup = true;
                    self.save_settings();
                }
            }
        }
        let busy: Vec<PathBuf> = (0..2).filter(|&d| self.ctl.shared.decks[d].is_playing()).filter_map(|d| self.decks[d].path.clone()).collect();
        if let Some(event) = self.tag_batch.as_mut().and_then(|b| b.show(&ctx, self.settings.online_lookup, &busy)) {
            use crate::tag_batch::BatchEvent;
            match event {
                BatchEvent::Close => self.tag_batch = None,
                BatchEvent::Notice(text) => self.notify(text, now),
                BatchEvent::AllowOnline => {
                    self.settings.online_lookup = true;
                    self.save_settings();
                }
                BatchEvent::Review(path) => {
                    let mut ed = TagEditor::open(path);
                    ed.start_search(&ctx);
                    self.tag_editor = Some(ed);
                }
                BatchEvent::Changed(paths) => {
                    for path in paths {
                        self.metas.remove(&path);
                        self.meta_requested.remove(&path);
                        self.meta_urgent.remove(&path);
                        self.request_meta_now(path);
                    }
                }
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

        self.send_sketch_requests();
        self.dev.on_frame(&ctx);
        // Rad-Ereignisse, die kein Regler genommen hat: dort läuft gerade ein Scrollen
        crate::hw::note_foreign_scroll(&ctx);
        // Pfeil- und Eingabetasten, die kein Regler genommen hat: für die zuletzt angeklickte Titelliste
        crate::keynav::collect(&ctx);
        if theme::lite().shapes {
            theme::simplify_shapes(&ctx);
        } else {
            crate::plate::batch_by_texture(&ctx);
        }
        // Teller, Wellenform und Pegel laufen, solange etwas spielt (sparsam mit höchstens 30 Bildern/s);
        // im Stillstand nur bei Eingaben und gelegentlich zum Abholen des Engine-Zustands
        if !self.is_moving() {
            ctx.request_repaint_after(if theme::lite().frame_limit { IDLE_FRAME_LITE } else { IDLE_FRAME });
        } else if theme::lite().frame_limit {
            // Auf langsamen Rechnern (Netbook mit Intel GMA) dauert ein Bild weit länger als 33 ms. Die Pause
            // zählt ab hier, das Zeichnen des Bildes fällt schon hinein; mit der doppelten Bildzeit nimmt die
            // Oberfläche höchstens etwa die Hälfte der Rechenzeit und lässt dem Ton Luft
            let work = frame.info().cpu_usage.unwrap_or(0.0).clamp(0.0, 0.5);
            let pause = LITE_FRAME.max(std::time::Duration::from_secs_f32(2.0 * work));
            // egui zieht die Dauer eines Monitorbilds ab, damit es nicht zu spät kommt; hier soll es nicht früher kommen
            let monitor_frame = std::time::Duration::from_secs_f32(ctx.input(|i| i.predicted_dt).clamp(0.0, 0.05));
            ctx.request_repaint_after(pause + monitor_frame);
        } else {
            // Volle Optik: höchstens 60 Bilder/s (auf 120-Hz-Bildschirmen kaum zu sehen, halbiert die Arbeit).
            // Dauert ein Bild länger als die Pause (alte Rechner), wartet die nächste so lange, wie das Bild
            // brauchte: Die Oberfläche nimmt dann höchstens etwa die Hälfte der Rechenzeit
            let work = frame.info().cpu_usage.unwrap_or(0.0).clamp(0.0, 0.5);
            let pause = FULL_FRAME.max(std::time::Duration::from_secs_f32(work));
            let monitor_frame = std::time::Duration::from_secs_f32(ctx.input(|i| i.predicted_dt).clamp(0.0, 0.05));
            ctx.request_repaint_after(pause + monitor_frame);
        }
    }
}

fn shuffle_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |t| t.as_nanos() as u64)
}

fn shuffle(paths: &mut Vec<PathBuf>) {
    let mut pl = Playlist::new(std::mem::take(paths), None);
    pl.shuffle_upcoming(shuffle_seed());
    *paths = pl.entries;
}

/// Zeigt eine Datei oder einen Ordner im Dateimanager des Systems.
fn reveal(path: &Path) -> std::io::Result<()> {
    use std::process::Command as Cmd;
    if cfg!(target_os = "macos") {
        Cmd::new("open").arg("-R").arg(path).spawn()?;
    } else if cfg!(windows) {
        explorer_select(path)?;
    } else {
        // Ohne einheitliches „Datei markieren“ unter Linux: den Ordner öffnen
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        Cmd::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}

/// `explorer /select,"Pfad"`, unverändert übergeben: Mit Leerzeichen im Pfad („FadeWerk DJ“) setzt Rust
/// sonst das ganze Argument in Anführungszeichen, und der Explorer tut dann gar nichts.
#[cfg(windows)]
fn explorer_select(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let mut arg = std::ffi::OsString::from("/select,\"");
    arg.push(path);
    arg.push("\"");
    std::process::Command::new("explorer").raw_arg(arg).spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn explorer_select(_path: &Path) -> std::io::Result<()> {
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
        // Laufende Aufnahme beim Beenden abschließen, damit die Datei vollständig ist
        if let Some(rec) = self.recording.take() {
            self.ctl.send(Command::SetRecording(false));
            if let (_, Err(e)) = rec.finish() {
                log::warn!("Aufnahme beim Beenden: {e:#}");
            }
        }
        self.save_settings();
        self.save_session_if_changed();
        if self.persist {
            self.caches.bpm.save_if_dirty();
            self.caches.key.save_if_dirty();
            self.caches.tag.save_if_dirty(true);
            self.caches.folder.save_if_dirty();
            self.history.save_dirty();
        }
    }
}

fn start_dir_for_demo(app: &FadeWerkDjApp) -> Option<PathBuf> {
    std::env::args_os().nth(1).map(PathBuf::from).filter(|p| p.is_dir()).or_else(|| app.browser.tracks().first().and_then(|p| p.parent().map(Path::to_path_buf)))
}

/// Gedrückte Tasten 1 bis 8 im Fenster von `ctx` (Effekt-Pads), nicht beim Tippen, nicht mit ⌘/Strg/⌥,
/// ohne Tastenwiederholung.
fn pad_keys(ctx: &egui::Context) -> Vec<usize> {
    if ctx.egui_wants_keyboard_input() {
        return Vec::new();
    }
    const KEYS: [egui::Key; fw_engine::FX_SLOTS] =
        [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8];
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !(modifiers.command || modifiers.ctrl || modifiers.alt) => {
                    KEYS.iter().position(|k| k == key)
                }
                _ => None,
            })
            .collect()
    })
}

/// So lange bietet „Sortieren“ danach „Rückgängig“ an.
const SORT_UNDO_SECS: f64 = 60.0;

/// Breite des Ordnerfensters beim ersten Ausklinken.
const DETACHED_DEFAULT_W: f32 = 900.0;

/// Zieht der Nutzer gerade eine Kante (Maustaste gedrückt)? Sonst hat sich der Inhalt geändert.
fn dragging_edge(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.primary_down())
}

/// Ablagefläche über einem Player: leuchtet beim Ziehen eines Titels auf und liefert ihn beim Loslassen.
fn deck_drop_zone(ui: &egui::Ui, rect: egui::Rect, d: usize) -> Option<DragItem> {
    let ctx = ui.ctx();
    // Titel aus dem ausgeklinkten Ordnerfenster
    let foreign = crate::crossdrag::hover(ctx).is_some_and(|p| rect.contains(p));
    if !egui::DragAndDrop::has_payload_of_type::<DragItem>(ctx) && !foreign {
        return None;
    }
    let over = foreign || ctx.pointer_latest_pos().is_some_and(|p| rect.contains(p));
    if !over {
        return None;
    }
    if foreign {
        // Das Leuchten zeichnet sich unten mit; abgelegt wird über `take_drop`
        let item = crate::crossdrag::take_drop(ctx, rect);
        paint_deck_drop(ui, rect, d);
        return item;
    }
    paint_deck_drop(ui, rect, d);
    if ctx.input(|i| i.pointer.any_released()) {
        return egui::DragAndDrop::take_payload::<DragItem>(ctx).map(|a| (*a).clone());
    }
    None
}

fn paint_deck_drop(ui: &egui::Ui, rect: egui::Rect, d: usize) {
    let color = theme::deck_color(d);
    let p = ui.painter();
    p.rect_filled(rect, 8.0, color.gamma_multiply(0.14));
    p.rect_stroke(rect, 8.0, Stroke::new(3.0, color), egui::StrokeKind::Inside);
    let label = trf("In Player {} laden", &[&theme::deck_letter(d)]);
    let pill = egui::Rect::from_center_size(rect.center(), vec2(240.0, 44.0));
    crate::hw::paint_button(p, pill, &label, crate::fonts::bold(18.0), crate::hw::Look::Color(color), true, false, false, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Engine meldet alle 10,7 ms (512 Frames bei 48 kHz), die Oberfläche zeichnet alle 8,3 ms (120 Hz):
    /// die angezeigte Position soll trotzdem in gleichmäßigen Schritten laufen.
    #[test]
    fn shown_position_moves_evenly_between_engine_updates() {
        let mut v = DeckView::default();
        let (block, frame) = (512.0 / 48000.0, 1.0 / 120.0);
        let mut steps = Vec::new();
        let mut last = 0.0;
        for i in 1..240 {
            let now = i as f64 * frame;
            let raw = (now / block).floor() * block;
            let shown = v.smooth_position(raw, 1.0, now, frame);
            if i > 60 {
                steps.push(shown - last);
            }
            last = shown;
        }
        let (min, max) = steps.iter().fold((f64::MAX, f64::MIN), |(a, b), &s| (a.min(s), b.max(s)));
        // Roh wären es 0, 1 oder 2 Blöcke je Bild (0 bis 21 ms); geglättet bleibt es nahe 8,3 ms
        assert!(min > frame * 0.7 && max < frame * 1.3, "Schritte {min:.4}…{max:.4}");
    }
}

/// Dateiname ohne Ordner, für Hinweise.
fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Dauer als „1:02:03“ bzw. „4:05“.
fn format_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}
