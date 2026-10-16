// SPDX-License-Identifier: GPL-3.0-or-later
//! Ansicht einer Playlist mit Umsortieren per Drag & Drop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_library::TrackMeta;

use crate::deck_ui::fmt_time;
use crate::playlist::{DragItem, Playlist};
use crate::i18n::{count_tracks, tr, trf};
use crate::keynav::{self, KeyList};
use crate::theme;
use crate::{fonts, hw};

pub enum PlaylistAction {
    Load(usize),
    Remove(usize),
    Clear,
    Shuffle,
    Drop { item: DragItem, before: usize },
    NeedMeta(PathBuf),
    /// Titel auf dem Kopfhörer vorhören (läuft über den Vorhör-Player des Ordnerbrowsers).
    Preview(PathBuf),
    StopPreview,
    EditTags(PathBuf),
    /// Tempo neu ermitteln (auch mit BPM im Tag), wie „BPM analysieren“ im Ordnerbrowser.
    AnalyzeBpm(PathBuf),
    /// Titel direkt hinter den geladenen verschieben.
    PlayNext(usize),
    /// Titel ans Ende der anderen Playlist verschieben.
    MoveToOther(usize),
}

const ROW_H: f32 = 44.0;
const ROW_GAP: f32 = 2.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    list: usize,
    pl: &Playlist,
    deck_has_track: bool,
    // Spielt der Player dieser Playlist gerade? Dann lässt er sich nicht neu laden.
    deck_playing: bool,
    preview: Option<&Path>,
    metas: &HashMap<PathBuf, TrackMeta>,
    bpms: &HashMap<PathBuf, f32>,
    out: &mut Vec<PlaylistAction>,
) {
    let color = theme::deck_color(list);
    ui.horizontal(|ui| {
        ui.label(RichText::new(trf("PLAYLIST {}", &[&theme::deck_letter(list)])).font(fonts::bold(16.0)).color(color).extra_letter_spacing(1.2));
        // Gesamtdauer und fehlende Tags nur neu durchgehen, wenn sich Länge der Liste oder Tag-Bestand ändern
        // (lange Playlists kosten sonst in jedem Bild Zeit)
        let sum_id = egui::Id::new(("pl_total", list));
        let sig = (pl.entries.len(), metas.len());
        let total = match ui.data(|d| d.get_temp::<((usize, usize), f64)>(sum_id)) {
            Some((s, t)) if s == sig => t,
            _ => {
                let mut t = 0.0;
                for p in &pl.entries {
                    match metas.get(p) {
                        Some(m) => t += m.duration_secs,
                        None => out.push(PlaylistAction::NeedMeta(p.clone())),
                    }
                }
                ui.data_mut(|d| d.insert_temp(sum_id, (sig, t)));
                t
            }
        };
        ui.label(RichText::new(count_tracks(pl.entries.len(), &fmt_time(total))).color(theme::dim()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(!pl.entries.is_empty(), egui::Button::new(tr("Leeren"))).clicked() {
                out.push(PlaylistAction::Clear);
            }
            let shuffle = ui
                .add_enabled(pl.upcoming() > 1, egui::Button::new(tr("Zufällige Reihenfolge")))
                .on_hover_text(tr("Mischt die kommenden Titel. Der geladene und die schon gespielten bleiben an ihrem Platz."));
            if shuffle.clicked() {
                out.push(PlaylistAction::Shuffle);
            }
        });
    });
    ui.separator();

    let current = if deck_has_track { pl.current } else { None };
    let next = pl.current.map_or(0, |c| c + 1);

    crate::hw::calm_plate(ui.painter(), ui.available_rect_before_wrap());
    // Der geladene Titel steht oben im sichtbaren Bereich, die gespielten darüber erreicht man per Scrollen.
    // Gescrollt wird nur, wenn ein anderer Titel geladen wird; danach darf der Nutzer frei scrollen.
    let view_h = ui.available_height();
    let seen_id = egui::Id::new(("pl_current", list));
    let seen: Option<Option<usize>> = ui.data(|d| d.get_temp(seen_id));
    let scroll_to_current = seen != Some(pl.current);
    ui.data_mut(|d| d.insert_temp(seen_id, pl.current));
    let mut current_top: Option<f32> = None;
    // Markierter Titel für Pfeiltasten und Eingabetaste (nach Klick auf eine Zeile dieser Liste)
    let me = KeyList::Playlist(list);
    let cursor_id = egui::Id::new(("pl_cursor", list));
    let mut cursor: Option<usize> = ui.data(|d| d.get_temp::<usize>(cursor_id)).filter(|&c| c < pl.entries.len());
    let keys = keynav::take(ui.ctx(), me);
    let mut scroll_to_cursor = false;
    if let Some(k) = keys {
        if k.steps != 0 {
            // Ohne Markierung beginnt sie beim geladenen Titel
            cursor = keynav::step(cursor.or(pl.current), k.steps, pl.entries.len());
            scroll_to_cursor = true;
        }
        if k.enter
            && let Some(c) = cursor
        {
            out.push(PlaylistAction::Load(c));
        }
    }
    let show_cursor = keynav::focused(ui.ctx()) == Some(me);
    egui::ScrollArea::vertical().id_salt(("pl", list)).auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width();
        let mut insert_line: Option<f32> = None;

        // Zeilen dicht untereinander, nur ein schmaler Spalt zwischen den Hinterlegungen
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        // Nur die sichtbaren Zeilen bauen; die übrigen sind ein leerer Block gleicher Höhe
        let spacing = ui.spacing().item_spacing.y;
        let stride = ROW_H + spacing;
        let n = pl.entries.len();
        let top = ui.cursor().top();
        let clip = ui.clip_rect();
        let first = (((clip.top() - top) / stride).floor().max(0.0) as usize).min(n);
        let last = ((((clip.bottom() - top) / stride).ceil().max(0.0)) as usize + 1).min(n);
        if let Some(c) = pl.current.filter(|&c| c < n) {
            let rect = Rect::from_min_size(pos2(ui.cursor().left(), top + c as f32 * stride), vec2(width, ROW_H));
            current_top = Some(rect.top());
            if scroll_to_current {
                // egui lässt beim Scrollen einen Zeilenabstand frei; ohne Ausgleich lugt die vorige Zeile hervor
                ui.scroll_to_rect(rect.translate(vec2(0.0, spacing)), Some(egui::Align::TOP));
            }
        }
        if let Some(c) = cursor.filter(|_| scroll_to_cursor) {
            // Nur so weit, dass die markierte Zeile ganz zu sehen ist
            ui.scroll_to_rect(Rect::from_min_size(pos2(ui.cursor().left(), top + c as f32 * stride), vec2(width, ROW_H)), None);
        }
        if first > 0 {
            ui.allocate_space(vec2(width, first as f32 * stride - spacing));
        }
        for (i, path) in pl.entries.iter().enumerate().take(last).skip(first) {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click_and_drag());
            let fallback;
            let m = match metas.get(path) {
                Some(m) => m,
                None => {
                    fallback = TrackMeta::from_filename(path);
                    &fallback
                }
            };
            let is_now = current == Some(i);
            let done = pl.current.is_some_and(|c| i < c);
            let p = ui.painter();
            if is_now {
                p.rect_filled(rect, 4.0, color.gamma_multiply(0.16));
            } else if resp.hovered() {
                p.rect_filled(rect, 4.0, theme::row_hover());
            }
            let alpha = if done { 0.45 } else { 1.0 };
            let mark = if is_now { "▶".to_owned() } else if i == next { "›".to_owned() } else { format!("{:02}", i + 1) };
            let mark_color = if is_now { color } else if i == next { theme::ink() } else { theme::faint() };
            p.text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, mark, FontId::monospace(11.0), mark_color.gamma_multiply(alpha));
            let text_left = rect.left() + 38.0;
            let text_right = rect.right() - 140.0;
            let clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom()));
            let pc = p.with_clip_rect(clip);
            pc.text(pos2(text_left, rect.top() + 5.0), Align2::LEFT_TOP, &m.title, fonts::bold(14.0), theme::ink().gamma_multiply(alpha));
            pc.text(pos2(text_left, rect.bottom() - 5.0), Align2::LEFT_BOTTOM, &m.artist, fonts::list(12.0), theme::dim().gamma_multiply(alpha));
            let bpm = bpms.get(path).copied().or(m.bpm).map(|b| format!("{b:.0}")).unwrap_or_default();
            p.text(pos2(rect.right() - 60.0, rect.center().y), Align2::RIGHT_CENTER, bpm, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));
            let dur = if m.duration_secs > 0.0 { fmt_time(m.duration_secs) } else { String::new() };
            p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, dur, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));

            // Vorhören: Kopfhörertaste vor BPM und Länge, leuchtet, solange dieser Titel vorgehört wird
            let previewing = preview == Some(path.as_path());
            let head = Rect::from_center_size(pos2(rect.right() - 118.0, rect.center().y), vec2(30.0, 24.0));
            if ui.place(head, |ui: &mut egui::Ui| hw::preview_button(ui, head.size(), previewing)).clicked() {
                out.push(if previewing { PlaylistAction::StopPreview } else { PlaylistAction::Preview(path.clone()) });
            }

            resp.dnd_set_drag_payload(DragItem::Row { list, index: i });
            let upper = |r: &egui::Response| r.hover_pos().or_else(|| ui.ctx().pointer_hover_pos()).is_some_and(|pt| pt.y < rect.center().y);
            if resp.dnd_hover_payload::<DragItem>().is_some() {
                insert_line = Some(if upper(&resp) { rect.top() } else { rect.bottom() });
            }
            if let Some(item) = resp.dnd_release_payload::<DragItem>() {
                let before = if upper(&resp) { i } else { i + 1 };
                out.push(PlaylistAction::Drop { item: (*item).clone(), before });
            }
            if show_cursor && cursor == Some(i) {
                ui.painter().rect_stroke(rect.shrink(0.75), 4.0, Stroke::new(1.5, theme::select_accent()), egui::StrokeKind::Inside);
            }
            if resp.clicked() || resp.secondary_clicked() {
                cursor = Some(i);
                keynav::focus(ui.ctx(), me);
            }
            if resp.double_clicked() {
                out.push(PlaylistAction::Load(i));
            }
            resp.context_menu(|ui| {
                let load = ui.add_enabled(!deck_playing, egui::Button::new(trf("In Player {} laden", &[&theme::deck_letter(list)])));
                if load.on_disabled_hover_text(tr("Spielt gerade. Erst stoppen.")).clicked() {
                    out.push(PlaylistAction::Load(i));
                    ui.close();
                }
                // Schon geladen oder ohnehin als Nächstes dran: nichts zu verschieben
                let movable = pl.current != Some(i) && i != next;
                if ui.add_enabled(movable, egui::Button::new(tr("Als Nächstes spielen"))).clicked() {
                    out.push(PlaylistAction::PlayNext(i));
                    ui.close();
                }
                // Ein geladener Titel bleibt in seiner Playlist (wie beim Ziehen)
                let loaded = pl.current == Some(i) && deck_has_track;
                let other = theme::deck_letter(1 - list);
                let b = ui.add_enabled(!loaded, egui::Button::new(trf("In Playlist {} verschieben", &[&other])));
                if b.on_disabled_hover_text(trf("Der Titel ist in Player {} geladen und bleibt in dessen Playlist", &[&theme::deck_letter(list)])).clicked() {
                    out.push(PlaylistAction::MoveToOther(i));
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("Titelinfo & Tags …")).clicked() {
                    out.push(PlaylistAction::EditTags(path.clone()));
                    ui.close();
                }
                let hint = tr("Tempo im Hintergrund neu ermitteln, auch wenn eine BPM-Angabe im Tag steht. Der Wert wird gemerkt.");
                if ui.button(tr("BPM analysieren")).on_hover_text(hint).clicked() {
                    out.push(PlaylistAction::AnalyzeBpm(path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("Aus der Playlist entfernen")).clicked() {
                    out.push(PlaylistAction::Remove(i));
                    ui.close();
                }
            });
        }

        if last < n {
            ui.allocate_space(vec2(width, (n - last) as f32 * stride - spacing));
        }

        // Freie Fläche unter den Titeln: Ablegen ans Ende
        // So hoch, dass auch einer der letzten Titel ganz nach oben rücken kann
        let below_current = current_top.map_or(0.0, |top| top + view_h - ui.cursor().top());
        let rest = ui.available_height().max(below_current).max(ROW_H);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, rest), Sense::hover());
        if pl.entries.is_empty() {
            ui.painter().text(rect.center_top() + vec2(0.0, 30.0), Align2::CENTER_CENTER, tr("Leer. Titel aus dem Ordner hierher ziehen."), FontId::proportional(12.5), theme::faint());
        }
        if resp.dnd_hover_payload::<DragItem>().is_some() {
            insert_line = Some(rect.top());
        }
        if let Some(item) = resp.dnd_release_payload::<DragItem>() {
            out.push(PlaylistAction::Drop { item: (*item).clone(), before: pl.entries.len() });
        }

        if let Some(y) = insert_line {
            ui.painter().line_segment([pos2(ui.min_rect().left(), y), pos2(ui.min_rect().left() + width, y)], Stroke::new(2.0, color));
        }
    });
    match cursor {
        Some(c) => ui.data_mut(|d| {
            d.insert_temp(cursor_id, c);
        }),
        None => ui.data_mut(|d| d.remove::<usize>(cursor_id)),
    }
}
