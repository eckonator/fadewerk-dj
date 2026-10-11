// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordnerbrowser: Ordner anklicken, Titel sind sofort da. Kein Einlesen in eine Datenbank.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Button, RichText, Sense, vec2};
use egui_extras::{Column, TableBuilder};
use fw_library::{SCAN_LIMIT, TrackMeta, list_dir};

use crate::deck_ui::fmt_time;
use crate::playlist::DragItem;
use crate::i18n::{count_tracks, tr, trf};
use crate::theme;
use crate::fonts;

pub enum BrowserAction {
    Add { list: usize, paths: Vec<PathBuf> },
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
}

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
    current: Option<PathBuf>,
    /// Alle Titel im Ordner und seinen Unterordnern, kommen nach und nach aus der Hintergrundsuche.
    tracks: Vec<PathBuf>,
    scanning: bool,
    limited: bool,
    scan_token: u64,
    pending_scan: Option<PathBuf>,
    error: Option<String>,
    filter: String,
    /// Für diesen Suchtext wurden die Tags aller Titel schon angefordert.
    meta_requested_for: String,
    subdirs: HashMap<PathBuf, Vec<PathBuf>>,
}

impl Browser {
    /// `user_roots` sind gespeicherte, vom Nutzer hinzugefügte Ordner.
    pub fn new(start_dir: Option<PathBuf>, user_roots: &[PathBuf]) -> Self {
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
            current: None,
            tracks: Vec::new(),
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

    /// Öffnet einen Ordner. Die Titel samt Unterordnern sucht die Bibliothek im Hintergrund
    /// (siehe [`Self::take_pending_scan`]); sie erscheinen, sobald sie gefunden sind.
    pub fn open(&mut self, path: PathBuf) {
        self.tracks.clear();
        self.limited = false;
        match list_dir(&path) {
            Ok(l) => {
                self.subdirs.insert(path.clone(), l.dirs);
                self.error = None;
                self.scanning = true;
                self.pending_scan = Some(path.clone());
            }
            Err(e) => {
                self.scanning = false;
                self.error = Some(trf("{} lässt sich nicht lesen: {}", &[&path.display(), &e]));
            }
        }
        self.current = Some(path);
        self.filter.clear();
        self.meta_requested_for.clear();
    }

    /// Ordner, für den eine Suche gestartet werden soll.
    pub fn take_pending_scan(&mut self) -> Option<PathBuf> {
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

    /// Titel im geöffneten Ordner samt Unterordnern (soweit schon gefunden).
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
                if let Some(cur) = &self.current {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // Pfade im Home-Ordner kurz als „~/…“
                        let shown = dirs::home_dir()
                            .and_then(|h| cur.strip_prefix(h).ok().map(|r| format!("~/{}", r.display())))
                            .unwrap_or_else(|| cur.display().to_string());
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
        ui.horizontal_top(|ui| {
            // Baum
            ui.allocate_ui_with_layout(vec2(250.0, avail), egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).min_scrolled_height(0.0).show(ui, |ui| {
                    for root in self.roots.clone() {
                        if self.tree_node(ui, &root.name(), &root.path, 0, root.user) {
                            out.push(BrowserAction::RemoveRoot(root.path.clone()));
                        }
                    }
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(vec2(ui.available_width(), avail), egui::Layout::top_down(egui::Align::Min), |ui| {
                self.track_list(ui, ctx, out);
            });
        });

        if let Some(p) = ctx.preview {
            let bar = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - PREVIEW_BAR_H + 4.0), full.max);
            preview_bar(&mut bar_ui, bar, p, ctx, out);
        }
    }

    /// Zeichnet einen Ordner im Baum. Liefert `true`, wenn der Nutzer ihn aus der Liste entfernen will.
    fn tree_node(&mut self, ui: &mut egui::Ui, name: &str, path: &Path, depth: usize, removable: bool) -> bool {
        let id = ui.id().with(("tree", path));
        let selected = self.current.as_deref() == Some(path);
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth == 0 && selected);
        let mut open_this = false;
        let mut remove = false;
        state
            .show_header(ui, |ui| {
                let icon = if depth == 0 { "🗀" } else { "📁" };
                let text = RichText::new(format!("{icon} {name}")).color(if selected { theme::ink() } else { theme::dim() });
                let resp = ui.add(Button::selectable(selected, text).frame_when_inactive(false));
                if resp.clicked() {
                    open_this = true;
                }
                if removable {
                    resp.context_menu(|ui| {
                        if ui.button(tr("Aus der Liste entfernen")).clicked() {
                            remove = true;
                            ui.close();
                        }
                    });
                }
            })
            .body(|ui| {
                for child in self.children(path) {
                    let n = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.tree_node(ui, &n, &child, depth + 1, false);
                }
            });
        if open_this {
            self.open(path.to_path_buf());
        }
        remove
    }

    fn track_list(&mut self, ui: &mut egui::Ui, ctx: &BrowserCtx<'_>, out: &mut Vec<BrowserAction>) {
        if let Some(e) = &self.error {
            ui.colored_label(theme::cue(), e);
            return;
        }
        let Some(cur) = self.current.clone() else { return };
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
                let mut hay = p.strip_prefix(&cur).unwrap_or(p).to_string_lossy().to_lowercase();
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
        let has_subfolders = self.tracks.iter().any(|p| p.parent() != Some(cur.as_path()));

        ui.horizontal(|ui| {
            let folder = cur.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| cur.display().to_string());
            ui.label(RichText::new(folder).font(fonts::bold(17.0)));
            let total: f64 = rows.iter().filter_map(|p| ctx.metas.get(p)).map(|m| m.duration_secs).sum();
            let count = if words.is_empty() {
                count_tracks(rows.len(), &fmt_time(total))
            } else {
                trf("{} von {} Titeln", &[&rows.len(), &self.tracks.len()])
            };
            ui.label(RichText::new(count).color(theme::dim()));
            if self.scanning {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(tr("Durchsucht Unterordner …")).size(12.0).color(theme::dim()));
            } else if self.limited {
                ui.label(RichText::new(trf("Nur die ersten {} Titel. Für mehr einen Unterordner wählen.", &[&SCAN_LIMIT])).size(12.0).color(theme::yellow()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for list in [1usize, 0] {
                    let c = theme::deck_color(list);
                    let b = Button::new(RichText::new(trf("Ordner in Playlist {}", &[&theme::deck_letter(list)])).color(c).strong())
                        .fill(c.gamma_multiply(0.12))
                        .stroke(egui::Stroke::new(1.0, c.gamma_multiply(0.6)));
                    if ui.add_enabled(!rows.is_empty(), b).clicked() {
                        out.push(BrowserAction::Add { list, paths: rows.clone() });
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
                    row.set_selected(previewing);
                    row.col(|ui| {
                        if crate::hw::preview_button(ui, vec2(28.0, 20.0), previewing).clicked() {
                            out.push(if previewing { BrowserAction::StopPreview } else { BrowserAction::Preview(path.clone()) });
                        }
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(format!("{:02}", i + 1)).monospace().size(11.0).color(theme::faint()));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&m.title).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&m.artist).color(theme::dim())).truncate().selectable(false));
                    });
                    row.col(|ui| {
                        // Unterordner relativ zum geöffneten Ordner, leer für Titel direkt darin
                        let sub = path.parent().and_then(|d| d.strip_prefix(&cur).ok()).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.add(egui::Label::new(RichText::new(sub).size(12.0).color(theme::faint())).truncate().selectable(false));
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
                            if b.on_hover_text(trf("In Playlist {}", &[&letter])).clicked() {
                                out.push(BrowserAction::Add { list, paths: vec![path.clone()] });
                            }
                        }
                    });
                    let resp = row.response();
                    resp.dnd_set_drag_payload(DragItem::Files(vec![path.clone()]));
                });
            });
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
