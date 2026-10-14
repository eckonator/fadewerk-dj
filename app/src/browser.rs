// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordnerbrowser: Ordner anklicken, Titel sind sofort da. Kein Einlesen in eine Datenbank.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Button, CursorIcon, Key, RichText, Sense, vec2};
use egui_extras::{Column, TableBuilder};
use fw_library::{SCAN_LIMIT, TrackMeta, is_system_dir, list_dir};

use crate::deck_ui::fmt_time;
use crate::history::{Entry, ExportFormat, History};
use crate::playlist::DragItem;
use crate::search::{AdvancedSearch, Field};
use crate::selection::Selection;
use crate::i18n::{count_tracks, tr, trf};
use crate::theme;
use crate::fonts;

pub enum BrowserAction {
    /// Hinten an die Playlist anhängen.
    Add { list: usize, paths: Vec<PathBuf> },
    /// Direkt hinter den geladenen Titel einreihen.
    AddNext { list: usize, paths: Vec<PathBuf> },
    /// In zufälliger Reihenfolge anhängen.
    AddShuffled { list: usize, paths: Vec<PathBuf> },
    /// Abwechselnd auf beide Playlists verteilen.
    Distribute(Vec<PathBuf>),
    LoadDeck { deck: usize, path: PathBuf },
    /// Im Finder (Explorer, Dateimanager) zeigen.
    Reveal(PathBuf),
    /// Fenster „Titelinfo & Tags“ öffnen.
    EditTags(PathBuf),
    /// Der Nutzer hat die Breite des Ordnerbaums geändert.
    TreeWidth(f32),
    /// Erweiterte Suche zum Speichern: „Suchfilter merken“ an/aus und die Suche (leer, wenn nicht gemerkt).
    SearchMemory(bool, AdvancedSearch),
    /// Sichtbare Spalten der Titelliste oder ihre Breiten haben sich geändert (Schlüssel, Breiten je Schlüssel).
    TrackColumns(Vec<String>, Vec<(String, f32)>),
    /// Laufende Suche abbrechen.
    StopScan,
    /// BPM im Hintergrund ermitteln. Ohne `force` nur für Titel ohne bekannte BPM (Tag oder gemerkt).
    AnalyzeBpm { paths: Vec<PathBuf>, force: bool },
    /// Wartende BPM-Analysen verwerfen.
    StopBpm,
    /// Gemerkte Einstellungen „Mit Unterordnern“ je Ordner (nur Abweichungen vom Standard).
    FolderRecursion(Vec<(PathBuf, bool)>),
    /// Historie: Tag umbenennen, löschen, exportieren; Einträge entfernen; Datei importieren.
    HistoryRename { date: String, name: String },
    HistoryDelete(String),
    HistoryExport { date: String, format: ExportFormat },
    HistoryRemove { date: String, slots: Vec<usize> },
    HistoryImport,
    Preview(PathBuf),
    StopPreview,
    PreviewSeek(f64),
    NeedMeta(PathBuf),
    AddRoot,
    RemoveRoot(PathBuf),
    OpenSettings,
    ToggleFullscreen,
}

/// Spalten der Titelliste mit veränderbarer Breite. Der Titel ist immer da, die übrigen sind wählbar;
/// Vorhören, Nummer und die A/B-Tasten haben eine feste Breite. Reihenfolge hier = Reihenfolge in der Tabelle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrackColumn {
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Comment,
    Subfolder,
    FileName,
    Format,
    Bpm,
    Time,
}

impl TrackColumn {
    pub const ALL: [Self; 10] = [
        Self::Artist,
        Self::Album,
        Self::Genre,
        Self::Year,
        Self::Comment,
        Self::Subfolder,
        Self::FileName,
        Self::Format,
        Self::Bpm,
        Self::Time,
    ];
    pub const DEFAULT: [Self; 4] = [Self::Artist, Self::Subfolder, Self::Bpm, Self::Time];

    /// Schlüssel in den Einstellungen (bleibt stabil, auch wenn sich die Anzeige ändert).
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Comment => "comment",
            Self::Subfolder => "subfolder",
            Self::FileName => "file",
            Self::Format => "format",
            Self::Bpm => "bpm",
            Self::Time => "time",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        std::iter::once(Self::Title).chain(Self::ALL).find(|c| c.key() == key)
    }

    /// Anzeigename (deutscher Schlüssel für `tr`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "Titel",
            Self::Artist => "Interpret",
            Self::Album => "Album",
            Self::Genre => "Genre",
            Self::Year => "Jahr",
            Self::Comment => "Kommentar",
            Self::Subfolder => "Unterordner",
            Self::FileName => "Dateiname",
            Self::Format => "Format",
            Self::Bpm => "BPM",
            Self::Time => "Zeit",
        }
    }

    fn default_width(self) -> f32 {
        match self {
            Self::Title => 300.0,
            Self::Artist | Self::Album | Self::Comment => 180.0,
            Self::Genre => 110.0,
            Self::Subfolder | Self::FileName => 200.0,
            Self::Year | Self::Format | Self::Bpm => 52.0,
            Self::Time => 48.0,
        }
    }

    fn min_width(self) -> f32 {
        match self {
            Self::Title => 120.0,
            Self::Year | Self::Format | Self::Bpm | Self::Time => 36.0,
            _ => 60.0,
        }
    }
}

pub struct BrowserCtx<'a> {
    pub metas: &'a HashMap<PathBuf, TrackMeta>,
    pub bpms: &'a HashMap<PathBuf, f32>,
    /// Laufende BPM-Analyse: (fertig, gesamt).
    pub bpm_progress: Option<(usize, usize)>,
    pub preview: Option<&'a Path>,
    pub preview_pos: f64,
    pub preview_dur: f64,
    /// Spielt Player A bzw. B gerade? Dann lässt er sich nicht neu laden.
    pub deck_playing: [bool; 2],
    /// Läuft das Fenster im Vollbild?
    pub fullscreen: bool,
    pub history: &'a History,
    /// Ein Import der Historie läuft gerade.
    pub importing: bool,
}

/// Zeile der Titelliste. In der Historie kann derselbe Titel mehrmals vorkommen, deshalb zählt neben dem
/// Pfad die Stelle im Tag (`slot`); im Ordner ist sie immer 0.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Row {
    path: PathBuf,
    slot: usize,
}

/// Was in der Historie gewählt ist: der Ordner selbst (Hinweis) oder ein Tag.
#[derive(Clone, PartialEq, Debug)]
enum HistView {
    Root,
    Day(String),
}

/// Angaben zu einem Titel: gelesene Tags, sonst die in der Historie gemerkten, sonst aus dem Dateinamen.
fn meta_for<'a>(metas: &'a HashMap<PathBuf, TrackMeta>, path: &Path, entry: Option<&Entry>) -> Cow<'a, TrackMeta> {
    if let Some(m) = metas.get(path) {
        return Cow::Borrowed(m);
    }
    let mut m = TrackMeta::from_filename(path);
    if let Some(e) = entry {
        if !e.title.is_empty() {
            m.title = e.title.clone();
            m.artist = e.artist.clone();
        }
        m.album = e.album.clone();
        m.duration_secs = e.duration_secs;
    }
    Cow::Owned(m)
}

/// Was ein Ordner-Kontextmenü mit allen Titeln der ausgewählten Ordner macht. Läuft die Suche
/// noch, wird die Aktion vorgemerkt und ausgeführt, sobald alle Titel gefunden sind.
#[derive(Clone, Copy)]
enum FolderOp {
    Add(usize),
    Shuffled(usize),
    Distribute,
    /// BPM aller Titel ohne bekannte BPM ermitteln.
    AnalyzeBpm,
}

/// Breite des Ordnerbaums ohne gespeicherten Wert und Grenzen beim Ziehen.
pub const DEFAULT_TREE_W: f32 = 250.0;
const TREE_MIN_W: f32 = 140.0;
/// Breite des Griffs zwischen Baum und Titelliste.
const SPLIT_W: f32 = 9.0;

/// Ein Einstiegspunkt im Ordnerbaum.
#[derive(Clone)]
struct Root {
    /// Fester Name (übersetzt angezeigt) oder `None` für den Ordnernamen.
    label: Option<&'static str>,
    path: PathBuf,
    /// Vom Nutzer hinzugefügt: wird gespeichert und lässt sich wieder entfernen.
    user: bool,
}

impl Root {
    fn name(&self) -> String {
        match self.label {
            Some(l) => tr(l).to_owned(),
            None => self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string()),
        }
    }
}

pub struct Browser {
    roots: Vec<Root>,
    /// Ausgewählte Ordner (⌘/Strg-Klick wählt mehrere); ihre Titel stehen zusammen in der Liste.
    selected: Vec<PathBuf>,
    /// Alle Titel in den Ordnern und ihren Unterordnern, kommen nach und nach aus der Hintergrundsuche.
    tracks: Vec<PathBuf>,
    /// Ausgewählte Titel in der Liste.
    picked: Selection<Row>,
    /// Statt der Ordner ist die Historie gewählt.
    hist: Option<HistView>,
    /// Gibt es die Datei eines Historien-Eintrags noch? Einmal geprüft, bis zum Aktualisieren.
    exists: HashMap<PathBuf, bool>,
    /// Tag, der gerade umbenannt wird: (Datum, Eingabe, Feld noch fokussieren).
    renaming: Option<(String, String, bool)>,
    /// Den Ordner „Historie“ im Baum aufklappen (ein Tag wurde von außen geöffnet).
    expand_history: bool,
    /// Ordner-Aktion, die auf das Ende der Suche wartet.
    queued: Option<FolderOp>,
    tree_width: f32,
    /// Sichtbare wählbare Spalten der Titelliste, in der Reihenfolge von [`TrackColumn::ALL`].
    columns: Vec<TrackColumn>,
    /// Vom Nutzer gezogene Spaltenbreiten; fehlt eine, gilt [`TrackColumn::default_width`].
    widths: HashMap<TrackColumn, f32>,
    /// Wird erhöht, wenn die Breiten zurückgesetzt werden; egui merkt sich die Tabelle sonst weiter.
    table_gen: u32,
    /// Entwicklerschalter `FW_SELECT`: wählt zum Prüfen einige Titel aus.
    pub demo_select: bool,
    /// Entwicklerschalter `FW_BPM`: ermittelt einmal die fehlenden BPM des geöffneten Ordners.
    pub demo_bpm: bool,
    scanning: bool,
    limited: bool,
    scan_token: u64,
    pending_scan: Option<Vec<(PathBuf, bool)>>,
    error: Option<String>,
    filter: String,
    /// Für so viele Titel (von vorn) wurden die Tags für die Suche schon angefordert. Die Hintergrundsuche
    /// liefert Titel nach und nach, die später eintreffenden werden nachgefordert.
    meta_requested_upto: usize,
    subdirs: HashMap<PathBuf, Vec<PathBuf>>,
    /// „Mit Unterordnern“ je Ordner, nur wo der Nutzer vom Standard abweicht (siehe [`default_recursive`]).
    recursion: HashMap<PathBuf, bool>,
    /// Der Nutzer hat die Suche angehalten.
    stopped: bool,
    /// Erweiterte Suche (Lupe neben dem Suchfeld, ⇧⌘F); filtert die Titelliste zusätzlich zur Schnellsuche.
    adv: AdvancedSearch,
    adv_open: bool,
    /// Breiten von Angaben und Tasten über der Titelliste im letzten Bild (für den Umbruch).
    head_widths: (f32, f32),
    /// Beim Öffnen das erste Feld fokussieren.
    adv_focus: bool,
    /// Oberkante des Browsers: das Suchfenster geht mittig über den Playern darüber auf.
    players_bottom: f32,
    /// Suche beim Schließen und Beenden behalten.
    remember: bool,
    /// Zuletzt gemeldeter Stand (`remember`, Suche), um Änderungen zum Speichern zu erkennen.
    reported: (bool, AdvancedSearch),
}

impl Browser {
    /// `user_roots` sind gespeicherte, vom Nutzer hinzugefügte Ordner, `tree_width` die gespeicherte
    /// Breite des Ordnerbaums (0 = Standard).
    pub fn new(start_dir: Option<PathBuf>, user_roots: &[PathBuf], tree_width: f32, recursion: &[(PathBuf, bool)]) -> Self {
        let mut roots = Vec::new();
        let fixed = |label, path| Root { label: Some(label), path, user: false };
        if let Some(m) = dirs::audio_dir() {
            roots.push(fixed("Musik", m));
        }
        if let Some(h) = dirs::home_dir() {
            roots.push(fixed("Benutzerordner", h));
        }
        if cfg!(target_os = "macos") {
            roots.push(fixed("Laufwerke", PathBuf::from("/Volumes")));
        } else if cfg!(target_os = "linux") {
            roots.push(fixed("Laufwerke", PathBuf::from("/media")));
        }
        for p in user_roots.iter().filter(|p| p.is_dir()) {
            roots.push(Root { label: None, path: p.clone(), user: true });
        }
        let mut b = Self {
            roots,
            selected: Vec::new(),
            tracks: Vec::new(),
            picked: Selection::default(),
            hist: None,
            exists: HashMap::new(),
            renaming: None,
            expand_history: false,
            queued: None,
            tree_width: if tree_width > 0.0 { tree_width } else { DEFAULT_TREE_W },
            columns: TrackColumn::DEFAULT.to_vec(),
            widths: HashMap::new(),
            table_gen: 0,
            demo_select: false,
            demo_bpm: false,
            scanning: false,
            limited: false,
            scan_token: 0,
            pending_scan: None,
            error: None,
            filter: String::new(),
            meta_requested_upto: 0,
            subdirs: HashMap::new(),
            recursion: recursion.iter().cloned().collect(),
            stopped: false,
            adv: AdvancedSearch::default(),
            adv_open: false,
            head_widths: (0.0, 0.0),
            adv_focus: false,
            players_bottom: 0.0,
            remember: true,
            reported: (true, AdvancedSearch::default()),
        };
        match start_dir {
            Some(dir) => {
                b.add_root(dir);
            }
            None => {
                if let Some(first) = b.roots.iter().find(|r| r.user).or(b.roots.first()).map(|r| r.path.clone()) {
                    b.open(first);
                }
            }
        }
        b
    }

    /// Übernimmt die gespeicherten Spalten der Titelliste (unbekannte Schlüssel fallen heraus).
    pub fn set_columns(&mut self, visible: &[String], widths: &[(String, f32)]) {
        let visible: Vec<TrackColumn> = visible.iter().filter_map(|k| TrackColumn::from_key(k)).collect();
        self.columns = TrackColumn::ALL.into_iter().filter(|c| visible.contains(c)).collect();
        // Der Titel ist immer da und steht nicht in der Auswahl
        self.widths = widths
            .iter()
            .filter_map(|(k, w)| Some((TrackColumn::from_key(k)?, *w)))
            .filter(|(_, w)| w.is_finite() && *w > 0.0)
            .collect();
    }

    fn width(&self, c: TrackColumn) -> f32 {
        self.widths.get(&c).copied().unwrap_or(c.default_width()).max(c.min_width())
    }

    fn columns_changed(&self, out: &mut Vec<BrowserAction>) {
        let visible = self.columns.iter().map(|c| c.key().to_owned()).collect();
        let mut widths: Vec<(String, f32)> = self.widths.iter().map(|(c, w)| (c.key().to_owned(), w.round())).collect();
        widths.sort_by(|a, b| a.0.cmp(&b.0));
        out.push(BrowserAction::TrackColumns(visible, widths));
    }

    /// Auswahl der Spalten (Rechtsklick auf den Tabellenkopf oder Taste „Spalten“).
    fn columns_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<BrowserAction>) {
        ui.label(RichText::new(tr("Spalten anzeigen")).size(11.0).color(theme::faint()));
        let mut on = true;
        ui.add_enabled(false, egui::Checkbox::new(&mut on, tr("Titel")));
        for c in TrackColumn::ALL {
            let mut on = self.columns.contains(&c);
            if ui.checkbox(&mut on, tr(c.name())).changed() {
                self.columns = TrackColumn::ALL.into_iter().filter(|x| if *x == c { on } else { self.columns.contains(x) }).collect();
                self.columns_changed(out);
            }
        }
        ui.separator();
        if ui.add_enabled(!self.widths.is_empty(), Button::new(tr("Spaltenbreiten zurücksetzen"))).clicked() {
            self.widths.clear();
            self.table_gen += 1;
            self.columns_changed(out);
            ui.close();
        }
    }

    /// Leiste mit den aktiven Feldern der erweiterten Suche; ein Klick entfernt das Feld.
    fn filter_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("FILTER")).size(10.5).color(theme::faint()));
            let accent = theme::select_accent();
            let set: Vec<Field> = Field::ALL.into_iter().filter(|f| self.adv.is_set(*f)).collect();
            for f in set {
                let text = format!("{}: {}  ×", tr(f.name()), self.adv.summary(f));
                let b = Button::new(RichText::new(text).size(12.0).color(accent))
                    .fill(accent.gamma_multiply(0.10))
                    .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.55)))
                    .corner_radius(10.0);
                if ui.add(b).on_hover_text(tr("Filter entfernen")).clicked() {
                    self.adv.clear(f);
                }
            }
            if ui.small_button(tr("Alle entfernen")).clicked() {
                self.adv = AdvancedSearch::default();
            }
            if !self.adv_open && ui.small_button(tr("Bearbeiten …")).clicked() {
                self.toggle_search();
            }
        });
    }

    /// Gespeicherte erweiterte Suche übernehmen.
    pub fn set_search_memory(&mut self, remember: bool, search: &AdvancedSearch) {
        self.remember = remember;
        self.adv = if remember { search.clone() } else { AdvancedSearch::default() };
        self.reported = (self.remember, self.adv.clone());
    }

    /// Entwicklerschalter `FW_SEARCH`: erweiterte Suche vorbelegt und offen.
    pub fn demo_search(&mut self, spec: &str) {
        self.adv = AdvancedSearch::from_dev_spec(spec);
        self.adv_open = true;
        self.adv_focus = true;
    }

    fn toggle_search(&mut self) {
        if self.adv_open {
            self.close_search();
        } else {
            self.adv_open = true;
            self.adv_focus = true;
        }
    }

    /// Ohne „Suchfilter merken“ endet die Suche mit dem Schließen des Fensters.
    fn close_search(&mut self) {
        self.adv_open = false;
        if !self.remember {
            self.adv = AdvancedSearch::default();
        }
    }

    /// Fenster der erweiterten Suche. Sperrt nichts: die Liste filtert schon beim Tippen.
    fn search_window(&mut self, ctx: &egui::Context) {
        if !self.adv_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        let mut window = egui::Window::new(tr("Erweiterte Suche"))
            .id(egui::Id::new("erweiterte-suche"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER);
        // Beim Öffnen immer mittig über den Playern, damit die gefilterte Liste frei bleibt; danach verschiebbar
        if self.adv_focus {
            let screen = ctx.content_rect();
            let bottom = if self.players_bottom > screen.top() { self.players_bottom } else { screen.bottom() };
            window = window.current_pos(egui::pos2(screen.center().x, (screen.top() + bottom) / 2.0));
        }
        window
            .frame(egui::Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(egui::Margin::same(14)))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                let mut focus = std::mem::take(&mut self.adv_focus);
                egui::Grid::new("erweiterte-suche-felder").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                    for f in Field::ALL {
                        ui.label(RichText::new(tr(f.name())).color(theme::dim()));
                        match self.adv.range_mut(f) {
                            Some((from, to)) => {
                                ui.horizontal(|ui| {
                                    ui.add(egui::TextEdit::singleline(from).hint_text(tr("von")).desired_width(80.0));
                                    ui.label(RichText::new("–").color(theme::faint()));
                                    ui.add(egui::TextEdit::singleline(to).hint_text(tr("bis")).desired_width(80.0));
                                });
                            }
                            None => {
                                let r = ui.add(egui::TextEdit::singleline(self.adv.text_mut(f)).desired_width(240.0));
                                // Beim Öffnen gleich ins erste Feld tippen können
                                if std::mem::take(&mut focus) {
                                    r.request_focus();
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(tr("Sucht in den gewählten Ordnern. Bei Jahr und BPM fallen Titel ohne Angabe heraus; BPM kennt FadeWerk DJ aus den Tags oder nach dem Laden in einen Player."))
                        .size(11.5)
                        .color(theme::faint()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.adv.is_empty(), Button::new(tr("Alles löschen"))).clicked() {
                        self.adv = AdvancedSearch::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr("Fertig")).clicked() {
                            close = true;
                        }
                        ui.add_space(6.0);
                        ui.checkbox(&mut self.remember, tr("Suchfilter merken"))
                            .on_hover_text(tr("Die Suchfilter bleiben nach dem Schließen des Fensters und nach einem Neustart erhalten."));
                    });
                });
            });
        if close || !open || ctx.input(|i| i.key_pressed(Key::Escape)) && ctx.memory(|m| m.focused().is_none()) {
            self.close_search();
        }
    }

    /// Fügt einen Ordner hinzu und öffnet ihn. `true`, wenn er neu in der Liste ist.
    pub fn add_root(&mut self, path: PathBuf) -> bool {
        let new = !self.roots.iter().any(|r| r.path == path);
        if new {
            self.roots.push(Root { label: None, path: path.clone(), user: true });
        }
        self.open(path);
        new
    }

    pub fn remove_root(&mut self, path: &Path) {
        self.roots.retain(|r| !(r.user && r.path == path));
    }

    /// Öffnet einen Ordner (und nur diesen). Die Titel samt Unterordnern sucht die Bibliothek im
    /// Hintergrund (siehe [`Self::take_pending_scan`]); sie erscheinen, sobald sie gefunden sind.
    pub fn open(&mut self, path: PathBuf) {
        self.hist = None;
        self.selected = vec![path];
        self.filter.clear();
        self.rescan();
    }

    /// ⌘/Strg-Klick: Ordner zur Auswahl hinzufügen oder herausnehmen. Einer bleibt immer ausgewählt.
    fn toggle_folder(&mut self, path: &Path) {
        if self.hist.take().is_some() {
            self.open(path.to_path_buf());
            return;
        }
        if let Some(i) = self.selected.iter().position(|p| p == path) {
            if self.selected.len() == 1 {
                return;
            }
            self.selected.remove(i);
        } else {
            self.selected.push(path.to_path_buf());
        }
        self.rescan();
    }

    /// Liest Ordnerbaum und Titelliste neu ein (Taste „Aktualisieren“, ⌘R, F5).
    /// Die Auswahl in der Liste bleibt, soweit die Titel noch da sind.
    pub fn refresh(&mut self) {
        self.subdirs.clear();
        self.exists.clear();
        let keep = std::mem::take(&mut self.picked);
        self.rescan();
        self.picked = keep;
    }

    fn rescan(&mut self) {
        self.tracks.clear();
        self.limited = false;
        self.picked.clear();
        self.queued = None;
        self.meta_requested_upto = 0;
        let mut readable = Vec::new();
        self.error = None;
        for path in &self.selected {
            match list_dir(path) {
                Ok(l) => {
                    self.subdirs.insert(path.clone(), l.dirs);
                    readable.push(path.clone());
                }
                Err(e) => {
                    self.error.get_or_insert_with(|| trf("{} lässt sich nicht lesen: {}", &[&path.display(), &e]));
                }
            }
        }
        self.stopped = false;
        // Liegt ein ausgewählter Ordner in einem anderen, der mit Unterordnern durchsucht wird,
        // wird er nicht doppelt gesucht
        let roots: Vec<(PathBuf, bool)> = readable
            .iter()
            .filter(|p| !readable.iter().any(|q| q != *p && p.starts_with(q) && self.recursive(q)))
            .map(|p| (p.clone(), self.recursive(p)))
            .collect();
        self.scanning = !roots.is_empty();
        self.pending_scan = (!roots.is_empty()).then_some(roots);
    }

    /// Wird dieser Ordner mit Unterordnern durchsucht?
    fn recursive(&self, path: &Path) -> bool {
        self.recursion.get(path).copied().unwrap_or_else(|| default_recursive(path))
    }

    /// Schalter „Mit Unterordnern“ für alle ausgewählten Ordner.
    fn set_recursive(&mut self, on: bool, out: &mut Vec<BrowserAction>) {
        for p in self.selected.clone() {
            if on == default_recursive(&p) {
                self.recursion.remove(&p);
            } else {
                self.recursion.insert(p, on);
            }
        }
        let mut list: Vec<(PathBuf, bool)> = self.recursion.iter().map(|(p, r)| (p.clone(), *r)).collect();
        list.sort();
        out.push(BrowserAction::FolderRecursion(list));
        self.rescan();
    }

    /// Ordner, für die eine Suche gestartet werden soll, je mit „Unterordner ja/nein“.
    pub fn take_pending_scan(&mut self) -> Option<Vec<(PathBuf, bool)>> {
        self.pending_scan.take()
    }

    pub fn begin_scan(&mut self, token: u64) {
        self.scan_token = token;
    }

    /// Ergebnisse der Hintergrundsuche. Ergebnisse alter Suchen werden verworfen.
    pub fn receive_scan(&mut self, token: u64, tracks: Vec<PathBuf>, done: bool, limited: bool) {
        if token != self.scan_token {
            return;
        }
        self.tracks.extend(tracks);
        if done {
            self.scanning = false;
            self.limited = limited;
        }
    }

    /// Titel in den ausgewählten Ordnern samt Unterordnern (soweit schon gefunden).
    pub fn tracks(&self) -> &[PathBuf] {
        &self.tracks
    }

    fn children(&mut self, path: &Path) -> Vec<PathBuf> {
        if let Some(c) = self.subdirs.get(path) {
            return c.clone();
        }
        let c = list_dir(path).map(|l| l.dirs).unwrap_or_default();
        self.subdirs.insert(path.to_path_buf(), c.clone());
        c
    }

    pub fn show(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        // Nicht innerhalb von `ui.input` abfragen: egui hielte sonst dieselbe Sperre zweimal
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (refocused, refresh_key, search_key) = ui.input(|i| {
            (
                i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true))),
                !typing && ((i.modifiers.command && i.key_pressed(Key::R)) || i.key_pressed(Key::F5)),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(Key::F),
            )
        });
        if search_key {
            self.toggle_search();
        }
        self.players_bottom = ui.max_rect().top();
        // Vor der Liste, damit Eingaben noch in diesem Bild filtern
        self.search_window(&ui.ctx().clone());
        // Geänderte Suche oder „Suchfilter merken“ zum Speichern melden (ohne Merken wird eine leere Suche gespeichert)
        let now = (self.remember, if self.remember { self.adv.clone() } else { AdvancedSearch::default() });
        if now != self.reported {
            self.reported = now.clone();
            out.push(BrowserAction::SearchMemory(now.0, now.1));
        }
        if refresh_key {
            self.refresh();
        } else if refocused {
            // Zurück im Fenster, etwa nach dem Anlegen eines Ordners im Finder: Baum neu lesen.
            // Die Titelliste bleibt, damit Auswahl und Scrollstand nicht springen.
            self.subdirs.clear();
        }
        if self.demo_bpm && !self.scanning && !self.tracks.is_empty() {
            self.demo_bpm = false;
            self.queued = Some(FolderOp::AnalyzeBpm);
        }
        // Vorgemerkte Ordner-Aktion ausführen, sobald alle Titel gefunden sind
        if !self.scanning
            && let Some(op) = self.queued.take()
            && !self.tracks.is_empty()
        {
            let paths = self.tracks.clone();
            out.push(match op {
                FolderOp::Add(list) => BrowserAction::Add { list, paths },
                FolderOp::Shuffled(list) => BrowserAction::AddShuffled { list, paths },
                FolderOp::Distribute => BrowserAction::Distribute(paths),
                FolderOp::AnalyzeBpm => BrowserAction::AnalyzeBpm { paths, force: false },
            });
        }

        // Kopfzeile
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("ORDNER")).font(fonts::bold(16.0)).extra_letter_spacing(1.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("⚙").size(15.0)).on_hover_text(tr("Einstellungen (⌘,)")).clicked() {
                    out.push(BrowserAction::OpenSettings);
                }
                if fullscreen_button(ui, ctx.fullscreen).clicked() {
                    out.push(BrowserAction::ToggleFullscreen);
                }
                let tip = if cfg!(target_os = "macos") { tr("Ordner neu einlesen (⌘R)") } else { tr("Ordner neu einlesen (F5)") };
                if ui.button(RichText::new("🔄").size(14.0)).on_hover_text(tip).clicked() {
                    self.refresh();
                }
                if ui.button(tr("Ordner hinzufügen …")).clicked() {
                    out.push(BrowserAction::AddRoot);
                }
                // Rechts neben der Schnellsuche (das Layout läuft hier von rechts nach links)
                if search_button(ui, self.adv_open || !self.adv.is_empty()).clicked() {
                    self.toggle_search();
                }
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(tr("Suchen")).desired_width(200.0));
                ui.label(RichText::new(tr("⏺ Live gelesen, keine Datenbank")).size(12.0).color(theme::play()));
                if let Some((done, total)) = ctx.bpm_progress {
                    if ui.small_button(tr("Stopp")).on_hover_text(tr("Wartende BPM-Analysen verwerfen")).clicked() {
                        out.push(BrowserAction::StopBpm);
                    }
                    ui.label(RichText::new(trf("BPM-Analyse {} von {}", &[&done, &total])).size(12.0).color(theme::dim()));
                    ui.add(egui::Spinner::new().size(14.0));
                }
                if !self.selected.is_empty() && self.hist.is_none() {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // Pfade im Home-Ordner kurz als „~/…“; mehrere Ordner nur mit Namen
                        let shown = match self.selected.as_slice() {
                            [one] => dirs::home_dir()
                                .and_then(|h| one.strip_prefix(h).ok().map(|r| format!("~/{}", r.display())))
                                .unwrap_or_else(|| one.display().to_string()),
                            many => many.iter().map(|p| dir_name(p)).collect::<Vec<_>>().join(" · "),
                        };
                        ui.add(egui::Label::new(RichText::new(shown).monospace().size(12.0).color(theme::dim())).truncate());
                    });
                }
            });
        });
        ui.separator();

        // Feste Aufteilung: Liste oben, Vorhör-Leiste unten. Die Kinder dürfen das Panel nicht vergrößern,
        // sonst schiebt sich der Browser beim Vorhören über die Player.
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, Sense::hover());
        let preview_h = if ctx.preview.is_some() { PREVIEW_BAR_H } else { 0.0 };
        let list_rect = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), full.bottom() - preview_h));
        let avail = list_rect.height();
        let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(egui::Layout::top_down(egui::Align::Min)));
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        let ui = &mut list_ui;
        ui.set_clip_rect(list_rect.intersect(ui.clip_rect()));
        crate::hw::calm_plate(ui.painter(), list_rect);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Baum
            let max_w = (ui.available_width() * 0.6).max(TREE_MIN_W);
            let tree_w = self.tree_width.clamp(TREE_MIN_W, max_w);
            ui.allocate_ui_with_layout(vec2(tree_w, avail), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).min_scrolled_height(0.0).show(ui, |ui| {
                    for root in self.roots.clone() {
                        self.tree_node(ui, &root.name(), &root.path, 0, root.user, out);
                    }
                    // Virtueller Ordner unter den echten
                    self.history_tree(ui, ctx, out);
                });
            });
            self.splitter(ui, avail, tree_w, max_w, out);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), avail), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                self.track_list(ui, ctx, out);
            });
        });

        if let Some(p) = ctx.preview {
            let bar = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - PREVIEW_BAR_H + 4.0), full.max);
            preview_bar(&mut bar_ui, bar, p, ctx, out);
        }
    }

    /// Griff zwischen Ordnerbaum und Titelliste: ziehen ändert die Breite, Doppelklick setzt sie zurück.
    fn splitter(&mut self, ui: &mut egui::Ui, height: f32, tree_w: f32, max_w: f32, out: &mut Vec<BrowserAction>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(SPLIT_W, height), Sense::click_and_drag());
        let resp = resp.on_hover_cursor(CursorIcon::ResizeHorizontal);
        if resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            self.tree_width = (tree_w + resp.drag_delta().x).clamp(TREE_MIN_W, max_w);
        }
        if resp.drag_stopped() {
            out.push(BrowserAction::TreeWidth(self.tree_width.round()));
        }
        if resp.double_clicked() {
            self.tree_width = DEFAULT_TREE_W;
            out.push(BrowserAction::TreeWidth(DEFAULT_TREE_W));
        }
        let active = resp.hovered() || resp.dragged();
        let (w, color) = if active { (2.0, theme::ink().gamma_multiply(0.5)) } else { (1.0, theme::line()) };
        let x = rect.center().x;
        ui.painter().vline(x, rect.y_range(), egui::Stroke::new(w, color));
        if active {
            // Drei Rillen in der Mitte als Griff
            for dy in [-6.0, 0.0, 6.0] {
                let y = rect.center().y + dy;
                ui.painter().hline(x - 3.0..=x + 3.0, y, egui::Stroke::new(1.0, color));
            }
        }
    }

    /// Zeichnet einen Ordner im Baum samt aufgeklappter Unterordner.
    fn tree_node(&mut self, ui: &mut egui::Ui, name: &str, path: &Path, depth: usize, removable: bool, out: &mut Vec<BrowserAction>) {
        let id = ui.id().with(("tree", path));
        let selected = self.hist.is_none() && self.selected.iter().any(|p| p == path);
        // Ein Menü auf einem ausgewählten Ordner gilt für alle ausgewählten
        let count = if selected { self.selected.len() } else { 1 };
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0 && selected);
        let mut open_this = false;
        let mut toggle_this = false;
        let mut menu = None;
        state
            .show_header(ui, |ui| {
                let icon = if depth == 0 { "🗀" } else { "📁" };
                let resp = tree_button(ui, &format!("{icon} {name}"), selected);
                if resp.clicked() {
                    if ui.input(|i| i.modifiers.command) { toggle_this = true } else { open_this = true }
                }
                // Rechtsklick auf einen nicht ausgewählten Ordner wählt ihn aus, wie im Finder
                if resp.secondary_clicked() && !selected {
                    open_this = true;
                }
                resp.context_menu(|ui| menu = folder_menu(ui, count, removable));
            })
            .body(|ui| {
                for child in self.children(path) {
                    let n = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.tree_node(ui, &n, &child, depth + 1, false, out);
                }
            });
        if open_this {
            self.open(path.to_path_buf());
        } else if toggle_this {
            self.toggle_folder(path);
        }
        match menu {
            Some(FolderMenu::Op(op)) => {
                if !self.selected.iter().any(|p| p == path) {
                    self.open(path.to_path_buf());
                }
                self.queued = Some(op);
            }
            Some(FolderMenu::Reveal) => out.push(BrowserAction::Reveal(path.to_path_buf())),
            Some(FolderMenu::Remove) => out.push(BrowserAction::RemoveRoot(path.to_path_buf())),
            None => {}
        }
    }

    /// Virtueller Ordner „Historie“ unter den Ordnern im Baum, darin die Tage (neueste zuerst).
    fn history_tree(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        let id = ui.id().with("history-root");
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        if std::mem::take(&mut self.expand_history) {
            state.set_open(true);
        }
        let mut open_root = false;
        let mut import = false;
        let mut open_day: Option<String> = None;
        let mut day_menu: Option<(String, DayMenu)> = None;
        let root_selected = self.hist == Some(HistView::Root);
        state
            .show_header(ui, |ui| {
                let resp = tree_button(ui, &format!("🕘 {}", tr("Historie")), root_selected);
                if resp.clicked() || resp.secondary_clicked() {
                    open_root = true;
                }
                resp.on_hover_text(tr("Gespielte Titel, Tag für Tag")).context_menu(|ui| {
                    if ui.add_enabled(!ctx.importing, Button::new(tr("Historie importieren …"))).clicked() {
                        import = true;
                        ui.close();
                    }
                });
            })
            .body(|ui| {
                if ctx.history.is_empty() {
                    ui.label(RichText::new(tr("noch leer")).font(fonts::list(12.0)).color(theme::faint()));
                }
                for day in ctx.history.days() {
                    let selected = self.hist == Some(HistView::Day(day.date.clone()));
                    // Umbenennen direkt im Baum: Enter oder Klick daneben übernimmt, Esc bricht ab
                    if let Some((date, text, focus)) = self.renaming.as_mut().filter(|r| r.0 == day.date) {
                        let r = ui.add(egui::TextEdit::singleline(text).hint_text(tr("Name, z. B. Omas Geburtstag")).desired_width(ui.available_width() - 8.0));
                        if std::mem::take(focus) {
                            r.request_focus();
                        }
                        let esc = ui.input(|i| i.key_pressed(Key::Escape));
                        if esc {
                            self.renaming = None;
                        } else if r.lost_focus() {
                            out.push(BrowserAction::HistoryRename { date: date.clone(), name: text.trim().to_owned() });
                            self.renaming = None;
                        }
                        continue;
                    }
                    let resp = tree_button(ui, &format!("📅 {}", day.label()), selected);
                    if resp.clicked() || (resp.secondary_clicked() && !selected) {
                        open_day = Some(day.date.clone());
                    }
                    if resp.double_clicked() {
                        day_menu = Some((day.date.clone(), DayMenu::Rename));
                    }
                    let n = day.entries.len();
                    resp.on_hover_text(trf("{} Titel", &[&n])).context_menu(|ui| {
                        if let Some(m) = history_menu(ui) {
                            day_menu = Some((day.date.clone(), m));
                        }
                    });
                }
            });
        if open_root {
            self.hist = Some(HistView::Root);
            self.picked.clear();
            let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
            st.set_open(true);
            st.store(ui.ctx());
        }
        if import {
            out.push(BrowserAction::HistoryImport);
        }
        if let Some(date) = open_day {
            self.open_history_day(date);
        }
        if let Some((date, m)) = day_menu {
            let Some(day) = ctx.history.day(&date) else { return };
            // Nur vorhandene Dateien gehen in die Playlists
            let mut paths = || day.entries.iter().map(|e| e.path.clone()).filter(|p| *self.exists.entry(p.clone()).or_insert_with(|| p.is_file())).collect::<Vec<_>>();
            match m {
                DayMenu::Add(list) => out.push(BrowserAction::Add { list, paths: paths() }),
                DayMenu::Distribute => out.push(BrowserAction::Distribute(paths())),
                DayMenu::Rename => self.renaming = Some((date.clone(), day.name.clone(), true)),
                DayMenu::Export(format) => out.push(BrowserAction::HistoryExport { date: date.clone(), format }),
                DayMenu::Delete => out.push(BrowserAction::HistoryDelete(date.clone())),
            }
        }
    }

    /// Zeigt einen Tag der Historie in der Titelliste.
    pub fn open_history_day(&mut self, date: String) {
        if self.hist.as_ref() != Some(&HistView::Day(date.clone())) {
            self.picked.clear();
            self.exists.clear();
        }
        self.hist = Some(HistView::Day(date));
        self.expand_history = true;
    }

    /// Hinweis, solange in der Historie kein Tag gewählt ist.
    fn history_intro(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Historie")).font(fonts::bold(17.0)));
            if ctx.importing {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(tr("Importiert …")).size(12.0).color(theme::dim()));
            }
        });
        ui.add_space(10.0);
        let text = if ctx.history.is_empty() {
            tr("Noch nichts gespielt. Hier landen die gespielten Titel, Tag für Tag der Reihe nach: ein Titel zählt, sobald er 30 Sekunden über Master zu hören war.")
        } else {
            tr("Links einen Tag wählen. Ein Titel zählt, sobald er 30 Sekunden über Master zu hören war; Titel bis 6 Uhr früh gehören zum Vortag.")
        };
        ui.label(RichText::new(text).color(theme::dim()));
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Rechtsklick auf einen Tag: umbenennen (z. B. „Omas Geburtstag“), exportieren oder löschen.")).size(12.0).color(theme::faint()));
        ui.add_space(10.0);
        if ui.add_enabled(!ctx.importing, Button::new(tr("Historie importieren …"))).on_hover_text(tr("FadeWerk-DJ-Historie (.json) oder Playlist (.m3u, .m3u8)")).clicked() {
            out.push(BrowserAction::HistoryImport);
        }
    }

    fn track_list(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        // Historie: ein Tag als Titelliste, ohne gewählten Tag ein Hinweis
        let hist_day = match &self.hist {
            Some(HistView::Day(d)) => ctx.history.day(d),
            _ => None,
        };
        if self.hist.is_some() && hist_day.is_none() {
            self.history_intro(ui, ctx, out);
            return;
        }
        if hist_day.is_none() && self.selected.is_empty() {
            return;
        }
        let dirs = self.selected.clone();
        let f = self.filter.trim().to_lowercase();
        let searching = !f.is_empty() || !self.adv.is_empty();
        // Einträge der Historie, deren Datei fehlt (ausgegraut, nicht abspielbar)
        let mut missing: HashSet<usize> = HashSet::new();
        if let Some(day) = hist_day {
            for (i, e) in day.entries.iter().enumerate() {
                let exists = *self.exists.entry(e.path.clone()).or_insert_with(|| e.path.is_file());
                if !exists {
                    missing.insert(i);
                } else if searching && !ctx.metas.contains_key(&e.path) {
                    out.push(BrowserAction::NeedMeta(e.path.clone()));
                }
            }
        } else if searching && self.meta_requested_upto < self.tracks.len() {
            // Für die Suche alle Tags im Hintergrund anfordern, auch für Titel, die die Ordnersuche noch nachliefert
            let new = &self.tracks[self.meta_requested_upto..];
            out.extend(new.iter().filter(|p| !ctx.metas.contains_key(*p)).map(|p| BrowserAction::NeedMeta(p.clone())));
            self.meta_requested_upto = self.tracks.len();
        } else if !searching {
            self.meta_requested_upto = 0;
        }
        let entry_of = |r: &Row| hist_day.and_then(|d| d.entries.get(r.slot));
        // Alle Suchwörter müssen vorkommen, in beliebiger Reihenfolge
        let words: Vec<&str> = f.split_whitespace().collect();
        let all: Box<dyn Iterator<Item = Row>> = match hist_day {
            Some(day) => Box::new(day.entries.iter().enumerate().map(|(slot, e)| Row { path: e.path.clone(), slot })),
            None => Box::new(self.tracks.iter().map(|p| Row { path: p.clone(), slot: 0 })),
        };
        let rows: Vec<Row> = all
            .filter(|r| {
                if self.adv.is_empty() && words.is_empty() {
                    return true;
                }
                let m = meta_for(ctx.metas, &r.path, entry_of(r));
                let p = &r.path;
                if !self.adv.is_empty() {
                    let folder = p.parent().map(|d| d.to_string_lossy()).unwrap_or_default();
                    let bpm = ctx.bpms.get(p).copied().or(m.bpm);
                    if !self.adv.matches(&m, &folder, bpm) {
                        return false;
                    }
                }
                if words.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {} {} {} {} {} {}",
                    subfolder(&dirs, p),
                    p.file_name().unwrap_or_default().to_string_lossy(),
                    m.title,
                    m.artist,
                    m.album,
                    m.genre,
                    m.year
                )
                .to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .collect();
        let total_count = hist_day.map_or(self.tracks.len(), |d| d.entries.len());
        let has_subfolders = hist_day.is_none() && (dirs.len() > 1 || self.tracks.iter().any(|p| p.parent() != Some(dirs[0].as_path())));
        // Pfade der Zeilen ohne fehlende Dateien: nur die lassen sich laden, vorhören, analysieren
        let playable = |rows: &[Row]| rows.iter().filter(|r| hist_day.is_none() || !missing.contains(&r.slot)).map(|r| r.path.clone()).collect::<Vec<_>>();

        if self.demo_select && rows.len() >= 4 && !self.scanning {
            self.demo_select = false;
            self.picked.click(&rows, &rows[1], false, false);
            self.picked.click(&rows, &rows[3], false, true);
        }

        // ⌘A wählt alle sichtbaren Titel, Esc hebt die Auswahl auf (nur mit der Maus über der Liste)
        if ui.rect_contains_pointer(ui.max_rect()) && !ui.ctx().egui_wants_keyboard_input() {
            let (all, esc) = ui.input(|i| (i.modifiers.command && i.key_pressed(Key::A), i.key_pressed(Key::Escape)));
            if all {
                self.picked.select_all(&rows);
            }
            if esc {
                self.picked.clear();
            }
        }
        let sel_rows = self.picked.ordered(&rows);
        let row_paths = playable(&rows);
        let sel_paths = playable(&sel_rows);

        // Reicht die Breite nicht für Angaben und Tasten nebeneinander, kommen die Tasten in eine eigene Zeile
        // (gemessen im letzten Bild, sonst überdecken sie „Mit Unterordnern“)
        let wrap_tools = self.head_widths.0 + self.head_widths.1 + 24.0 > ui.available_width();
        let mut tools_w = self.head_widths.1;
        ui.horizontal(|ui| {
            let left = ui.horizontal(|ui| {
                let folder = match (hist_day, dirs.as_slice()) {
                    (Some(day), _) => day.label(),
                    (None, [one]) => dir_name(one),
                    (None, many) => trf("{} Ordner", &[&many.len()]),
                };
                ui.label(RichText::new(folder).font(fonts::bold(17.0)));
                let total: f64 = rows.iter().map(|r| ctx.metas.get(&r.path).map_or_else(|| entry_of(r).map_or(0.0, |e| e.duration_secs), |m| m.duration_secs)).sum();
                let count = if words.is_empty() && self.adv.is_empty() {
                    count_tracks(rows.len(), &fmt_time(total))
                } else {
                    trf("{} von {} Titeln", &[&rows.len(), &total_count])
                };
                ui.label(RichText::new(count).color(theme::dim()));
                if !sel_rows.is_empty() {
                    ui.label(RichText::new(trf("· {} ausgewählt", &[&sel_rows.len()])).font(fonts::list(12.5)).color(theme::ink()));
                }
                if hist_day.is_some() {
                    if !missing.is_empty() {
                        ui.label(RichText::new(trf("{} nicht gefunden", &[&missing.len()])).size(12.0).color(theme::yellow()))
                            .on_hover_text(tr("Diese Dateien gibt es nicht mehr an ihrem Platz. Sie bleiben in der Historie, lassen sich aber nicht abspielen."));
                    }
                    return;
                }
                let all_recursive = dirs.iter().all(|d| self.recursive(d));
                let mut rec = all_recursive;
                let tip = tr("Auch alle Unterordner durchsuchen. Bei Laufwerken und dem Benutzerordner ist das anfangs aus, damit nicht die ganze Platte durchsucht wird. Systemordner werden immer ausgelassen.");
                if ui.checkbox(&mut rec, tr("Mit Unterordnern")).on_hover_text(tip).changed() {
                    self.set_recursive(rec, out);
                }
                if self.scanning {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new(tr("Durchsucht Unterordner …")).size(12.0).color(theme::dim()));
                    if ui.small_button(tr("Stopp")).on_hover_text(tr("Suche anhalten, gefundene Titel bleiben")).clicked() {
                        self.scanning = false;
                        self.stopped = true;
                        out.push(BrowserAction::StopScan);
                    }
                } else if self.stopped {
                    ui.label(RichText::new(tr("Suche angehalten")).size(12.0).color(theme::yellow()));
                } else if self.limited {
                    ui.label(RichText::new(trf("Nur die ersten {} Titel. Für mehr einen Unterordner wählen.", &[&SCAN_LIMIT])).size(12.0).color(theme::yellow()));
                }
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).size(12.0).color(theme::cue()));
                }
            });
            self.head_widths.0 = left.response.rect.width();
            if !wrap_tools {
                tools_w = list_tools(ui, ctx, &row_paths, &sel_paths, hist_day.is_some(), out);
            }
        });
        if wrap_tools {
            ui.horizontal(|ui| tools_w = list_tools(ui, ctx, &row_paths, &sel_paths, hist_day.is_some(), out));
        }
        self.head_widths.1 = tools_w;

        if !self.adv.is_empty() {
            self.filter_bar(ui);
        }

        if total_count == 0 {
            ui.add_space(20.0);
            let text = if hist_day.is_some() {
                tr("An diesem Tag ist nichts mehr eingetragen.")
            } else if self.scanning {
                tr("Suche läuft …")
            } else if !dirs.iter().all(|d| self.recursive(d)) {
                tr("Keine Audiodateien direkt in diesem Ordner. „Mit Unterordnern“ durchsucht auch die Ordner darin.")
            } else {
                tr("Keine Audiodateien in diesem Ordner und seinen Unterordnern.")
            };
            ui.vertical_centered(|ui| ui.label(RichText::new(text).color(theme::faint())));
            return;
        }

        // Unterordner nur, wenn die Titel aus mehreren Ordnern kommen
        let cols: Vec<TrackColumn> = std::iter::once(TrackColumn::Title)
            .chain(self.columns.iter().copied().filter(|c| *c != TrackColumn::Subfolder || has_subfolders))
            .collect();
        let spacing = ui.spacing().item_spacing.x;
        // In der Historie vorn Uhrzeit und Player
        let hist_cols = hist_day.is_some();
        let (clock_w, deck_w) = (60.0, 22.0);
        // Feste Spalten (Vorhören, Nummer, A/B, ggf. Uhrzeit und Player) samt Abständen
        let fixed_w = 30.0 + 40.0 + 80.0 + spacing * (cols.len() + 3) as f32 + if hist_cols { clock_w + deck_w + spacing * 2.0 } else { 0.0 };
        // Platz für die Spalten mit veränderbarer Breite
        let space = ui.available_width() - fixed_w;
        let col_widths = fit_widths(&cols, |c| self.widths.get(&c).copied(), |c| self.width(c), space);
        // Hängen die Breiten gerade vom Platz ab (Titel füllt oder Spalten geben nach), rechnet egui sie bei
        // geänderter Fenstergröße neu; sonst behält die Tabelle die gezogenen Breiten
        let flexible = !self.widths.contains_key(&TrackColumn::Title)
            || cols.iter().zip(&col_widths).any(|(c, w)| (self.width(*c) - w).abs() > 0.5);
        let fill_key: Vec<i32> = if flexible { col_widths.iter().map(|w| (w / 4.0).round() as i32).collect() } else { Vec::new() };
        // Tatsächliche Breiten der Spalten in diesem Bild, aus dem Tabellenkopf gemessen
        let mut measured: Vec<f32> = Vec::new();
        let mut header_resp: Option<egui::Response> = None;
        let mut menu_button: Option<egui::Response> = None;
        if rows.is_empty() && searching {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(tr("Keine Titel passen zur Suche.")).color(theme::faint())));
            return;
        }
        let hist_date = hist_day.map(|d| d.date.clone());
        let picked = &mut self.picked;
        let table_id = ("tracks", self.table_gen, fill_key, &cols, hist_cols);
        ui.visuals_mut().selection.bg_fill = theme::row_selected();
        // Text ausgewählter Zeilen behält seine Farbe (egui färbt ihn sonst einheitlich ein)
        ui.visuals_mut().selection.stroke.color = theme::ink();
        // Sind die Spalten breiter als die Liste, lässt sie sich waagerecht schieben
        egui::ScrollArea::horizontal().id_salt("tracks_h").auto_shrink([false, false]).show(ui, |ui| {
            // Rahmen der Auswahl erst nach der Tabelle zeichnen, sonst überdeckt die nächste Zeile ihn im Zwischenraum
            let mut frames: Vec<(egui::Rect, bool, bool)> = Vec::new();
            // Sichtbarer Bereich der Tabellenzeilen (ohne Kopf), zusammengesetzt aus den Zellen
            let mut body_clip = egui::Rect::NOTHING;
            let mut table = TableBuilder::new(ui)
                // Eigene Kennung je Spaltensatz, damit egui keine Breiten einer anderen Aufteilung übernimmt
                .id_salt(table_id)
                .striped(true)
                // Inhalt jeder Zelle senkrecht mittig zur Zebra-Zeile
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .sense(Sense::click_and_drag())
                .auto_shrink([false, false])
                // Ohne das hält egui die Tabelle mindestens 200 Punkte hoch und bläht den Ordnerbereich auf
                .min_scrolled_height(0.0)
                .column(Column::exact(30.0))
                .column(Column::exact(40.0));
            if hist_cols {
                table = table.column(Column::exact(clock_w)).column(Column::exact(deck_w));
            }
            // Jede Spalte zieht man an ihrem rechten Rand; was rechts davon liegt, rückt mit
            for (c, w) in cols.iter().zip(&col_widths) {
                table = table.column(Column::initial(*w).at_least(c.min_width()).at_most(1200.0).clip(true).resizable(true));
            }
            table
                // Leerer Rest vor den A/B-Tasten: das Ziehen einer Spalte verschiebt keine andere,
                // und die Tasten bleiben am rechten Rand
                .column(Column::remainder())
                .column(Column::exact(80.0))
                .header(20.0, |mut h| {
                    let head = |ui: &mut egui::Ui, t: &str| {
                        ui.add(egui::Label::new(RichText::new(t).size(10.5).color(theme::faint())).truncate().selectable(false));
                    };
                    h.col(|ui| head(ui, ""));
                    h.col(|ui| head(ui, "#"));
                    if hist_cols {
                        h.col(|ui| head(ui, &tr("Gespielt").to_uppercase()));
                        h.col(|ui| head(ui, ""));
                    }
                    for c in &cols {
                        h.col(|ui| {
                            measured.push(ui.max_rect().width());
                            head(ui, &tr(c.name()).to_uppercase());
                        });
                    }
                    h.col(|_| {});
                    h.col(|ui| {
                        let b = Button::new(RichText::new(tr("SPALTEN")).size(10.5).color(theme::faint())).frame(false);
                        menu_button = Some(ui.add(b).on_hover_text(tr("Spalten auswählen (auch mit Rechtsklick auf den Tabellenkopf)")));
                    });
                    header_resp = Some(h.response());
                })
                .body(|body| {
                    body.rows(26.0, rows.len(), |mut row| {
                        let i = row.index();
                        let r = &rows[i];
                        let path = &r.path;
                        let entry = entry_of(r);
                        let gone = hist_cols && missing.contains(&r.slot);
                        if !gone && !ctx.metas.contains_key(path) {
                            out.push(BrowserAction::NeedMeta(path.clone()));
                        }
                        let m = meta_for(ctx.metas, path, entry);
                        let previewing = ctx.preview == Some(path.as_path());
                        let in_sel = picked.contains(r);
                        // Gilt eine Aktion dieser Zeile für die ganze Auswahl oder nur für diesen Titel?
                        let targets = || if in_sel { sel_paths.clone() } else if gone { Vec::new() } else { vec![path.clone()] };
                        row.set_selected(in_sel);
                        row.col(|ui| {
                            body_clip = body_clip.union(ui.clip_rect());
                            if !gone && crate::hw::preview_button(ui, vec2(28.0, 20.0), previewing).clicked() {
                                out.push(if previewing { BrowserAction::StopPreview } else { BrowserAction::Preview(path.clone()) });
                            }
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(format!("{:02}", i + 1)).monospace().size(11.0).color(theme::faint()));
                        });
                        if hist_cols {
                            row.col(|ui| {
                                let t = entry.map(|e| e.clock()).unwrap_or_default();
                                ui.label(RichText::new(if t.is_empty() { "–".into() } else { t }).monospace().size(12.0).color(theme::dim()));
                            });
                            row.col(|ui| {
                                if let Some(d) = entry.and_then(|e| e.deck).map(usize::from).filter(|d| *d < 2) {
                                    ui.label(RichText::new(theme::deck_letter(d)).font(fonts::bold(12.0)).color(theme::deck_color(d)));
                                }
                            });
                        }
                        // Voller Text der abgeschnittenen Zelle unter der Maus, als Tooltip der Zeile
                        let mut tip: Option<String> = None;
                        // Fehlende Dateien ausgegraut
                        let shade = |c: egui::Color32| if gone { theme::faint().gamma_multiply(0.8) } else { c };
                        for c in &cols {
                            row.col(|ui| {
                                let text_cell = |t: &str, size: f32, color| RichText::new(t).font(fonts::list(size)).color(shade(color));
                                let number_cell = |t: &str| RichText::new(t).monospace().size(12.0).color(shade(theme::dim()));
                                let text = match c {
                                    TrackColumn::Title => {
                                        let t = text_cell(&m.title, 12.5, theme::ink());
                                        if gone { t.strikethrough() } else { t }
                                    }
                                    TrackColumn::Artist => text_cell(&m.artist, 12.5, theme::dim()),
                                    TrackColumn::Album => text_cell(&m.album, 12.5, theme::dim()),
                                    TrackColumn::Genre => text_cell(&m.genre, 12.0, theme::dim()),
                                    TrackColumn::Year => number_cell(&m.year),
                                    TrackColumn::Comment => text_cell(&m.comment, 12.0, theme::faint()),
                                    // Unterordner relativ zum ausgewählten Ordner, leer für Titel direkt darin
                                    TrackColumn::Subfolder => text_cell(&subfolder(&dirs, path), 12.0, theme::faint()),
                                    TrackColumn::FileName => {
                                        text_cell(&path.file_name().unwrap_or_default().to_string_lossy(), 12.0, theme::faint())
                                    }
                                    TrackColumn::Format => {
                                        let ext = path.extension().unwrap_or_default().to_string_lossy().to_uppercase();
                                        text_cell(&ext, 11.5, theme::faint())
                                    }
                                    TrackColumn::Bpm => {
                                        let bpm = ctx.bpms.get(path).copied().or(m.bpm);
                                        number_cell(&bpm.map(|b| format!("{b:.0}")).unwrap_or_else(|| "–".into()))
                                    }
                                    TrackColumn::Time => {
                                        // „…“ = Tags werden noch gelesen; fehlende Dateien ohne bekannte Länge zeigen „–“
                                        let t = if m.duration_secs > 0.0 { fmt_time(m.duration_secs) } else if gone { "–".into() } else { "…".into() };
                                        number_cell(&t)
                                    }
                                };
                                elided_cell(ui, text, &mut tip);
                            });
                        }
                        let mut right_edge = f32::INFINITY;
                        row.col(|_| {});
                        row.col(|ui| {
                            right_edge = ui.max_rect().right() + ui.spacing().item_spacing.x / 2.0;
                            // Feste Größe, damit die Tasten in die Zeilenhöhe passen und mittig sitzen
                            for list in [0usize, 1] {
                                let letter = theme::deck_letter(list);
                                let b = crate::hw::button(ui, vec2(30.0, 20.0), letter, fonts::bold(12.0), crate::hw::Look::Color(theme::deck_color(list)), false, !gone);
                                let hint = if in_sel && sel_rows.len() > 1 {
                                    trf("Auswahl ({} Titel) in Playlist {}", &[&sel_paths.len(), &letter])
                                } else {
                                    trf("In Playlist {}", &[&letter])
                                };
                                if b.on_hover_text(hint).clicked() {
                                    out.push(BrowserAction::Add { list, paths: targets() });
                                }
                            }
                        });
                        let mut resp = row.response();
                        if gone {
                            tip = Some(trf("Datei nicht gefunden: {}", &[&path.display()]));
                        }
                        if let Some(t) = tip {
                            resp = resp.on_hover_ui_at_pointer(|ui| {
                                ui.label(t);
                            });
                        }
                        if in_sel {
                            // Rahmen um die Auswahl; zusammenhängende Zeilen bekommen einen gemeinsamen Rahmen
                            let above = i > 0 && picked.contains(&rows[i - 1]);
                            let below = rows.get(i + 1).is_some_and(|p| picked.contains(p));
                            // Die Zeilenfläche reicht über die letzte Spalte hinaus; der Rahmen endet an der Spalte
                            // Wie die Hinterlegung über den halben Zeilenabstand hinweg, damit die Kanten durchlaufen
                            let gap = resp.ctx.global_style().spacing.item_spacing.y / 2.0;
                            let mut fr = resp.rect.expand2(vec2(-1.0, gap)).shrink(0.75);
                            fr.max.x = fr.max.x.min(right_edge - 1.5);
                            frames.push((fr, above, below));
                        }
                        if resp.clicked() {
                            let m = resp.ctx.input(|i| i.modifiers);
                            picked.click(&rows, r, m.command, m.shift);
                        }
                        if resp.secondary_clicked() && !in_sel {
                            picked.select_only(r);
                        }
                        if resp.drag_started() {
                            let t = targets();
                            if !t.is_empty() {
                                resp.dnd_set_drag_payload(DragItem::Files(t));
                            }
                        }
                        // In der Historie lassen sich Einträge (auch fehlende) wieder entfernen
                        let remove = hist_date.as_ref().map(|date| {
                            let slots = if in_sel { sel_rows.iter().map(|r| r.slot).collect() } else { vec![r.slot] };
                            (date.clone(), slots)
                        });
                        resp.context_menu(|ui| track_menu(ui, &targets(), remove.clone(), ctx, out));
                    });
                });
            let stroke = egui::Stroke::new(1.5, theme::select_accent());
            for (r, above, below) in frames {
                let clip = egui::Rect::from_x_y_ranges(r.expand(2.0).x_range(), body_clip.y_range()).intersect(ui.clip_rect());
                let p = egui::Painter::new(ui.ctx().clone(), ui.layer_id(), clip);
                p.vline(r.left(), r.y_range(), stroke);
                p.vline(r.right(), r.y_range(), stroke);
                if !above {
                    p.hline(r.x_range(), r.top(), stroke);
                }
                if !below {
                    p.hline(r.x_range(), r.bottom(), stroke);
                }
            }
        });
        // Spaltenwahl: Rechtsklick auf den Kopf oder Taste „Spalten“; bleibt beim Anhaken offen
        if let Some(h) = header_resp {
            egui::Popup::context_menu(&h)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.columns_menu(ui, out));
        }
        if let Some(b) = menu_button {
            egui::Popup::menu(&b)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| self.columns_menu(ui, out));
        }
        // Gezogene Breiten merken, sobald die Maus losgelassen ist
        // Im unsichtbaren Messdurchgang von egui haben die Zellen noch keine Fläche
        let valid = measured.len() == cols.len() && measured.iter().all(|w| w.is_finite() && *w > 0.0);
        if valid && !ui.is_sizing_pass() && !ui.input(|i| i.pointer.any_down()) {
            let mut changed = false;
            for (c, (w, old)) in cols.iter().zip(measured.iter().zip(&col_widths)) {
                if (w - old).abs() > 0.5 {
                    self.widths.insert(*c, w.round());
                    changed = true;
                }
            }
            if changed {
                self.columns_changed(out);
            }
        }
    }
}

/// Zelle mit Text, der bei zu schmaler Spalte mit „…“ endet. Ist er abgeschnitten und steht die Maus
/// darüber, landet der volle Text in `tip`. Der eigene Tooltip des Labels kommt unter der anklickbaren
/// Zeile nicht zuverlässig an, deshalb zeigt ihn die Zeile.
fn elided_cell(ui: &mut egui::Ui, text: RichText, tip: &mut Option<String>) {
    let full_w = egui::WidgetText::from(text.clone())
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::FontSelection::Default)
        .size()
        .x;
    if full_w > ui.available_width() + 0.5 && ui.rect_contains_pointer(ui.max_rect()) {
        *tip = Some(text.text().to_owned());
    }
    ui.add(egui::Label::new(text).truncate().show_tooltip_when_elided(false).selectable(false));
}

/// Tasten über der Titelliste (BPM ermitteln, in Playlist A/B), rechtsbündig. Liefert die belegte Breite.
fn list_tools(ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, rows: &[PathBuf], sel_rows: &[PathBuf], history: bool, out: &mut Vec<BrowserAction>) -> f32 {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Mit Auswahl gelten die Tasten nur für die ausgewählten Titel
        let (paths, label) = match (sel_rows.is_empty(), history) {
            (false, _) => (sel_rows, "Auswahl in Playlist {}"),
            (true, false) => (rows, "Ordner in Playlist {}"),
            (true, true) => (rows, "Alle in Playlist {}"),
        };
        for list in [1usize, 0] {
            let c = theme::deck_color(list);
            let b = Button::new(RichText::new(trf(label, &[&theme::deck_letter(list)])).color(c).strong())
                .fill(c.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, c.gamma_multiply(0.6)));
            if ui.add_enabled(!paths.is_empty(), b).clicked() {
                out.push(BrowserAction::Add { list, paths: paths.to_vec() });
            }
        }
        // BPM aus dem Tag zählt als bekannt; Titel mit noch ungelesenen Tags gelten als offen
        let known = |p: &PathBuf| ctx.bpms.contains_key(p) || ctx.metas.get(p).is_some_and(|m| m.bpm.is_some());
        let missing = paths.iter().filter(|p| !known(p)).count();
        let tip = if missing == 0 {
            tr("Alle BPM sind bekannt")
        } else if sel_rows.is_empty() {
            tr("Fehlende BPM der Titel in der Liste im Hintergrund ermitteln (BPM aus den Tags werden übernommen)")
        } else {
            tr("Fehlende BPM der ausgewählten Titel im Hintergrund ermitteln (BPM aus den Tags werden übernommen)")
        };
        if ui.add_enabled(missing > 0, Button::new(tr("BPM ermitteln"))).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
            out.push(BrowserAction::AnalyzeBpm { paths: paths.to_vec(), force: false });
        }
        ui.min_rect().width()
    })
    .inner
}

/// Eintrag im Ordnerbaum, ausgewählt mit Rahmen in der Auswahlfarbe. Selbst gezeichnet mit fester Größe:
/// Der egui-Button bekommt beim Überfahren einen Rahmen, der den Text um einen Punkt verschiebt.
fn tree_button(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), fonts::list(12.5), theme::dim());
    let pad = ui.spacing().button_padding;
    let size = vec2(galley.size().x + 2.0 * pad.x, (galley.size().y + 2.0 * pad.y).max(ui.spacing().interact_size.y));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered();
        if selected {
            p.rect_filled(rect, 4.0, theme::row_selected());
        } else if hovered {
            p.rect_filled(rect, 4.0, theme::row_hover());
        }
        let color = if selected || hovered { theme::ink() } else { theme::dim() };
        let pos = egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y / 2.0);
        p.galley_with_override_text_color(pos, galley, color);
        if selected {
            p.rect_stroke(rect, 4.0, egui::Stroke::new(1.5, theme::select_accent()), egui::StrokeKind::Inside);
        }
    }
    resp
}

/// Rechtsklick-Menü eines Tages der Historie.
enum DayMenu {
    Add(usize),
    Distribute,
    Rename,
    Export(ExportFormat),
    Delete,
}

fn history_menu(ui: &mut egui::Ui) -> Option<DayMenu> {
    let mut pick = None;
    for list in [0usize, 1] {
        if ui.button(trf("An Playlist {} anhängen", &[&theme::deck_letter(list)])).clicked() {
            pick = Some(DayMenu::Add(list));
        }
    }
    let hint = tr("Abwechselnd ein Titel nach A, der nächste nach B. Passt zum Auto-Fade.");
    if ui.button(tr("Auf Playlist A und B verteilen")).on_hover_text(hint).clicked() {
        pick = Some(DayMenu::Distribute);
    }
    ui.separator();
    if ui.button(tr("Umbenennen …")).on_hover_text(tr("Eigener Name für diesen Tag, z. B. „Omas Geburtstag“")).clicked() {
        pick = Some(DayMenu::Rename);
    }
    ui.menu_button(tr("Exportieren"), |ui| {
        for (format, label, tip) in [
            (ExportFormat::M3u, "Als Playlist (M3U8) …", "Lässt sich in fast jedem Player öffnen"),
            (ExportFormat::Csv, "Als Tabelle (CSV) …", "Uhrzeit, Interpret, Titel, Album, Länge – etwa für Excel oder eine Musikfolge-Meldung"),
            (ExportFormat::Json, "Als FadeWerk-DJ-Historie …", "Mit allen Angaben, zum Sichern oder für einen anderen Rechner"),
        ] {
            if ui.button(tr(label)).on_hover_text(tr(tip)).clicked() {
                pick = Some(DayMenu::Export(format));
            }
        }
    });
    ui.separator();
    if ui.button(RichText::new(tr("Tag löschen …")).color(theme::cue())).clicked() {
        pick = Some(DayMenu::Delete);
    }
    if pick.is_some() {
        ui.close();
    }
    pick
}

enum FolderMenu {
    Op(FolderOp),
    Reveal,
    Remove,
}

/// Rechtsklick-Menü eines Ordners. `count` = Anzahl der Ordner, für die es gilt.
fn folder_menu(ui: &mut egui::Ui, count: usize, removable: bool) -> Option<FolderMenu> {
    let mut pick = None;
    if count > 1 {
        ui.label(RichText::new(trf("{} Ordner ausgewählt", &[&count])).color(theme::dim()));
        ui.separator();
    }
    for list in [0usize, 1] {
        if ui.button(trf("An Playlist {} anhängen", &[&theme::deck_letter(list)])).clicked() {
            pick = Some(FolderMenu::Op(FolderOp::Add(list)));
        }
    }
    for list in [0usize, 1] {
        if ui.button(trf("Gemischt an Playlist {} anhängen", &[&theme::deck_letter(list)])).clicked() {
            pick = Some(FolderMenu::Op(FolderOp::Shuffled(list)));
        }
    }
    let hint = tr("Abwechselnd ein Titel nach A, der nächste nach B. Passt zum Auto-Fade.");
    if ui.button(tr("Auf Playlist A und B verteilen")).on_hover_text(hint).clicked() {
        pick = Some(FolderMenu::Op(FolderOp::Distribute));
    }
    ui.separator();
    let hint = tr("Im Hintergrund für alle Titel ohne BPM-Angabe im Tag. Die Werte werden gemerkt.");
    if ui.button(tr("Fehlende BPM ermitteln")).on_hover_text(hint).clicked() {
        pick = Some(FolderMenu::Op(FolderOp::AnalyzeBpm));
    }
    ui.separator();
    if ui.button(reveal_label()).clicked() {
        pick = Some(FolderMenu::Reveal);
    }
    if removable {
        ui.separator();
        if ui.button(tr("Aus der Liste entfernen")).clicked() {
            pick = Some(FolderMenu::Remove);
        }
    }
    if pick.is_some() {
        ui.close();
    }
    pick
}

/// Rechtsklick-Menü für einen oder mehrere Titel.
/// `remove` = Tag und Einträge der Historie, die „Aus der Historie entfernen“ löscht (nur in der Historie).
fn track_menu(ui: &mut egui::Ui, paths: &[PathBuf], remove: Option<(String, Vec<usize>)>, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
    let mut act = None;
    if let Some((date, slots)) = remove {
        let label = if slots.len() > 1 { trf("{} Einträge aus der Historie entfernen", &[&slots.len()]) } else { tr("Aus der Historie entfernen").to_owned() };
        if ui.button(label).clicked() {
            act = Some(BrowserAction::HistoryRemove { date, slots });
        }
        if !paths.is_empty() {
            ui.separator();
        }
    }
    if paths.is_empty() {
        if let Some(a) = act {
            out.push(a);
            ui.close();
        }
        return;
    }
    if paths.len() > 1 {
        ui.label(RichText::new(trf("{} Titel ausgewählt", &[&paths.len()])).color(theme::dim()));
        ui.separator();
    }
    for list in [0usize, 1] {
        if ui.button(trf("An Playlist {} anhängen", &[&theme::deck_letter(list)])).clicked() {
            act = Some(BrowserAction::Add { list, paths: paths.to_vec() });
        }
    }
    for list in [0usize, 1] {
        let b = ui.button(trf("Als Nächstes in Playlist {}", &[&theme::deck_letter(list)]));
        if b.on_hover_text(tr("Direkt hinter den geladenen Titel")).clicked() {
            act = Some(BrowserAction::AddNext { list, paths: paths.to_vec() });
        }
    }
    if paths.len() > 1 {
        let hint = tr("Abwechselnd ein Titel nach A, der nächste nach B. Passt zum Auto-Fade.");
        if ui.button(tr("Auf Playlist A und B verteilen")).on_hover_text(hint).clicked() {
            act = Some(BrowserAction::Distribute(paths.to_vec()));
        }
    }
    ui.separator();
    let hint = tr("Tempo im Hintergrund neu ermitteln, auch wenn eine BPM-Angabe im Tag steht. Der Wert wird gemerkt.");
    if ui.button(tr("BPM analysieren")).on_hover_text(hint).clicked() {
        act = Some(BrowserAction::AnalyzeBpm { paths: paths.to_vec(), force: true });
    }
    if let [path] = paths {
        ui.separator();
        for deck in [0usize, 1] {
            let b = ui.add_enabled(!ctx.deck_playing[deck], Button::new(trf("In Player {} laden", &[&theme::deck_letter(deck)])));
            if b.on_disabled_hover_text(tr("Spielt gerade. Erst stoppen.")).clicked() {
                act = Some(BrowserAction::LoadDeck { deck, path: path.clone() });
            }
        }
        if ctx.preview == Some(path.as_path()) {
            if ui.button(tr("Vorhören beenden")).clicked() {
                act = Some(BrowserAction::StopPreview);
            }
        } else if ui.button(tr("Vorhören")).clicked() {
            act = Some(BrowserAction::Preview(path.clone()));
        }
        ui.separator();
        if ui.button(tr("Titelinfo & Tags …")).clicked() {
            act = Some(BrowserAction::EditTags(path.clone()));
        }
        if ui.button(reveal_label()).clicked() {
            act = Some(BrowserAction::Reveal(path.clone()));
        }
    }
    if let Some(a) = act {
        out.push(a);
        ui.close();
    }
}

/// Standard für „Mit Unterordnern“: an für normale Ordner, aus für Laufwerke, den Benutzerordner,
/// den Laufwerks-Sammelordner und Systemordner. Dort würde eine Suche sonst die ganze Platte lesen.
pub fn default_recursive(path: &Path) -> bool {
    // Wurzel eines Laufwerks: `/`, `C:\`
    if path.parent().is_none() {
        return false;
    }
    if dirs::home_dir().is_some_and(|h| h == path) || is_system_dir(path) {
        return false;
    }
    // Eingehängte Laufwerke und ihr Sammelordner: /Volumes, /Volumes/X, /media, /media/user/X, /mnt/X
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let mount = |base: &str, depth: usize| parts.get(1).is_some_and(|p| p == base) && parts.len() <= depth;
    !(mount("Volumes", 3) || mount("media", 4) || mount("mnt", 3) || (mount("run", 5) && parts.get(2).is_some_and(|p| p == "media")))
}

pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        tr("Im Finder zeigen")
    } else if cfg!(windows) {
        tr("Im Explorer zeigen")
    } else {
        tr("Im Dateimanager zeigen")
    }
}

fn dir_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Unterordner eines Titels relativ zum ausgewählten Ordner, in dem er liegt. Bei mehreren
/// ausgewählten Ordnern mit dessen Namen davor, damit klar ist, woher der Titel stammt.
fn subfolder(dirs: &[PathBuf], path: &Path) -> String {
    let Some(parent) = path.parent() else { return String::new() };
    let Some(base) = dirs.iter().filter(|d| parent.starts_with(d)).max_by_key(|d| d.as_os_str().len()) else {
        return String::new();
    };
    let rel = parent.strip_prefix(base).unwrap_or(parent);
    if dirs.len() > 1 {
        Path::new(&dir_name(base)).join(rel).display().to_string()
    } else {
        rel.display().to_string()
    }
}

/// Höhe der Vorhör-Leiste unter der Titelliste.
const PREVIEW_BAR_H: f32 = 40.0;

fn preview_bar(ui: &mut egui::Ui, rect: egui::Rect, path: &Path, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
    // Rot umrandet, damit sofort auffällt, dass gerade etwas im Kopfhörer läuft
    let red = theme::cue();
    ui.painter().rect(rect, 5.0, theme::mix(theme::panel2(), red, 0.14), egui::Stroke::new(1.0, red.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let inner = rect.shrink2(vec2(8.0, 4.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        if crate::hw::preview_button(ui, vec2(34.0, 26.0), true).clicked() {
            out.push(BrowserAction::StopPreview);
        }
        let label = if theme::skin().dark { crate::hw::lighten(red, 0.45) } else { crate::hw::darken(red, 0.2) };
        ui.label(RichText::new(tr("IM KOPFHÖRER")).font(fonts::bold(11.0)).color(label));
        // Rechts Zeit und Positionsregler, der Titel nimmt den Platz dazwischen und wird notfalls gekürzt
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{} / {}", fmt_time(ctx.preview_pos), fmt_time(ctx.preview_dur))).monospace().size(12.0).color(theme::dim()));
            let mut pos = ctx.preview_pos;
            ui.spacing_mut().slider_width = (ui.available_width() * 0.45).clamp(80.0, 500.0);
            if ui.add(egui::Slider::new(&mut pos, 0.0..=ctx.preview_dur.max(0.1)).show_value(false)).changed() {
                out.push(BrowserAction::PreviewSeek(pos));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let title = ctx.metas.get(path).map(|m| format!("{} · {}", m.title, m.artist)).unwrap_or_else(|| TrackMeta::from_filename(path).title);
                ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
            });
        });
    });
}


/// Taste für das Vollbild: vier Ecken, die nach außen (Vollbild an) oder innen (Vollbild aus) zeigen.
/// Selbst gezeichnet, weil die Schriften kein passendes Zeichen haben.
/// Breiten der Spalten, die in `space` passen. Ohne gezogene Breite füllt der Titel den Rest.
/// Reicht der Platz nicht, gibt erst der Titel nach, dann alle übrigen gleichmäßig bis zur Mindestbreite.
/// So bleiben die A/B-Tasten am rechten Rand sichtbar; gespeichert bleiben die gewünschten Breiten.
fn fit_widths(cols: &[TrackColumn], saved: impl Fn(TrackColumn) -> Option<f32>, width: impl Fn(TrackColumn) -> f32, space: f32) -> Vec<f32> {
    let others: f32 = cols.iter().filter(|c| **c != TrackColumn::Title).map(|c| width(*c)).sum();
    let title_min = TrackColumn::Title.min_width();
    let title = saved(TrackColumn::Title).map_or(space - others, |w| w.min(space - others)).max(title_min);
    let mut out: Vec<f32> = cols.iter().map(|c| if *c == TrackColumn::Title { title } else { width(*c) }).collect();
    let excess = out.iter().sum::<f32>() - space;
    if excess > 0.0 {
        let slack: f32 = cols.iter().zip(&out).filter(|(c, _)| **c != TrackColumn::Title).map(|(c, w)| w - c.min_width()).sum();
        if slack > 0.0 {
            let k = (1.0 - excess / slack).max(0.0);
            for (c, w) in cols.iter().zip(out.iter_mut()).filter(|(c, _)| **c != TrackColumn::Title) {
                *w = c.min_width() + (*w - c.min_width()) * k;
            }
        }
    }
    out
}

/// Lupe für die erweiterte Suche, selbst gezeichnet (das Zeichen fehlt in den Schriften).
/// Leuchtet in der Auswahlfarbe, solange Filter aktiv sind oder das Fenster offen ist.
fn search_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    let tip = if cfg!(target_os = "macos") { tr("Erweiterte Suche (⇧⌘F)") } else { tr("Erweiterte Suche (Strg+Umschalt+F)") };
    let resp = ui.add(egui::Button::new("").min_size(egui::vec2(h + 4.0, h))).on_hover_text(tip);
    let color = if active { theme::select_accent() } else { ui.style().interact(&resp).fg_stroke.color };
    let stroke = egui::Stroke::new(1.7, color);
    let c = resp.rect.center() + egui::vec2(-1.5, -1.5);
    let r = 4.5;
    let p = ui.painter();
    p.circle_stroke(c, r, stroke);
    let d = egui::vec2(1.0, 1.0).normalized();
    p.line_segment([c + d * (r + 0.5), c + d * (r + 5.0)], egui::Stroke::new(2.2, color));
    resp
}

fn fullscreen_button(ui: &mut egui::Ui, fullscreen: bool) -> egui::Response {
    let tip = match (fullscreen, cfg!(target_os = "macos")) {
        (false, true) => tr("Vollbild (⌃⌘F)"),
        (false, false) => tr("Vollbild (F11)"),
        (true, true) => tr("Vollbild beenden (⌃⌘F)"),
        (true, false) => tr("Vollbild beenden (F11)"),
    };
    let h = ui.spacing().interact_size.y;
    let resp = ui.add(egui::Button::new("").min_size(egui::vec2(h + 4.0, h))).on_hover_text(tip);
    let color = ui.style().interact(&resp).fg_stroke.color;
    let stroke = egui::Stroke::new(1.6, color);
    let c = resp.rect.center();
    let (r, l) = (5.5, 3.5);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = c + egui::vec2(sx * r, sy * r);
        // Vollbild an: Ecken zeigen nach außen; aus: der Winkel sitzt innen und zeigt zur Mitte
        let (dx, dy) = if fullscreen { (sx * l, sy * l) } else { (-sx * l, -sy * l) };
        let vertex = if fullscreen { corner - egui::vec2(sx * l, sy * l) } else { corner };
        let p = ui.painter();
        p.line_segment([vertex, vertex + egui::vec2(dx, 0.0)], stroke);
        p.line_segment([vertex, vertex + egui::vec2(0.0, dy)], stroke);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_fit_so_the_deck_buttons_stay_visible() {
        use TrackColumn::*;
        let cols = [Title, Artist, Time];
        let width = |c: TrackColumn| match c {
            Title => 300.0,
            Artist => 200.0,
            _ => 50.0,
        };
        // Titel ohne gezogene Breite füllt den Rest
        assert_eq!(fit_widths(&cols, |_| None, width, 600.0), vec![350.0, 200.0, 50.0]);
        // Gezogene Breite bleibt, solange sie passt
        assert_eq!(fit_widths(&cols, |c| (c == Title).then_some(300.0), width, 600.0), vec![300.0, 200.0, 50.0]);
        // Zu eng: erst der Titel bis zur Mindestbreite, dann die übrigen gleichmäßig
        let w = fit_widths(&cols, |c| (c == Title).then_some(300.0), width, 300.0);
        assert_eq!(w[0], Title.min_width());
        assert!((w.iter().sum::<f32>() - 300.0).abs() < 0.01, "{w:?}");
        assert!(w[1] >= Artist.min_width() && w[2] >= Time.min_width());
    }

    #[test]
    fn subfolders_off_for_drives_and_home() {
        let root = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        assert!(!default_recursive(&root));
        if let Some(home) = dirs::home_dir() {
            assert!(!default_recursive(&home));
            assert!(default_recursive(&home.join("Music")));
            assert!(!default_recursive(&home.join("Library")));
        }
        if !cfg!(windows) {
            assert!(!default_recursive(Path::new("/Volumes")));
            assert!(!default_recursive(Path::new("/Volumes/USB-Stick")));
            assert!(default_recursive(Path::new("/Volumes/USB-Stick/Musik")));
            assert!(!default_recursive(Path::new("/media/dj/STICK")));
            assert!(default_recursive(Path::new("/media/dj/STICK/Musik")));
        }
    }
}
