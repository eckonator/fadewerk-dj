// SPDX-License-Identifier: GPL-3.0-or-later
//! Ansicht einer Playlist mit Umsortieren per Drag & Drop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_library::TrackMeta;

use crate::deck_ui::fmt_time;
use crate::playlist::{DragItem, Playlist};
use crate::i18n::{count_tracks, tr, trf};
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
}

const ROW_H: f32 = 48.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    list: usize,
    pl: &Playlist,
    deck_has_track: bool,
    preview: Option<&Path>,
    metas: &HashMap<PathBuf, TrackMeta>,
    bpms: &HashMap<PathBuf, f32>,
    out: &mut Vec<PlaylistAction>,
) {
    let color = theme::deck_color(list);
    ui.horizontal(|ui| {
        ui.label(RichText::new(trf("PLAYLIST {}", &[&theme::deck_letter(list)])).font(fonts::bold(16.0)).color(color).extra_letter_spacing(1.2));
        let total: f64 = pl.entries.iter().filter_map(|p| metas.get(p)).map(|m| m.duration_secs).sum();
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
    egui::ScrollArea::vertical().id_salt(("pl", list)).auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width();
        let mut insert_line: Option<f32> = None;

        for (i, path) in pl.entries.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click_and_drag());
            let meta = metas.get(path);
            if meta.is_none() {
                out.push(PlaylistAction::NeedMeta(path.clone()));
            }
            let fallback = TrackMeta::from_filename(path);
            let m = meta.unwrap_or(&fallback);

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
            pc.text(pos2(text_left, rect.top() + 7.0), Align2::LEFT_TOP, &m.title, fonts::bold(14.0), theme::ink().gamma_multiply(alpha));
            pc.text(pos2(text_left, rect.bottom() - 7.0), Align2::LEFT_BOTTOM, &m.artist, fonts::list(12.0), theme::dim().gamma_multiply(alpha));
            let bpm = bpms.get(path).copied().or(m.bpm).map(|b| format!("{b:.0}")).unwrap_or_default();
            p.text(pos2(rect.right() - 60.0, rect.center().y), Align2::RIGHT_CENTER, bpm, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));
            let dur = if m.duration_secs > 0.0 { fmt_time(m.duration_secs) } else { String::new() };
            p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, dur, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));

            // Vorhören: Kopfhörertaste vor BPM und Länge, leuchtet, solange dieser Titel vorgehört wird
            let previewing = preview == Some(path.as_path());
            let head = Rect::from_center_size(pos2(rect.right() - 118.0, rect.center().y), vec2(30.0, 24.0));
            if ui.put(head, |ui: &mut egui::Ui| hw::preview_button(ui, head.size(), previewing)).clicked() {
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
            if resp.double_clicked() {
                out.push(PlaylistAction::Load(i));
            }
            resp.context_menu(|ui| {
                if ui.button(tr("Titelinfo & Tags …")).clicked() {
                    out.push(PlaylistAction::EditTags(path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button(tr("Aus der Playlist entfernen")).clicked() {
                    out.push(PlaylistAction::Remove(i));
                    ui.close();
                }
            });
        }

        // Freie Fläche unter den Titeln: Ablegen ans Ende
        let rest = ui.available_height().max(ROW_H);
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
}
