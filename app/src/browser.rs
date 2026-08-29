// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordnerbrowser: Ordner anklicken, Titel sind sofort da. Kein Einlesen in eine Datenbank.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Button, CursorIcon, Key, RichText, Sense, vec2};
use egui_extras::{Column, TableBuilder};
use fw_library::{SCAN_LIMIT, TrackMeta, list_dir};

use crate::deck_ui::fmt_time;
use crate::playlist::DragItem;
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
    /// Der Nutzer hat die Breite des Ordnerbaums geändert.
    TreeWidth(f32),
    Preview(PathBuf),
    StopPreview,
    PreviewSeek(f64),
    NeedMeta(PathBuf),
    AddRoot,
    RemoveRoot(PathBuf),
    OpenSettings,
}

pub struct BrowserCtx<'a> {
    pub metas: &'a HashMap<PathBuf, TrackMeta>,
    pub bpms: &'a HashMap<PathBuf, f32>,
    pub preview: Option<&'a Path>,
    pub preview_pos: f64,
    pub preview_dur: f64,
    /// Spielt Player A bzw. B gerade? Dann lässt er sich nicht neu laden.
    pub deck_playing: [bool; 2],
}

/// Was ein Ordner-Kontextmenü mit allen Titeln der ausgewählten Ordner macht. Läuft die Suche
/// noch, wird die Aktion vorgemerkt und ausgeführt, sobald alle Titel gefunden sind.
#[derive(Clone, Copy)]
enum FolderOp {
    Add(usize),
    Shuffled(usize),
    Distribute,
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
    picked: Selection<PathBuf>,
    /// Ordner-Aktion, die auf das Ende der Suche wartet.
    queued: Option<FolderOp>,
    tree_width: f32,
    /// Entwicklerschalter `FW_SELECT`: wählt zum Prüfen einige Titel aus.
    pub demo_select: bool,
    scanning: bool,
    limited: bool,
    scan_token: u64,
    pending_scan: Option<Vec<PathBuf>>,
    error: Option<String>,
    filter: String,
    /// Für diesen Suchtext wurden die Tags aller Titel schon angefordert.
    meta_requested_for: String,
    subdirs: HashMap<PathBuf, Vec<PathBuf>>,
}

impl Browser {
    /// `user_roots` sind gespeicherte, vom Nutzer hinzugefügte Ordner, `tree_width` die gespeicherte
    /// Breite des Ordnerbaums (0 = Standard).
    pub fn new(start_dir: Option<PathBuf>, user_roots: &[PathBuf], tree_width: f32) -> Self {
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
            queued: None,
            tree_width: if tree_width > 0.0 { tree_width } else { DEFAULT_TREE_W },
            demo_select: false,
            scanning: false,
            limited: false,
            scan_token: 0,
            pending_scan: None,
            error: None,
            filter: String::new(),
            meta_requested_for: String::new(),
            subdirs: HashMap::new(),
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
        self.selected = vec![path];
        self.filter.clear();
        self.rescan();
    }

    /// ⌘/Strg-Klick: Ordner zur Auswahl hinzufügen oder herausnehmen. Einer bleibt immer ausgewählt.
    fn toggle_folder(&mut self, path: &Path) {
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

    fn rescan(&mut self) {
        self.tracks.clear();
        self.limited = false;
        self.picked.clear();
        self.queued = None;
        self.meta_requested_for.clear();
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
        // Liegt ein ausgewählter Ordner in einem anderen ausgewählten, wird er nicht doppelt durchsucht
        let roots: Vec<PathBuf> = readable.iter().filter(|p| !readable.iter().any(|q| q != *p && p.starts_with(q))).cloned().collect();
        self.scanning = !roots.is_empty();
        self.pending_scan = (!roots.is_empty()).then_some(roots);
    }

    /// Ordner, für die eine Suche gestartet werden soll.
    pub fn take_pending_scan(&mut self) -> Option<Vec<PathBuf>> {
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
            });
        }

        // Kopfzeile
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("ORDNER")).font(fonts::bold(16.0)).extra_letter_spacing(1.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(RichText::new("⚙").size(15.0)).on_hover_text(tr("Einstellungen (⌘,)")).clicked() {
                    out.push(BrowserAction::OpenSettings);
                }
                if ui.button(tr("Ordner hinzufügen …")).clicked() {
                    out.push(BrowserAction::AddRoot);
                }
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(tr("Suchen (mit Unterordnern)")).desired_width(200.0));
                ui.label(RichText::new(tr("⏺ Live gelesen, keine Datenbank")).size(12.0).color(theme::play()));
                if !self.selected.is_empty() {
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
        let selected = self.selected.iter().any(|p| p == path);
        // Ein Menü auf einem ausgewählten Ordner gilt für alle ausgewählten
        let count = if selected { self.selected.len() } else { 1 };
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0 && selected);
        let mut open_this = false;
        let mut toggle_this = false;
        let mut menu = None;
        state
            .show_header(ui, |ui| {
                let icon = if depth == 0 { "🗀" } else { "📁" };
                let text = RichText::new(format!("{icon} {name}")).font(fonts::list(12.5)).color(if selected { theme::ink() } else { theme::dim() });
                let resp = ui.add(Button::selectable(selected, text).frame_when_inactive(false));
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

    fn track_list(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        if self.selected.is_empty() {
            return;
        }
        let dirs = self.selected.clone();
        let f = self.filter.trim().to_lowercase();
        // Für die Suche nach Interpret und Titel einmal alle Tags im Hintergrund anfordern
        if !f.is_empty() && self.meta_requested_for.is_empty() {
            out.extend(self.tracks.iter().filter(|p| !ctx.metas.contains_key(*p)).map(|p| BrowserAction::NeedMeta(p.clone())));
            self.meta_requested_for = f.clone();
        } else if f.is_empty() {
            self.meta_requested_for.clear();
        }
        // Alle Suchwörter müssen vorkommen, in beliebiger Reihenfolge
        let words: Vec<&str> = f.split_whitespace().collect();
        let rows: Vec<PathBuf> = self
            .tracks
            .iter()
            .filter(|p| {
                if words.is_empty() {
                    return true;
                }
                let mut hay = format!("{} {}", subfolder(&dirs, p), p.file_name().unwrap_or_default().to_string_lossy()).to_lowercase();
                if let Some(m) = ctx.metas.get(*p) {
                    hay.push(' ');
                    hay.push_str(&m.title.to_lowercase());
                    hay.push(' ');
                    hay.push_str(&m.artist.to_lowercase());
                    hay.push(' ');
                    hay.push_str(&m.album.to_lowercase());
                }
                words.iter().all(|w| hay.contains(w))
            })
            .cloned()
            .collect();
        let has_subfolders = dirs.len() > 1 || self.tracks.iter().any(|p| p.parent() != Some(dirs[0].as_path()));

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

        ui.horizontal(|ui| {
            let folder = match dirs.as_slice() {
                [one] => dir_name(one),
                many => trf("{} Ordner", &[&many.len()]),
            };
            ui.label(RichText::new(folder).font(fonts::bold(17.0)));
            let total: f64 = rows.iter().filter_map(|p| ctx.metas.get(p)).map(|m| m.duration_secs).sum();
            let count = if words.is_empty() {
                count_tracks(rows.len(), &fmt_time(total))
            } else {
                trf("{} von {} Titeln", &[&rows.len(), &self.tracks.len()])
            };
            ui.label(RichText::new(count).color(theme::dim()));
            if !sel_rows.is_empty() {
                ui.label(RichText::new(trf("· {} ausgewählt", &[&sel_rows.len()])).font(fonts::list(12.5)).color(theme::ink()));
            }
            if self.scanning {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(tr("Durchsucht Unterordner …")).size(12.0).color(theme::dim()));
            } else if self.limited {
                ui.label(RichText::new(trf("Nur die ersten {} Titel. Für mehr einen Unterordner wählen.", &[&SCAN_LIMIT])).size(12.0).color(theme::yellow()));
            }
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).size(12.0).color(theme::cue()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Mit Auswahl gelten die Tasten nur für die ausgewählten Titel
                let (paths, label) = if sel_rows.is_empty() { (&rows, "Ordner in Playlist {}") } else { (&sel_rows, "Auswahl in Playlist {}") };
                for list in [1usize, 0] {
                    let c = theme::deck_color(list);
                    let b = Button::new(RichText::new(trf(label, &[&theme::deck_letter(list)])).color(c).strong())
                        .fill(c.gamma_multiply(0.12))
                        .stroke(egui::Stroke::new(1.0, c.gamma_multiply(0.6)));
                    if ui.add_enabled(!paths.is_empty(), b).clicked() {
                        out.push(BrowserAction::Add { list, paths: paths.clone() });
                    }
                }
            });
        });

        if self.tracks.is_empty() {
            ui.add_space(20.0);
            let text = if self.scanning { tr("Suche läuft …") } else { tr("Keine Audiodateien in diesem Ordner und seinen Unterordnern.") };
            ui.vertical_centered(|ui| ui.label(RichText::new(text).color(theme::faint())));
            return;
        }

        let picked = &mut self.picked;
        ui.visuals_mut().selection.bg_fill = theme::row_selected();
        // Text ausgewählter Zeilen behält seine Farbe (egui färbt ihn sonst einheitlich ein)
        ui.visuals_mut().selection.stroke.color = theme::ink();
        TableBuilder::new(ui)
            .id_salt("tracks")
            .striped(true)
            // Inhalt jeder Zelle senkrecht mittig zur Zebra-Zeile
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .sense(Sense::click_and_drag())
            .auto_shrink([false, false])
            // Ohne das hält egui die Tabelle mindestens 200 Punkte hoch und bläht den Ordnerbereich auf
            .min_scrolled_height(0.0)
            .column(Column::exact(30.0))
            .column(Column::exact(40.0))
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::initial(180.0).at_least(80.0).clip(true))
            .column(Column::initial(if has_subfolders { 200.0 } else { 1.0 }).at_least(1.0).clip(true))
            .column(Column::exact(52.0))
            .column(Column::exact(48.0))
            .column(Column::exact(80.0))
            .header(20.0, |mut h| {
                for t in ["", "#", "TITEL", "INTERPRET", if has_subfolders { "UNTERORDNER" } else { "" }, "BPM", "ZEIT", ""] {
                    h.col(|ui| {
                        ui.label(RichText::new(tr(t)).size(10.5).color(theme::faint()));
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, rows.len(), |mut row| {
                    let i = row.index();
                    let path = &rows[i];
                    let meta = ctx.metas.get(path);
                    if meta.is_none() {
                        out.push(BrowserAction::NeedMeta(path.clone()));
                    }
                    let fallback = TrackMeta::from_filename(path);
                    let m = meta.unwrap_or(&fallback);
                    let previewing = ctx.preview == Some(path.as_path());
                    let in_sel = picked.contains(path);
                    // Gilt eine Aktion dieser Zeile für die ganze Auswahl oder nur für diesen Titel?
                    let targets = || if in_sel { sel_rows.clone() } else { vec![path.clone()] };
                    row.set_selected(in_sel);
                    row.col(|ui| {
                        if in_sel {
                            // Markierung am linken Rand, damit die Auswahl auch neben Zebra-Streifen auffällt
                            let r = ui.max_rect().expand2(vec2(0.0, 3.0));
                            ui.painter().rect_filled(egui::Rect::from_min_size(r.left_top(), vec2(3.0, r.height())), 1.0, theme::ink());
                        }
                        if crate::hw::preview_button(ui, vec2(28.0, 20.0), previewing).clicked() {
                            out.push(if previewing { BrowserAction::StopPreview } else { BrowserAction::Preview(path.clone()) });
                        }
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(format!("{:02}", i + 1)).monospace().size(11.0).color(theme::faint()));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&m.title).font(fonts::list(12.5))).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&m.artist).font(fonts::list(12.5)).color(theme::dim())).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        // Unterordner relativ zum ausgewählten Ordner, leer für Titel direkt darin
                        ui.add(egui::Label::new(RichText::new(subfolder(&dirs, path)).font(fonts::list(12.0)).color(theme::faint())).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        let bpm = ctx.bpms.get(path).copied().or(m.bpm);
                        let text = bpm.map(|b| format!("{b:.0}")).unwrap_or_else(|| "–".into());
                        ui.label(RichText::new(text).monospace().size(12.0).color(theme::dim()));
                    });
                    row.col(|ui| {
                        let t = if m.duration_secs > 0.0 { fmt_time(m.duration_secs) } else { "…".into() };
                        ui.label(RichText::new(t).monospace().size(12.0).color(theme::dim()));
                    });
                    row.col(|ui| {
                        // Feste Größe, damit die Tasten in die Zeilenhöhe passen und mittig sitzen
                        for list in [0usize, 1] {
                            let letter = theme::deck_letter(list);
                            let b = crate::hw::button(ui, vec2(30.0, 20.0), letter, fonts::bold(12.0), crate::hw::Look::Color(theme::deck_color(list)), false, true);
                            let hint = if in_sel && sel_rows.len() > 1 {
                                trf("Auswahl ({} Titel) in Playlist {}", &[&sel_rows.len(), &letter])
                            } else {
                                trf("In Playlist {}", &[&letter])
                            };
                            if b.on_hover_text(hint).clicked() {
                                out.push(BrowserAction::Add { list, paths: targets() });
                            }
                        }
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        let m = resp.ctx.input(|i| i.modifiers);
                        picked.click(&rows, path, m.command, m.shift);
                    }
                    if resp.secondary_clicked() && !in_sel {
                        picked.select_only(path);
                    }
                    if resp.drag_started() {
                        resp.dnd_set_drag_payload(DragItem::Files(targets()));
                    }
                    resp.context_menu(|ui| track_menu(ui, &targets(), ctx, out));
                });
            });
    }
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
fn track_menu(ui: &mut egui::Ui, paths: &[PathBuf], ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
    let mut act = None;
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
        if ui.button(reveal_label()).clicked() {
            act = Some(BrowserAction::Reveal(path.clone()));
        }
    }
    if let Some(a) = act {
        out.push(a);
        ui.close();
    }
}

fn reveal_label() -> &'static str {
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
