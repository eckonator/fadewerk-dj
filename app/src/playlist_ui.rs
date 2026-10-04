// SPDX-License-Identifier: GPL-3.0-or-later
//! Ansicht einer Playlist mit Umsortieren per Drag & Drop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_library::sketch::{EDGE_RATE, EDGE_SECS};
use fw_library::{TrackMeta, WaveSketch};

use crate::fade_preview::{EditorAudio, Kind};
use crate::fadewerk::{Region, TrackFade, fmt_point, fmt_secs};

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
    /// Kleine Wellenform für die Zeile anfordern (FadeWerk-Modus).
    NeedSketch(PathBuf),
    /// FadeWerk-Modus ein- oder ausschalten (gilt für beide Playlists).
    FadeWerk(bool),
    /// Neue Fade-Bereiche für einen Eintrag.
    SetFade(usize, TrackFade),
    /// Fade-Bereiche aller Einträge in beiden Playlists entfernen.
    ResetAllFades,
    /// Im Fade-Editor vorhören: Bereich allein oder Übergang mit dem Nachbartitel.
    FadePreview(usize, Kind),
}

/// Höhe des aufgeklappten Fade-Editors unter einer Zeile.
const EDIT_H: f32 = 150.0;

/// Welche Zeile gerade den Fade-Editor offen hat (höchstens eine in beiden Playlists).
#[derive(Clone)]
struct OpenEditor {
    list: usize,
    index: usize,
    path: PathBuf,
}

fn editor_id() -> egui::Id {
    egui::Id::new("fw_fade_editor")
}

/// Liste und Eintrag mit offenem Fade-Editor.
pub fn open_editor(ctx: &egui::Context) -> Option<(usize, usize)> {
    ctx.data(|d| d.get_temp::<OpenEditor>(editor_id())).map(|o| (o.list, o.index))
}

/// Öffnet den Fade-Editor für einen Eintrag (nur für die Demo, `FW_FADEWERK=edit`).
pub fn demo_open_editor(ctx: &egui::Context, list: usize, index: usize, path: PathBuf) {
    ctx.data_mut(|d| d.insert_temp(editor_id(), OpenEditor { list, index, path }));
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
    // Im FadeWerk-Modus die Wellenformen der Titel, sonst `None`
    sketches: Option<&HashMap<PathBuf, Option<Arc<WaveSketch>>>>,
    // Nachbartitel und laufendes Vorhören für den offenen Fade-Editor
    editor_audio: &EditorAudio,
    out: &mut Vec<PlaylistAction>,
) {
    let color = theme::deck_color(list);
    // Markierter Titel für Pfeiltasten und Eingabetaste (nach Klick auf eine Zeile dieser Liste)
    let me = KeyList::Playlist(list);
    let cursor_id = egui::Id::new(("pl_cursor", list));
    let mut cursor: Option<usize> = ui.data(|d| d.get_temp::<usize>(cursor_id)).filter(|&c| c < pl.entries.len());
    let keys = keynav::take(ui.ctx(), me);
    // Ausrichtung beim Scrollen zur Markierung: Pfeiltasten nur bis sichtbar, Suchtreffer in die Mitte
    let mut scroll_to_cursor: Option<Option<egui::Align>> = None;
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
        ui.add_space(4.0);
        // Ein Schalter für beide Playlists: beide Tasten leuchten gemeinsam
        let on = sketches.is_some();
        let tip = tr("FadeWerk: zeigt die Wellenform jedes Titels. Hier legst du fest, wie jeder Titel ein- und ausgeblendet wird.");
        if hw::button(ui, vec2(84.0, 22.0), "FADEWERK", fonts::bold(11.0), hw::Look::Color(theme::yellow()), on, true).on_hover_text(tip).clicked() {
            out.push(PlaylistAction::FadeWerk(!on));
        }
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
            // Schnellsuche: springt zum ersten Treffer ab dem geladenen Titel, Enter zum nächsten, ⇧Enter zurück
            if let Some(hit) = quick_search(ui, list, pl, metas, cursor) {
                cursor = Some(hit);
                scroll_to_cursor = Some(Some(egui::Align::Center));
                keynav::focus(ui.ctx(), me);
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
    if let Some(k) = keys {
        if k.steps != 0 {
            // Ohne Markierung beginnt sie beim geladenen Titel
            cursor = keynav::step(cursor.or(pl.current), k.steps, pl.entries.len());
            scroll_to_cursor = Some(None);
        }
        if k.enter
            && let Some(c) = cursor
        {
            out.push(PlaylistAction::Load(c));
        }
    }
    let show_cursor = keynav::focused(ui.ctx()) == Some(me);
    // Offener Fade-Editor dieser Liste. Wandert der Titel (Umsortieren), folgt der Editor ihm.
    let mut open: Option<OpenEditor> = ui.data(|d| d.get_temp(editor_id()));
    if sketches.is_none() {
        open = None;
    }
    let mut edit: Option<usize> = None;
    if let Some(o) = open.as_mut().filter(|o| o.list == list) {
        if pl.entries.get(o.index) != Some(&o.path) {
            match pl.entries.iter().position(|p| *p == o.path) {
                Some(i) => o.index = i,
                None => open = None,
            }
        }
        edit = open.as_ref().map(|o| o.index);
    }
    // Esc schließt den Editor (nicht, wenn gerade ein Textfeld tippt)
    if edit.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input() {
        open = None;
        edit = None;
    }
    let mut close_editor = false;
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
        // Unter der Zeile mit offenem Editor rücken alle folgenden um dessen Höhe nach unten
        let extra = EDIT_H + spacing;
        let y_of = |i: usize| top + i as f32 * stride + if edit.is_some_and(|e| i > e) { extra } else { 0.0 };
        let row_at = |y: f32| ((y - top) / stride).max(0.0);
        let first = match edit {
            Some(e) if row_at(clip.top()).floor() as usize > e => (row_at(clip.top() - extra).floor() as usize).max(e),
            _ => row_at(clip.top()).floor() as usize,
        }
        .min(n);
        let last = match edit {
            Some(e) if (row_at(clip.bottom()).ceil() as usize + 1) > e => (row_at(clip.bottom() - extra).ceil() as usize + 1).max(e + 1),
            _ => row_at(clip.bottom()).ceil() as usize + 1,
        }
        .min(n);
        if let Some(c) = pl.current.filter(|&c| c < n) {
            let rect = Rect::from_min_size(pos2(ui.cursor().left(), y_of(c)), vec2(width, ROW_H));
            current_top = Some(rect.top());
            if scroll_to_current {
                // egui lässt beim Scrollen einen Zeilenabstand frei; ohne Ausgleich lugt die vorige Zeile hervor
                ui.scroll_to_rect(rect.translate(vec2(0.0, spacing)), Some(egui::Align::TOP));
            }
        }
        if let (Some(c), Some(align)) = (cursor, scroll_to_cursor) {
            ui.scroll_to_rect(Rect::from_min_size(pos2(ui.cursor().left(), y_of(c)), vec2(width, ROW_H)), align);
        }
        if first > 0 {
            ui.allocate_space(vec2(width, y_of(first) - top - spacing));
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
            if let Some(sketches) = sketches {
                match sketches.get(path) {
                    Some(Some(sk)) => row_wave(p, rect, sk, pl.fade(i), if is_now { color } else { color.gamma_multiply(0.7 * alpha) }),
                    Some(None) => {}
                    None => {
                        // Wird noch berechnet: feine Mittellinie als Platzhalter
                        let (l, r) = wave_span(rect);
                        p.line_segment([pos2(l, rect.center().y), pos2(r, rect.center().y)], Stroke::new(1.0, theme::faint().gamma_multiply(0.5)));
                        out.push(PlaylistAction::NeedSketch(path.clone()));
                    }
                }
            }
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
            // Im FadeWerk-Modus öffnet ein Klick den Fade-Editor dieser Zeile (ein anderer schließt dabei)
            if sketches.is_some() && resp.clicked() && edit != Some(i) {
                // Erst im nächsten Bild aufklappen, die Zeilen dieses Bilds liegen schon fest
                open = Some(OpenEditor { list, index: i, path: path.clone() });
                ui.ctx().request_repaint();
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
                if sketches.is_some() || pl.has_fades() {
                    ui.separator();
                    let fade = pl.fade(i);
                    if ui.add_enabled(!fade.is_empty(), egui::Button::new(tr("Fade-Bereiche dieses Titels entfernen"))).clicked() {
                        out.push(PlaylistAction::SetFade(i, TrackFade::default()));
                        ui.close();
                    }
                    // Rückfrage im Untermenü: das betrifft alle Titel
                    ui.menu_button(tr("Alle Fade-Bereiche entfernen …"), |ui| {
                        ui.label(RichText::new(tr("Ein- und Ausblendbereiche aller Titel in beiden Playlists entfernen?")).color(theme::yellow()));
                        if ui.button(tr("Ja, alle entfernen")).clicked() {
                            out.push(PlaylistAction::ResetAllFades);
                            ui.close();
                        }
                    });
                }
                ui.separator();
                if ui.button(tr("Aus der Playlist entfernen")).clicked() {
                    out.push(PlaylistAction::Remove(i));
                    ui.close();
                }
            });

            if edit == Some(i) {
                let duration = match sketches.and_then(|s| s.get(path)) {
                    Some(Some(sk)) if sk.duration_secs > 0.0 => sk.duration_secs,
                    _ => m.duration_secs as f32,
                };
                let sk = sketches.and_then(|s| s.get(path)).and_then(|s| s.as_deref());
                let (erect, eresp) = ui.allocate_exact_size(vec2(width, EDIT_H), Sense::click());
                // Zeile und Editor gehören sichtbar zusammen
                ui.painter().rect_stroke(rect.union(erect).expand(1.0), 5.0, Stroke::new(1.5, theme::yellow().gamma_multiply(0.8)), egui::StrokeKind::Inside);
                if fade_editor(ui, (list, i), erect, sk, duration, pl.fade(i), color, editor_audio, out) || eresp.secondary_clicked() {
                    close_editor = true;
                }
            }
        }

        if last < n {
            let end = top + n as f32 * stride + if edit.is_some() { extra } else { 0.0 };
            ui.allocate_space(vec2(width, end - y_of(last) - spacing));
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
    if close_editor {
        open = None;
    }
    // Nur die Liste mit dem Editor (oder die, in der er gerade aufging) schreibt den Zustand zurück,
    // sonst würde die andere Liste ihn im selben Bild wieder überschreiben
    let before: Option<OpenEditor> = ui.data(|d| d.get_temp(editor_id()));
    let mine = open.as_ref().is_some_and(|o| o.list == list) || before.as_ref().is_some_and(|o| o.list == list) || sketches.is_none();
    if mine {
        match open {
            Some(o) => ui.data_mut(|d| {
                d.insert_temp(editor_id(), o);
            }),
            None => ui.data_mut(|d| {
                d.remove::<OpenEditor>(editor_id());
            }),
        }
    }
}

/// Welcher Teil eines Bereichs gerade gezogen wird.
#[derive(Clone, Copy)]
enum Grab {
    Start,
    End,
    /// Neuer Bereich ab dieser Stelle (Sekunden).
    New(f32),
}

/// Aufgeklappter Fade-Editor unter einer Zeile: links die ersten, rechts die letzten Sekunden des Titels,
/// vergrößert. Liefert `true`, wenn er geschlossen werden soll.
#[allow(clippy::too_many_arguments)]
fn fade_editor(
    ui: &mut egui::Ui,
    key: (usize, usize),
    rect: Rect,
    sk: Option<&WaveSketch>,
    duration: f32,
    fade: TrackFade,
    color: egui::Color32,
    audio: &EditorAudio,
    out: &mut Vec<PlaylistAction>,
) -> bool {
    let p = ui.painter().clone();
    p.rect_filled(rect, 4.0, theme::yellow().gamma_multiply(0.05));
    let inner = rect.shrink2(vec2(10.0, 6.0));
    let gap = 14.0;
    let half = (inner.width() - gap) / 2.0;
    let wells_top = inner.top() + 20.0;
    // Unter den Ausschnitten die Vorhör-Tasten, darunter die Fußzeile
    let wells_bottom = inner.bottom() - 52.0;
    let left = Rect::from_min_max(pos2(inner.left(), wells_top), pos2(inner.left() + half, wells_bottom));
    let right = Rect::from_min_max(pos2(inner.right() - half, wells_top), pos2(inner.right(), wells_bottom));
    let mut changed = fade;

    match sk.filter(|_| duration > 0.0) {
        Some(sk) => {
            let span = EDGE_SECS.min(duration);
            let id = egui::Id::new(("fw_fade_well", key));
            // Abspielstrich beim Vorhören im passenden Ausschnitt
            let head = |is_head: bool| audio.playing.and_then(|(k, t)| t.filter(|_| k.is_head() == is_head));
            if let Some(r) = edge_well(ui, id.with("in"), left, (0.0, span), &sk.head, 0.0, fade.fade_in, true, color, head(true)) {
                changed.fade_in = r;
            }
            let tail_t0 = duration - sk.tail.len() as f32 / EDGE_RATE;
            if let Some(r) = edge_well(ui, id.with("out"), right, (duration - span, duration), &sk.tail, tail_t0, fade.fade_out, false, color, head(false)) {
                changed.fade_out = r;
            }
        }
        None => {
            for r in [left, right] {
                hw::well(&p, r, 3.0);
            }
            p.text(left.center(), Align2::CENTER_CENTER, tr("Wellenform wird berechnet …"), FontId::proportional(12.0), theme::well_ink().gamma_multiply(0.6));
        }
    }
    if changed != fade {
        out.push(PlaylistAction::SetFade(key.1, changed));
    }

    // Überschriften mit den Werten
    let label = |x: f32, right: f32, head: &str, text: String| {
        let p = p.with_clip_rect(Rect::from_min_max(pos2(x, inner.top() - 2.0), pos2(right, inner.top() + 16.0)));
        let g = p.layout_no_wrap(head.to_owned(), fonts::bold(11.5), theme::yellow());
        let w = g.size().x;
        p.galley(pos2(x, inner.top()), g, theme::yellow());
        p.text(pos2(x + w + 8.0, inner.top() + 1.0), Align2::LEFT_TOP, text, FontId::proportional(11.5), theme::dim());
    };
    let fade_in_text = match changed.fade_in {
        None => tr("nicht gesetzt (halbe Überblendzeit)").to_owned(),
        Some(r) if r.len() < 0.05 => trf("harter Einstieg bei {}", &[&fmt_point(r.start)]),
        Some(r) => trf("{} bis {} · {} s", &[&fmt_point(r.start), &fmt_point(r.end), &fmt_secs(r.len())]),
    };
    let fade_out_text = match changed.fade_out {
        None => tr("nicht gesetzt (halbe Überblendzeit)").to_owned(),
        Some(r) if r.len() < 0.05 => trf("harter Schluss bei {}", &[&fmt_point(r.end)]),
        Some(r) => trf("{} bis {} · {} s", &[&fmt_point(r.start), &fmt_point(r.end), &fmt_secs(r.len())]),
    };
    label(left.left(), left.right(), tr("EINBLENDEN"), fade_in_text);
    label(right.left(), right.right(), tr("AUSBLENDEN"), fade_out_text);
    preview_row(ui, left, Kind::In, Kind::From, &audio.partners[0], key.1, audio, out);
    preview_row(ui, right, Kind::Out, Kind::To, &audio.partners[1], key.1, audio, out);

    // Fußzeile: Bedienhinweis und Tasten
    let foot_y = inner.bottom() - 10.0;
    let done = Rect::from_center_size(pos2(inner.right() - 34.0, foot_y), vec2(68.0, 20.0));
    let reset = Rect::from_center_size(pos2(done.left() - 8.0 - 56.0, foot_y), vec2(112.0, 20.0));
    let hint = tr("Ziehen = Bereich · Klick = harter Schnitt · Rechtsklick = löschen");
    p.with_clip_rect(Rect::from_min_max(pos2(inner.left(), foot_y - 10.0), pos2(reset.left() - 8.0, foot_y + 10.0)))
        .text(pos2(inner.left(), foot_y), Align2::LEFT_CENTER, hint, FontId::proportional(11.0), theme::faint());
    let reset_resp = ui.place(reset, |ui: &mut egui::Ui| hw::button(ui, reset.size(), tr("ZURÜCKSETZEN"), fonts::bold(10.0), hw::Look::Plain, false, !changed.is_empty()));
    if reset_resp.clicked() {
        out.push(PlaylistAction::SetFade(key.1, TrackFade::default()));
    }
    ui.place(done, |ui: &mut egui::Ui| hw::button(ui, done.size(), tr("FERTIG"), fonts::bold(10.0), hw::Look::Color(theme::yellow()), false, true))
        .on_hover_text(tr("Schließt den Editor (auch mit Esc oder Rechtsklick neben die Ausschnitte)"))
        .clicked()
}

/// Vorhör-Tasten unter einem Ausschnitt: der Bereich allein und der Übergang mit dem Nachbartitel.
#[allow(clippy::too_many_arguments)]
fn preview_row(
    ui: &mut egui::Ui,
    well: Rect,
    alone: Kind,
    mix: Kind,
    partner: &Option<crate::fade_preview::Partner>,
    index: usize,
    audio: &EditorAudio,
    out: &mut Vec<PlaylistAction>,
) {
    let y = well.bottom() + 13.0;
    let font = fonts::bold(10.0);
    let playing = |k: Kind| audio.playing.is_some_and(|(p, _)| p == k);
    let mut x = well.left();
    let mut place = |w: f32, label: &str, kind: Kind, enabled: bool, tip: String| {
        let rect = Rect::from_min_size(pos2(x, y - 10.0), vec2(w, 20.0));
        x += w + 6.0;
        let on = playing(kind);
        let text = format!("{} {label}", if on { "⏹" } else { "▶" });
        let look = if on { hw::Look::Color(theme::cue()) } else { hw::Look::Plain };
        let r = ui.place(rect, |ui: &mut egui::Ui| hw::button(ui, rect.size(), &text, font.clone(), look, on, enabled));
        if r.on_hover_text(tip).clicked() {
            out.push(if on { PlaylistAction::StopPreview } else { PlaylistAction::FadePreview(index, kind) });
        }
    };
    let tip = if alone == Kind::In { tr("Einblenden allein auf dem Kopfhörer vorhören") } else { tr("Ausblenden allein auf dem Kopfhörer vorhören") };
    place(78.0, tr("BEREICH"), alone, true, tip.to_owned());
    let tip = match (partner, mix) {
        (Some(pt), Kind::From) => trf("Übergang von „{}“ (Playlist {}) auf dem Kopfhörer vorhören", &[&pt.name, &theme::deck_letter(pt.list)]),
        (Some(pt), _) => trf("Übergang zu „{}“ (Playlist {}) auf dem Kopfhörer vorhören", &[&pt.name, &theme::deck_letter(pt.list)]),
        (None, Kind::From) => tr("Davor kommt kein Titel aus der anderen Playlist").to_owned(),
        (None, _) => tr("Danach kommt kein Titel aus der anderen Playlist").to_owned(),
    };
    place(96.0, tr("ÜBERGANG"), mix, partner.is_some(), tip);
    // Mit wem der Übergang klingt
    if let Some(pt) = partner {
        let text = if mix == Kind::From { trf("von „{}“ ({})", &[&pt.name, &theme::deck_letter(pt.list)]) } else { trf("zu „{}“ ({})", &[&pt.name, &theme::deck_letter(pt.list)]) };
        let p = ui.painter().with_clip_rect(Rect::from_min_max(pos2(x, y - 10.0), pos2(well.right(), y + 10.0)));
        p.text(pos2(x, y), Align2::LEFT_CENTER, text, FontId::proportional(11.0), theme::dim());
    }
}

/// Vergrößerter Ausschnitt am Anfang (`fade_in`) oder Ende eines Titels mit dem Bereich darin.
/// `window` ist der gezeigte Zeitraum, `peaks` beginnen bei `peaks_t0` mit [`EDGE_RATE`] Werten pro Sekunde.
/// Liefert den geänderten Bereich (`Some(None)` = gelöscht).
#[allow(clippy::too_many_arguments)]
fn edge_well(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    window: (f32, f32),
    peaks: &[u8],
    peaks_t0: f32,
    region: Option<Region>,
    fade_in: bool,
    color: egui::Color32,
    // Stelle des Vorhörens im Titel (Sekunden)
    playhead: Option<f32>,
) -> Option<Option<Region>> {
    let (t0, t1) = window;
    let span = (t1 - t0).max(0.01);
    let to_x = |t: f32| rect.left() + (t - t0) / span * rect.width();
    let to_t = |x: f32| (t0 + (x - rect.left()) / rect.width() * span).clamp(t0, t1);
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let p = ui.painter_at(rect);
    hw::well(&p, rect, 3.0);

    // Sekundenraster: feine Striche alle 5 s, Beschriftung alle 10 s
    let mut t = (t0 / 5.0).ceil() * 5.0;
    while t < t1 {
        let x = to_x(t);
        let major = (t / 10.0).round() * 10.0 == t;
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.top() + if major { 6.0 } else { 3.0 })], Stroke::new(1.0, theme::well_ink().gamma_multiply(0.25)));
        if major && x > rect.left() + 12.0 && x < rect.right() - 12.0 {
            p.text(pos2(x, rect.bottom() - 2.0), Align2::CENTER_BOTTOM, crate::deck_ui::fmt_time(t as f64), FontId::monospace(9.0), theme::well_ink().gamma_multiply(0.35));
        }
        t += 5.0;
    }

    // Wellenform
    let inset = rect.shrink2(vec2(1.0, 4.0));
    let cols = (inset.width() / 2.0).max(1.0) as usize;
    let col_w = inset.width() / cols as f32;
    let mut mesh = egui::Mesh::default();
    for i in 0..cols {
        let a = t0 + i as f32 / cols as f32 * span;
        let b = a + span / cols as f32;
        let ia = (((a - peaks_t0) * EDGE_RATE).floor().max(0.0) as usize).min(peaks.len());
        let ib = (((b - peaks_t0) * EDGE_RATE).ceil().max(0.0) as usize).clamp(ia, peaks.len());
        let v = peaks[ia..ib].iter().copied().max().unwrap_or(0) as f32 / 255.0;
        let h = (v * inset.height() * 0.46).max(0.5);
        let x = inset.left() + i as f32 * col_w;
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, inset.center().y - h), pos2(x + col_w, inset.center().y + h)), color.gamma_multiply(0.8));
    }
    p.add(egui::Shape::mesh(mesh));

    // Bereich: abgeschnittener Teil abgedunkelt, Rampe als Linie
    let accent = theme::yellow();
    if let Some(r) = region {
        let (xs, xe) = (to_x(r.start), to_x(r.end));
        let cut = if fade_in { Rect::from_min_max(rect.min, pos2(xs, rect.bottom())) } else { Rect::from_min_max(pos2(xe, rect.top()), rect.max) };
        p.rect_filled(cut, 0.0, egui::Color32::from_black_alpha(150));
        if xe - xs >= 1.0 {
            p.rect_filled(Rect::from_min_max(pos2(xs, rect.top()), pos2(xe, rect.bottom())), 0.0, accent.gamma_multiply(0.14));
            let (low, high) = (rect.bottom() - 3.0, rect.top() + 3.0);
            let ramp = if fade_in { [pos2(xs, low), pos2(xe, high)] } else { [pos2(xs, high), pos2(xe, low)] };
            p.line_segment(ramp, Stroke::new(2.0, accent));
            for x in [xs, xe] {
                p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, accent.gamma_multiply(0.7)));
            }
        } else {
            p.line_segment([pos2(xs, rect.top()), pos2(xs, rect.bottom())], Stroke::new(2.0, accent));
        }
    }

    if let Some(t) = playhead.filter(|t| (t0..=t1).contains(t)) {
        let x = to_x(t);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, theme::cue()));
    }

    // Maus: Linie mit Zeitangabe, über den Kanten ein Verschiebe-Zeiger
    const EDGE_PX: f32 = 6.0;
    let near = |x: f32, t: f32| (x - to_x(t)).abs() <= EDGE_PX;
    let ptr = resp.hover_pos().or(resp.interact_pointer_pos());
    if let Some(pos) = ptr {
        let on_edge = region.is_some_and(|r| r.len() >= 0.05 && (near(pos.x, r.start) || near(pos.x, r.end)));
        ui.ctx().set_cursor_icon(if on_edge { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::Crosshair });
        let x = pos.x.clamp(rect.left(), rect.right());
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, egui::Color32::from_white_alpha(120)));
        let (align, tx) = if x > rect.right() - 50.0 { (Align2::RIGHT_TOP, x - 4.0) } else { (Align2::LEFT_TOP, x + 4.0) };
        p.text(pos2(tx, rect.top() + 2.0), align, fmt_point(to_t(x)), FontId::monospace(10.0), theme::well_ink());
    }

    // Nahe am Rand rastet die Zeit am Rand ein (Titelanfang bzw. -ende)
    let snap = |x: f32| {
        if x <= rect.left() + EDGE_PX {
            t0
        } else if x >= rect.right() - EDGE_PX {
            t1
        } else {
            to_t(x)
        }
    };
    let grab_id = id.with("grab");
    if resp.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        let grab = match region {
            Some(r) if r.len() >= 0.05 && near(origin.x, r.start) => Grab::Start,
            Some(r) if r.len() >= 0.05 && near(origin.x, r.end) => Grab::End,
            _ => Grab::New(snap(origin.x)),
        };
        ui.data_mut(|d| d.insert_temp(grab_id, grab));
    }
    let mut result = None;
    if resp.dragged()
        && let (Some(pos), Some(grab)) = (resp.interact_pointer_pos(), ui.data(|d| d.get_temp::<Grab>(grab_id)))
    {
        let t = snap(pos.x);
        let r = region.unwrap_or(Region { start: t, end: t });
        let new = match grab {
            Grab::New(a) => Region::between(a, t),
            Grab::Start => Region::between(t, r.end),
            Grab::End => Region::between(r.start, t),
        };
        if Some(new) != region {
            result = Some(Some(new));
        }
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<Grab>(grab_id));
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let t = snap(pos.x);
        result = Some(Some(Region { start: t, end: t }));
    }
    if resp.secondary_clicked() && region.is_some() {
        result = Some(None);
    }
    result
}

/// Waagerechter Bereich der Wellenform in einer Zeile: hinter Titel und Interpret, bis vor die Kopfhörertaste.
fn wave_span(rect: Rect) -> (f32, f32) {
    (rect.left() + 32.0, rect.right() - 136.0)
}

/// Wellenform eines Titels als Hintergrund der Zeile (wie die Übersicht im Player), dezent unter dem Text.
fn row_wave(p: &egui::Painter, rect: Rect, sk: &WaveSketch, fade: TrackFade, color: egui::Color32) {
    let (l, r) = wave_span(rect);
    if r <= l || sk.overview.is_empty() {
        return;
    }
    let cols = ((r - l) / 2.0).max(1.0) as usize;
    let col_w = (r - l) / cols as f32;
    let mid = rect.center().y;
    let half = rect.height() * 0.42;
    let fill = color.gamma_multiply(0.22);
    let mut mesh = egui::Mesh::default();
    for i in 0..cols {
        let f = i as f32 / cols as f32;
        let v = (sk.overview_max(f, f + 1.0 / cols as f32) * half).max(0.5);
        let x = l + i as f32 * col_w;
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, mid - v), pos2(x + col_w, mid + v)), fill);
    }
    p.add(egui::Shape::mesh(mesh));
    // Gesetzte Bereiche: abgeschnittene Teile dunkler, Rampen als feine Linien
    let dur = sk.duration_secs;
    if fade.is_empty() || dur <= 0.0 {
        return;
    }
    let to_x = |t: f32| l + (t / dur).clamp(0.0, 1.0) * (r - l);
    let (top, bottom) = (mid - half, mid + half);
    let shade = egui::Color32::from_black_alpha(90);
    let stroke = Stroke::new(1.5, theme::yellow().gamma_multiply(0.85));
    if let Some(f) = fade.fade_in {
        let (xs, xe) = (to_x(f.start), to_x(f.end));
        p.rect_filled(Rect::from_min_max(pos2(l, top), pos2(xs, bottom)), 0.0, shade);
        p.line_segment([pos2(xs, bottom), pos2(xe.max(xs + 1.0), top)], stroke);
    }
    if let Some(f) = fade.fade_out {
        let (xs, xe) = (to_x(f.start), to_x(f.end));
        p.rect_filled(Rect::from_min_max(pos2(xe, top), pos2(r, bottom)), 0.0, shade);
        p.line_segment([pos2(xs.min(xe - 1.0), top), pos2(xe, bottom)], stroke);
    }
}

/// Schnellsuche in der Kopfzeile einer Playlist. Liefert den Titel, zu dem gesprungen werden soll: beim Tippen
/// der erste Treffer ab dem geladenen Titel (die kommenden zuerst), mit Enter der nächste, mit ⇧Enter der vorige.
/// Esc leert das Feld. Gesucht wird mit allen Wörtern in Titel, Interpret, Album und Dateiname.
fn quick_search(ui: &mut egui::Ui, list: usize, pl: &Playlist, metas: &HashMap<PathBuf, TrackMeta>, cursor: Option<usize>) -> Option<usize> {
    let id = egui::Id::new(("pl_search", list));
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // Trefferzahl der letzten Suche, als (Stelle, Anzahl)
    let hits_id = id.with("hits");
    let w = (ui.available_width() * 0.45).clamp(80.0, 200.0);
    let resp = ui.add(egui::TextEdit::singleline(&mut query).hint_text(tr("Suchen")).desired_width(w));
    let (enter, shift, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.modifiers.shift, i.key_pressed(egui::Key::Escape)));
    let mut jump = None;
    if resp.lost_focus() && esc {
        query.clear();
    } else if resp.changed() || (resp.lost_focus() && enter) {
        let next = !resp.changed();
        if next {
            // Feld behalten, damit man weiter Enter drücken oder nachtippen kann
            resp.request_focus();
        }
        let words: Vec<String> = query.split_whitespace().map(crate::browser::fold).collect();
        let hits: Vec<usize> = if words.is_empty() {
            Vec::new()
        } else {
            pl.entries
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    let file = p.file_name().unwrap_or_default().to_string_lossy();
                    let hay = match metas.get(*p) {
                        Some(m) => crate::browser::fold(&format!("{} {} {} {}", m.title, m.artist, m.album, file)),
                        None => crate::browser::fold(&file),
                    };
                    words.iter().all(|w| hay.contains(w.as_str()))
                })
                .map(|(i, _)| i)
                .collect()
        };
        let from = if next { cursor } else { pl.current };
        jump = match (next, from) {
            _ if hits.is_empty() => None,
            (true, Some(c)) if shift => Some(hits.iter().rev().find(|&&h| h < c).copied().unwrap_or(hits[hits.len() - 1])),
            (true, Some(c)) => Some(hits.iter().find(|&&h| h > c).copied().unwrap_or(hits[0])),
            (false, Some(c)) => Some(hits.iter().find(|&&h| h >= c).copied().unwrap_or(hits[0])),
            (_, None) => Some(hits[0]),
        };
        let pos = jump.and_then(|j| hits.iter().position(|&h| h == j)).map_or(0, |p| p + 1);
        ui.data_mut(|d| d.insert_temp(hits_id, (pos, hits.len())));
    }
    if query.trim().is_empty() {
        ui.data_mut(|d| d.remove::<(usize, usize)>(hits_id));
    } else if let Some((pos, n)) = ui.data(|d| d.get_temp::<(usize, usize)>(hits_id)) {
        let text = if n == 0 { tr("kein Treffer").to_owned() } else { format!("{pos}/{n}") };
        ui.label(RichText::new(text).size(11.5).color(if n == 0 { theme::cue() } else { theme::dim() }));
    }
    resp.on_hover_text(tr("Springt zum passenden Titel. Enter: nächster Treffer, ⇧Enter: voriger, Esc: leeren"));
    ui.data_mut(|d| d.insert_temp(id, query));
    jump
}
