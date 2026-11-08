// SPDX-License-Identifier: GPL-3.0-or-later
//! Ansicht einer Playlist mit Umsortieren per Drag & Drop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align2, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_library::sketch::{EDGE_RATE, EDGE_SECS};
use fw_library::{Camelot, TrackMeta, WaveSketch};

use crate::fade_preview::{EditorAudio, Kind};
use crate::fadewerk::{Region, TrackFade, fmt_point, fmt_secs};

use crate::deck_ui::fmt_time;
use crate::playlist::{DragItem, Playlist};
use crate::i18n::{count_tracks, tr, trf};
use crate::crossdrag;
use crate::keynav::{self, KeyList};
use crate::selection::Selection;
use crate::theme;
use crate::{fonts, hw};

pub enum PlaylistAction {
    Load(usize),
    /// Laden aus dem Kontextmenü: der Titel rückt vorher direkt hinter die gespielten.
    LoadBehindPlayed(usize),
    Remove(usize),
    /// Markierte Titel entfernen (der geladene bleibt).
    RemoveMany(Vec<usize>),
    Clear,
    /// Beide Playlists und beide Player leeren (fragt vorher nach).
    ClearAll,
    /// Weitere Exemplare desselben Titels entfernen (der geladene bleibt).
    RemoveDuplicates,
    /// FadeWerk-Bereiche vorschlagen: für einen Eintrag (überschreibt) bzw. für alle ohne eigene Bereiche.
    SuggestFade(usize),
    SuggestFades,
    /// Fenster „Playlist prüfen“ öffnen.
    Check,
    Shuffle,
    /// Kommende Titel beider Playlists harmonisch ordnen (siehe `harmonic.rs`).
    Harmonic,
    /// Die letzte Sortierung (harmonisch oder zufällig) zurücknehmen.
    SortUndo,
    Drop { item: DragItem, before: usize },
    NeedMeta(PathBuf),
    /// Tags einer sichtbaren Zeile, vor allen anderen.
    ShowMeta(PathBuf),
    /// Titel auf dem Kopfhörer vorhören (läuft über den Vorhör-Player des Ordnerbrowsers).
    Preview(PathBuf),
    StopPreview,
    EditTags(PathBuf),
    /// Tempo und Tonart neu ermitteln (auch mit Angaben im Tag), wie im Ordnerbrowser.
    Analyze(PathBuf),
    /// Titel bewerten (0 = Bewertung entfernen).
    Rate { paths: Vec<PathBuf>, stars: u8 },
    /// Titel direkt hinter den geladenen verschieben.
    PlayNext(usize),
    /// Titel ans Ende der anderen Playlist verschieben.
    MoveToOther(usize),
    /// Markierte Titel ans Ende der anderen Playlist verschieben.
    MoveManyToOther(Vec<usize>),
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
    /// Gerade aufgeklappt: einmal so scrollen, dass Zeile und Editor ganz zu sehen sind.
    reveal: bool,
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
    ctx.data_mut(|d| d.insert_temp(editor_id(), OpenEditor { list, index, path, reveal: false }));
}

/// Markierte Zeilen einer Playlist als (Stelle, Titel), damit sie ihrem Eintrag folgen, wenn sich die Liste ändert.
type Marked = Selection<(usize, PathBuf)>;

const ROW_H: f32 = 44.0;
/// Zeilenhöhe der kompakten Ansicht (Titel und Interpret in einer Zeile).
const ROW_H_COMPACT: f32 = 30.0;
/// Höhe von Suchfeld, „Sortieren“ und „Leeren“ in der Kopfzeile.
const HEADER_CTRL_H: f32 = theme::CTRL_H;
const ROW_GAP: f32 = 2.0;

/// Angaben zu den Titeln der Zeilen: Tags, ermittelte BPM und Tonarten. `reference` = klingende Tonart und
/// Pfad des Titels auf Sendung; dazu passende Titel bekommen einen Rahmen um ihre Tonart.
pub struct TrackInfo<'a> {
    pub metas: &'a HashMap<PathBuf, TrackMeta>,
    pub bpms: &'a HashMap<PathBuf, f32>,
    pub keys: &'a HashMap<PathBuf, Option<Camelot>>,
    pub ratings: &'a crate::ratings::Ratings,
    /// Beatraster (für den Fade-Editor: Beat-Linien, Einrasten).
    pub grids: &'a HashMap<PathBuf, Option<fw_library::BeatGrid>>,
    pub reference: Option<(Camelot, &'a Path)>,
    /// Zustand des Auswahlfelds „Sortieren“ (gilt für beide Playlists).
    pub harmonic: Harmonic,
    /// Kompakte Ansicht: einzeilige, flachere Zeilen (Titel · Interpret).
    pub compact: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Harmonic {
    Ready,
    /// Fehlende Tonarten werden ermittelt, danach wird sortiert.
    Analyzing,
    /// Gerade sortiert: „Rückgängig“ steht im Auswahlfeld.
    Undo,
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    list: usize,
    pl: &Playlist,
    deck_has_track: bool,
    // Spielt der Player dieser Playlist gerade? Dann lässt er sich nicht neu laden.
    deck_playing: bool,
    preview: Option<&Path>,
    key_info: &TrackInfo<'_>,
    // Im FadeWerk-Modus die Wellenformen der Titel, sonst `None`
    sketches: Option<&HashMap<PathBuf, Option<Arc<WaveSketch>>>>,
    // Nachbartitel und laufendes Vorhören für den offenen Fade-Editor
    editor_audio: &EditorAudio,
    out: &mut Vec<PlaylistAction>,
) {
    let (metas, bpms) = (key_info.metas, key_info.bpms);
    let color = theme::deck_color(list);
    // Markierter Titel für Pfeiltasten und Eingabetaste (nach Klick auf eine Zeile dieser Liste)
    let me = KeyList::Playlist(list);
    let cursor_id = egui::Id::new(("pl_cursor", list));
    // Gemerkt mit dem Titel: wurde er verschoben (Ziehen, „Als Nächstes spielen“), folgt die Markierung ihm
    let mut cursor: Option<usize> = ui.data(|d| d.get_temp::<(usize, PathBuf)>(cursor_id)).and_then(|(c, path)| {
        if pl.entries.get(c) == Some(&path) { Some(c) } else { pl.entries.iter().position(|p| *p == path) }
    });
    let keys = keynav::take(ui.ctx(), me);
    let sel_id = egui::Id::new(("pl_sel", list));
    let mut sel: Marked = ui.data(|d| d.get_temp(sel_id)).unwrap_or_default();
    // Hat sich die Liste darunter geändert (Ziehen, Entfernen, Sortieren), folgt die Markierung ihren Titeln
    if sel.items().any(|(i, p)| pl.entries.get(*i) != Some(p)) {
        sel.remap(|(i, p)| match pl.entries.get(*i) == Some(p) {
            true => Some((*i, p.clone())),
            false => pl.entries.iter().position(|e| e == p).map(|j| (j, p.clone())),
        });
    }
    // Treffer der Schnellsuche, die markiert werden sollen („Alle markieren“)
    let mut mark_hits: Option<Vec<usize>> = None;
    // Ausrichtung beim Scrollen zur Markierung: Pfeiltasten nur bis sichtbar, Suchtreffer in die Mitte
    let mut scroll_to_cursor: Option<Option<egui::Align>> = None;
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
    let title = trf("PLAYLIST {}", &[&theme::deck_letter(list)]);
    let count = count_tracks(pl.entries.len(), &fmt_time(total));
    // Passt alles in eine Zeile? Sonst wird die Suche zur Lupe, die das Feld erst auf Klick zeigt
    // (vorher schob sich bei schmalen Fenstern „Sortieren“ über die FADEWERK-Taste)
    let gap = ui.spacing().item_spacing.x;
    let text_w = |t: &str, font: egui::FontId| ui.painter().layout_no_wrap(t.to_owned(), font, egui::Color32::WHITE).size().x;
    let body = egui::TextStyle::Body.resolve(ui.style());
    let left_w = text_w(&title, fonts::bold(16.0)) + 1.2 * title.chars().count() as f32 + text_w(&count, body) + FADEWERK_W + 3.0 * gap + 4.0;
    // Sortieren und Leeren stecken im ⋯-Menü (eine Symboltaste statt Auswahlfeld und zerstörerischer Taste)
    let menu_w = theme::ICON_BTN_W;
    let tools_w = menu_w + SEARCH_MIN_W + 2.0 * gap;
    let avail_w = ui.available_width();
    let narrow = avail_w < left_w + tools_w;
    // Platz für Suchfeld, Trefferzahl und „Alle markieren“ zwischen FADEWERK und „Sortieren“
    let search_room = avail_w - left_w - (tools_w - SEARCH_MIN_W);
    // Ohne Feld: passt es mit Lupe statt Feld, sonst fällt zusätzlich die Titelzahl weg (steht dann im Tooltip)
    let lupe_w = theme::ICON_BTN_W;
    let show_count = !narrow || avail_w >= left_w + menu_w + lupe_w + 3.0 * gap;
    let open_id = egui::Id::new(("pl_search_open", list));
    let field_id = egui::Id::new(("pl_search_field", list));
    let query_active = ui.data(|d| d.get_temp::<String>(egui::Id::new(("pl_search", list)))).is_some_and(|q| !q.trim().is_empty());
    // Beim Suchen braucht der Kopf Platz für Trefferzahl und „Alle markieren“; fehlt er, bekommt die Suche
    // die ganze Zeile wie nach Klick auf die Lupe
    let (hits_w, mark_w) = hits_widths(ui);
    let cramped = query_active && search_room < SEARCH_MIN_SEARCHING_W + hits_w + mark_w;
    let search_open = (narrow && ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false)) || ((narrow || cramped) && query_active);

    // Ein Schalter für beide Playlists: beide Tasten leuchten gemeinsam
    let fadewerk_on = sketches.is_some();
    let mut head_out: Vec<PlaylistAction> = Vec::new();
    // Titel, Anzahl und FADEWERK-Taste; liefert, ob die Taste geklickt wurde
    let left = |ui: &mut egui::Ui| {
        let t = ui.label(RichText::new(&title).font(fonts::bold(16.0)).color(color).extra_letter_spacing(1.2));
        if show_count {
            ui.label(RichText::new(&count).color(theme::dim()));
        } else {
            t.on_hover_text(&count);
        }
        ui.add_space(4.0);
        let tip = tr("FadeWerk: zeigt die Wellenform jedes Titels. Hier legst du fest, wie jeder Titel ein- und ausgeblendet wird.");
        hw::button(ui, vec2(FADEWERK_W, HEADER_CTRL_H), "FADEWERK", fonts::bold(11.0), hw::Look::Color(theme::yellow()), fadewerk_on, true).on_hover_text(tip).clicked()
    };
    // Sortieren und Schnellsuche, von rechts nach links; in der zweiten Zeile nimmt die Suche den ganzen Rest
    // `search`: Feld zeigen (`Some(fill)`, mit `fill` über den ganzen Rest) oder gar nicht (`None`)
    let mut tools = |ui: &mut egui::Ui, sort: bool, search: Option<(bool, bool)>| {
        if !sort {
            if let Some((fill, focus)) = search {
                let (jump, mark) = quick_search(ui, list, pl, metas, cursor, fill, focus, f32::INFINITY);
                if let Some(hit) = jump {
                    cursor = Some(hit);
                    scroll_to_cursor = Some(Some(egui::Align::Center));
                    keynav::focus(ui.ctx(), me);
                }
                mark_hits = mark.or(mark_hits.take());
            }
            return;
        }
        // ⋯-Menü: Sortieren (harmonisch, zufällig, rückgängig) und, abgesetzt, Leeren. „Rückgängig“ nimmt die
        // letzte Sortierung zurück (gilt für beide Playlists)
        let analyzing = key_info.harmonic == Harmonic::Analyzing;
        let more = more_button(ui, analyzing);
        let more = if analyzing { more.on_hover_text(tr("Fehlende Tonarten werden ermittelt, danach wird sortiert.")) } else { more.on_hover_text(tr("Sortieren, Leeren …")) };
        egui::Popup::menu(&more).show(|ui| {
            let harmonic = ui
                .add_enabled(!analyzing && pl.upcoming() > 0, egui::Button::new(tr("Harmonisch sortieren (Tonart, Tempo)")))
                .on_hover_text(tr("Ordnet die kommenden Titel beider Playlists so, dass jeder Übergang im Wechsel A – B – A passt: zuerst nach Tonart, dann nach Tempo. Fehlende Tonarten werden vorher ermittelt."));
            if harmonic.clicked() {
                out.push(PlaylistAction::Harmonic);
            }
            let shuffle = ui
                .add_enabled(!analyzing && pl.upcoming() > 1, egui::Button::new(tr("Zufällige Reihenfolge")))
                .on_hover_text(tr("Mischt die kommenden Titel. Der geladene und die schon gespielten bleiben an ihrem Platz."));
            if shuffle.clicked() {
                out.push(PlaylistAction::Shuffle);
            }
            if key_info.harmonic == Harmonic::Undo {
                let undo = ui
                    .add(egui::Button::new(tr("Sortieren rückgängig")))
                    .on_hover_text(tr("Stellt die Reihenfolge von vor dem letzten Sortieren wieder her."));
                if undo.clicked() {
                    out.push(PlaylistAction::SortUndo);
                }
            }
            let dups = ui
                .add_enabled(pl.entries.len() > 1, egui::Button::new(tr("Dubletten entfernen")))
                .on_hover_text(tr("Lässt von mehreren Exemplaren desselben Titels (gleicher Interpret und Titel, fast gleiche Länge) nur eines stehen. Der geladene Titel bleibt."));
            if dups.clicked() {
                out.push(PlaylistAction::RemoveDuplicates);
            }
            let suggest = ui
                .add_enabled(pl.entries.iter().enumerate().any(|(i, _)| pl.fade(i).is_empty()), egui::Button::new(tr("Fades vorschlagen")))
                .on_hover_text(tr("Setzt für alle Titel ohne eigene Bereiche Ein- und Ausblendbereiche nach Beatraster und Lautstärke: Einstieg auf dem ersten Takt nach der Stille, Ausblenden vor dem Ausklang, so lang wie die Überblendzeit."));
            if suggest.clicked() {
                out.push(PlaylistAction::SuggestFades);
            }
            let check = ui
                .add_enabled(!pl.entries.is_empty(), egui::Button::new(tr("Playlist prüfen …")))
                .on_hover_text(tr("Vor dem Auftritt: findet fehlende, verschobene, nicht abspielbare und noch nicht analysierte Titel."));
            if check.clicked() {
                out.push(PlaylistAction::Check);
            }
            ui.separator();
            // Abgesetzt und rot: leert die Liste, ein laufender Titel bleibt
            if ui.add_enabled(!pl.entries.is_empty(), egui::Button::new(RichText::new(tr("Playlist leeren")).color(theme::cue()))).clicked() {
                out.push(PlaylistAction::Clear);
            }
            if ui.add(egui::Button::new(RichText::new(tr("Alles leeren (A und B) …")).color(theme::cue()))).on_hover_text(tr("Leert beide Playlists und beide Player, für einen neuen Abend")).clicked() {
                out.push(PlaylistAction::ClearAll);
            }
        });
        // Schnellsuche: zeigt nur die Treffer und markiert den ersten ab dem geladenen Titel, Enter den nächsten, ⇧Enter den vorigen
        if let Some((fill, focus)) = search {
            let (jump, mark) = quick_search(ui, list, pl, metas, cursor, fill, focus, search_room);
            if let Some(hit) = jump {
                cursor = Some(hit);
                scroll_to_cursor = Some(Some(egui::Align::Center));
                keynav::focus(ui.ctx(), me);
            }
            mark_hits = mark.or(mark_hits.take());
        }
    };
    let right_to_left = egui::Layout::right_to_left(egui::Align::Center);
    if search_open {
        // Suche offen: Titel, Feld über die ganze Breite und Schließen; die übrigen Tasten kommen danach wieder
        let just_opened = ui.data(|d| d.get_temp::<bool>(open_id.with("focus"))).unwrap_or(false);
        ui.data_mut(|d| d.remove::<bool>(open_id.with("focus")));
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&title).font(fonts::bold(16.0)).color(color).extra_letter_spacing(1.2)).on_hover_text(&count);
            ui.with_layout(right_to_left, |ui| {
                ui.spacing_mut().interact_size.y = HEADER_CTRL_H;
                if close_button(ui).on_hover_text(tr("Suche schließen")).clicked() {
                    close = true;
                }
                tools(ui, false, Some((true, just_opened)));
            });
        });
        let focused = ui.memory(|m| m.has_focus(field_id));
        let query_empty = ui.data(|d| d.get_temp::<String>(egui::Id::new(("pl_search", list)))).is_none_or(|q| q.trim().is_empty());
        // Leeres Feld ohne Fokus (weggeklickt, Esc) schließt sich wieder
        if close || (query_empty && !focused && !just_opened) {
            ui.data_mut(|d| {
                d.insert_temp(open_id, false);
                d.insert_temp(egui::Id::new(("pl_search", list)), String::new());
                d.remove::<(usize, usize, usize)>(egui::Id::new(("pl_search", list)).with("hits"));
            });
        }
    } else if narrow {
        ui.horizontal(|ui| {
            if left(ui) {
                head_out.push(PlaylistAction::FadeWerk(!fadewerk_on));
            }
            ui.with_layout(right_to_left, |ui| {
                ui.spacing_mut().interact_size.y = HEADER_CTRL_H;
                tools(ui, true, None);
                if lupe_button(ui).on_hover_text(tr("In der Playlist suchen")).clicked() {
                    ui.data_mut(|d| {
                        d.insert_temp(open_id, true);
                        d.insert_temp(open_id.with("focus"), true);
                    });
                }
            });
        });
    } else {
        ui.horizontal(|ui| {
            if left(ui) {
                head_out.push(PlaylistAction::FadeWerk(!fadewerk_on));
            }
            ui.with_layout(right_to_left, |ui| {
                // Menü und Suchen gleich hoch, damit sie auf einer Linie sitzen
                ui.spacing_mut().interact_size.y = HEADER_CTRL_H;
                tools(ui, true, Some((false, false)));
            });
        });
    }
    out.extend(head_out);
    ui.separator();

    // Mit Suchtext zeigt die Liste nur die Treffer (Stellen in `pl.entries`, aufsteigend); sonst alle Titel
    let query: String = ui.data(|d| d.get_temp(egui::Id::new(("pl_search", list)))).unwrap_or_default();
    let shown: Option<Vec<usize>> = (!query.trim().is_empty()).then(|| filtered(ui, list, pl, metas, &query));
    let n_rows = shown.as_ref().map_or(pl.entries.len(), Vec::len);
    // Zeile k der Ansicht → Eintrag, Eintrag → Zeile (falls gezeigt)
    let entry_at = |k: usize| shown.as_ref().map_or(k, |s| s[k]);
    let row_of = |i: usize| match &shown {
        Some(s) => s.binary_search(&i).ok(),
        None => (i < pl.entries.len()).then_some(i),
    };
    let all_rows = || (0..n_rows).map(|k| (entry_at(k), pl.entries[entry_at(k)].clone())).collect::<Vec<_>>();

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
    if let Some(hits) = mark_hits {
        sel.clear();
        sel.select_all(&hits.iter().filter_map(|&i| Some((i, pl.entries.get(i)?.clone()))).collect::<Vec<_>>());
        keynav::focus(ui.ctx(), me);
    }
    // ⌘A markiert alle Titel, Esc hebt die Markierung auf, Entf/⌫ entfernt die markierten
    // (nach einem Klick in diese Liste, nicht beim Tippen)
    if keynav::focused(ui.ctx()) == Some(me) && !ui.ctx().egui_wants_keyboard_input() && !egui::Popup::is_any_open(ui.ctx()) {
        let (all, esc, del) = ui.input_mut(|i| {
            let all = i.consume_key(egui::Modifiers::COMMAND, egui::Key::A);
            let del = i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) | i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
            (all, i.key_pressed(egui::Key::Escape), del)
        });
        if all {
            sel.select_all(&all_rows());
        }
        if esc {
            sel.clear();
        }
        if del && !sel.is_empty() {
            out.push(PlaylistAction::RemoveMany(marked_indices(&sel)));
            sel.clear();
        }
    }
    if let Some(k) = keys {
        if k.steps != 0 {
            // Ohne Markierung beginnt sie beim geladenen Titel
            cursor = keynav::step(cursor.or(pl.current).and_then(row_of), k.steps, n_rows).map(entry_at);
            scroll_to_cursor = Some(None);
            // Mit ⇧ wird der Bereich ab dem zuletzt geklickten Titel markiert
            if let Some(c) = cursor {
                let row = (c, pl.entries[c].clone());
                if k.extend { sel.click(&all_rows(), &row, false, true) } else { sel.select_only(&row) }
            }
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
        // Titel aus dem ausgeklinkten Ordnerfenster, nur über dem sichtbaren Teil der Liste
        let foreign = crossdrag::hover(ui.ctx()).filter(|&pt| ui.clip_rect().contains(pt));

        // Zeilen dicht untereinander, nur ein schmaler Spalt zwischen den Hinterlegungen
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        // Nur die sichtbaren Zeilen bauen; die übrigen sind ein leerer Block gleicher Höhe
        let spacing = ui.spacing().item_spacing.y;
        let row_h = if key_info.compact { ROW_H_COMPACT } else { ROW_H };
        let stride = row_h + spacing;
        let n = n_rows;
        // Zeile mit offenem Editor (ist ihr Titel weggefiltert, steht der Editor nicht in der Liste)
        let edit_row = edit.and_then(row_of);
        let top = ui.cursor().top();
        let clip = ui.clip_rect();
        // Unter der Zeile mit offenem Editor rücken alle folgenden um dessen Höhe nach unten
        let extra = EDIT_H + spacing;
        let y_of = |k: usize| top + k as f32 * stride + if edit_row.is_some_and(|e| k > e) { extra } else { 0.0 };
        let row_at = |y: f32| ((y - top) / stride).max(0.0);
        let first = match edit_row {
            Some(e) if row_at(clip.top()).floor() as usize > e => (row_at(clip.top() - extra).floor() as usize).max(e),
            _ => row_at(clip.top()).floor() as usize,
        }
        .min(n);
        let last = match edit_row {
            Some(e) if (row_at(clip.bottom()).ceil() as usize + 1) > e => (row_at(clip.bottom() - extra).ceil() as usize + 1).max(e + 1),
            _ => row_at(clip.bottom()).ceil() as usize + 1,
        }
        .min(n);
        if let Some(c) = pl.current.and_then(row_of) {
            let rect = Rect::from_min_size(pos2(ui.cursor().left(), y_of(c)), vec2(width, row_h));
            current_top = Some(rect.top());
            if scroll_to_current {
                // egui lässt beim Scrollen einen Zeilenabstand frei; ohne Ausgleich lugt die vorige Zeile hervor
                ui.scroll_to_rect(rect.translate(vec2(0.0, spacing)), Some(egui::Align::TOP));
            }
        }
        if let (Some(c), Some(align)) = (cursor.and_then(row_of), scroll_to_cursor) {
            ui.scroll_to_rect(Rect::from_min_size(pos2(ui.cursor().left(), y_of(c)), vec2(width, row_h)), align);
        }
        if first > 0 {
            ui.allocate_space(vec2(width, y_of(first) - top - spacing));
        }
        for i in (first..last).map(entry_at) {
            let path = &pl.entries[i];
            let (rect, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::click_and_drag());
            let fallback;
            let m = match metas.get(path) {
                Some(m) => m,
                None => {
                    out.push(PlaylistAction::ShowMeta(path.clone()));
                    fallback = TrackMeta::from_filename(path);
                    &fallback
                }
            };
            let is_now = current == Some(i);
            let done = pl.current.is_some_and(|c| i < c);
            let marked = !sel.is_empty() && sel.contains(&(i, path.clone()));
            let p = ui.painter();
            if is_now {
                p.rect_filled(rect, 4.0, color.gamma_multiply(0.16));
            }
            if marked {
                p.rect_filled(rect, 4.0, theme::row_selected());
            } else if resp.hovered() && !is_now {
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
            let fade = pl.fade(i);
            // Eigene Fades: zwei Dreiecke vor der Kopfhörertaste, dafür endet der Text früher
            let text_right = rect.right() - if fade.is_empty() { 222.0 } else { 256.0 };
            let clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom()));
            let pc = p.with_clip_rect(clip);
            if key_info.compact {
                // Einzeilig: Titel, dahinter schwächer der Interpret
                let mut job = egui::text::LayoutJob::default();
                job.append(&m.title, 0.0, egui::TextFormat::simple(fonts::bold(13.0), theme::ink().gamma_multiply(alpha)));
                if !m.artist.is_empty() {
                    job.append(&format!(" · {}", m.artist), 0.0, egui::TextFormat::simple(fonts::list(11.5), theme::dim().gamma_multiply(alpha)));
                }
                job.wrap = egui::text::TextWrapping { max_width: (text_right - text_left).max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let galley = pc.layout_job(job);
                pc.galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, theme::ink());
            } else {
                pc.text(pos2(text_left, rect.top() + 5.0), Align2::LEFT_TOP, &m.title, fonts::bold(14.0), theme::ink().gamma_multiply(alpha));
                pc.text(pos2(text_left, rect.bottom() - 5.0), Align2::LEFT_BOTTOM, &m.artist, fonts::list(12.0), theme::dim().gamma_multiply(alpha));
            }
            // Tonart als Plakette in der Farbe des Camelot-Rades; passt sie zum Titel auf Sendung, grün umrandet
            if let Some(key) = crate::browser::key_of(key_info.metas, key_info.keys, path) {
                let pill = Rect::from_center_size(pos2(rect.right() - 104.0, rect.center().y), vec2(32.0, 18.0));
                let kc = theme::key_color(key);
                p.rect_filled(pill, 4.0, kc.gamma_multiply(0.18 * alpha));
                if key_info.reference.is_some_and(|(rk, rp)| rp != path.as_path() && rk.compatible(key)) {
                    p.rect_stroke(pill, 4.0, Stroke::new(1.5, theme::key_match().gamma_multiply(alpha)), egui::StrokeKind::Inside);
                }
                p.text(pill.center(), Align2::CENTER_CENTER, key.label(), fonts::bold(11.5), kc.gamma_multiply(alpha));
            }
            let bpm = bpms.get(path).copied().or(m.bpm).map(|b| format!("{b:.0}")).unwrap_or_default();
            p.text(pos2(rect.right() - 60.0, rect.center().y), Align2::RIGHT_CENTER, bpm, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));
            let dur = if m.duration_secs > 0.0 { fmt_time(m.duration_secs) } else { String::new() };
            p.text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, dur, FontId::monospace(12.0), theme::dim().gamma_multiply(alpha));
            // Bewertung links neben der Tonart; leere Sterne nur bei Maus darüber (Klick bewertet wie im Browser)
            let hot = ui.rect_contains_pointer(rect);
            let stars_rect = Rect::from_min_size(pos2(rect.right() - 176.0, rect.center().y - 7.0), vec2(52.0, 14.0));
            let stars = key_info.ratings.stars(path, Some(m));
            if let Some(stars) = hw::star_rating(ui, egui::Id::new(("pl_stars", list, i)), stars_rect, stars, true, hot) {
                let paths = if marked && sel.len() > 1 { marked_indices(&sel).iter().filter_map(|&k| pl.entries.get(k).cloned()).collect() } else { vec![path.clone()] };
                out.push(PlaylistAction::Rate { paths, stars });
            }

            if !fade.is_empty() {
                let mark = Rect::from_center_size(pos2(rect.right() - 239.0, rect.center().y), vec2(24.0, 11.0));
                let tip = fade_mark(ui.painter(), mark, fade, alpha);
                // Tooltip über die Zeile: ein eigenes Element darüber bekäme unter der anklickbaren Zeile kein Hover
                if resp.hover_pos().is_some_and(|pt| mark.expand(4.0).contains(pt)) {
                    resp.clone().on_hover_text_at_pointer(tip);
                }
            }

            // Vorhören: Kopfhörertaste vor BPM und Länge, leuchtet, solange dieser Titel vorgehört wird. Nur bei
            // Bedarf sichtbar (Maus über der Zeile, geladener oder markierter Titel), das nimmt Unruhe aus der Liste
            let previewing = preview == Some(path.as_path());
            let head = Rect::from_center_size(pos2(rect.right() - 200.0, rect.center().y), vec2(30.0, 24.0));
            let wanted = previewing || current == Some(i) || cursor == Some(i) || ui.rect_contains_pointer(rect);
            if wanted && ui.place(head, |ui: &mut egui::Ui| hw::preview_button(ui, head.size(), previewing)).clicked() {
                out.push(if previewing { PlaylistAction::StopPreview } else { PlaylistAction::Preview(path.clone()) });
            }

            // Gehört die Zeile zu mehreren markierten, wandern alle mit
            if resp.drag_started() {
                let item = if marked && sel.len() > 1 { DragItem::Rows { list, indices: marked_indices(&sel) } } else { DragItem::Row { list, index: i } };
                egui::DragAndDrop::set_payload(ui.ctx(), item);
            }
            // Zeiger beim Ziehen: eigener oder einer aus dem ausgeklinkten Ordnerfenster
            let foreign = foreign.filter(|&pt| rect.contains(pt));
            let upper = |r: &egui::Response| foreign.or_else(|| r.hover_pos()).or_else(|| ui.ctx().pointer_hover_pos()).is_some_and(|pt| pt.y < rect.center().y);
            if resp.dnd_hover_payload::<DragItem>().is_some() || foreign.is_some() {
                insert_line = Some(if upper(&resp) { rect.top() } else { rect.bottom() });
            }
            let dropped = resp.dnd_release_payload::<DragItem>().map(|item| (*item).clone()).or_else(|| foreign.and_then(|_| crossdrag::take_drop(ui.ctx(), rect)));
            if let Some(item) = dropped {
                let before = if upper(&resp) { i } else { i + 1 };
                out.push(PlaylistAction::Drop { item, before });
            }
            if show_cursor && cursor == Some(i) {
                ui.painter().rect_stroke(rect.shrink(0.75), 4.0, Stroke::new(1.5, theme::select_accent()), egui::StrokeKind::Inside);
            }
            let mods = ui.input(|i| i.modifiers);
            if resp.clicked() {
                sel.click(&all_rows(), &(i, path.clone()), mods.command, mods.shift);
            }
            if resp.secondary_clicked() && !marked {
                sel.select_only(&(i, path.clone()));
            }
            if resp.clicked() || resp.secondary_clicked() {
                cursor = Some(i);
                keynav::focus(ui.ctx(), me);
            }
            // Im FadeWerk-Modus öffnet ein Klick den Fade-Editor dieser Zeile (ein anderer schließt dabei),
            // nicht beim Markieren mit ⌘ oder ⇧
            if sketches.is_some() && resp.clicked() && !mods.command && !mods.shift && edit != Some(i) {
                // Erst im nächsten Bild aufklappen, die Zeilen dieses Bilds liegen schon fest
                open = Some(OpenEditor { list, index: i, path: path.clone(), reveal: true });
                ui.ctx().request_repaint();
            }
            if resp.double_clicked() {
                out.push(PlaylistAction::Load(i));
            }
            // Rechtsklick in eine Markierung mehrerer Titel: Verschieben und Entfernen gelten für alle
            let several = (marked && sel.len() > 1).then(|| marked_indices(&sel));
            resp.context_menu(|ui| {
                let load = ui.add_enabled(!deck_playing, egui::Button::new(trf("In Player {} laden", &[&theme::deck_letter(list)])));
                if load.on_disabled_hover_text(tr("Spielt gerade. Erst stoppen.")).clicked() {
                    out.push(PlaylistAction::LoadBehindPlayed(i));
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
                if let Some(several) = &several {
                    if ui.button(trf("{} Titel in Playlist {} verschieben", &[&several.len(), &other])).clicked() {
                        out.push(PlaylistAction::MoveManyToOther(several.clone()));
                        ui.close();
                    }
                } else {
                    let b = ui.add_enabled(!loaded, egui::Button::new(trf("In Playlist {} verschieben", &[&other])));
                    if b.on_disabled_hover_text(trf("Der Titel ist in Player {} geladen und bleibt in dessen Playlist", &[&theme::deck_letter(list)])).clicked() {
                        out.push(PlaylistAction::MoveToOther(i));
                        ui.close();
                    }
                }
                ui.separator();
                let paths: Vec<PathBuf> = match &several {
                    Some(s) => s.iter().filter_map(|&k| pl.entries.get(k).cloned()).collect(),
                    None => vec![path.clone()],
                };
                let current = several.is_none().then(|| key_info.ratings.stars(path, metas.get(path)));
                if let Some(stars) = crate::browser::rating_menu(ui, current) {
                    out.push(PlaylistAction::Rate { paths, stars });
                    ui.close();
                }
                if ui.button(tr("Titelinfo & Tags …")).clicked() {
                    out.push(PlaylistAction::EditTags(path.clone()));
                    ui.close();
                }
                let hint = tr(crate::browser::ANALYZE_HINT);
                if ui.button(tr("Analysieren (BPM, Tonart)")).on_hover_text(hint).clicked() {
                    out.push(PlaylistAction::Analyze(path.clone()));
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
                if let Some(several) = &several {
                    if ui.button(trf("{} Titel aus der Playlist entfernen", &[&several.len()])).on_hover_text(tr("Entf")).clicked() {
                        out.push(PlaylistAction::RemoveMany(several.clone()));
                        ui.close();
                    }
                } else if ui.button(tr("Aus der Playlist entfernen")).clicked() {
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
                let grid = key_info.grids.get(path).copied().flatten();
                if fade_editor(ui, (list, i), erect, sk, grid, duration, pl.fade(i), color, editor_audio, out) || eresp.secondary_clicked() {
                    close_editor = true;
                }
                // Unten in der Liste aufgeklappt: nur so weit scrollen, bis auch die Fußzeile im Bild ist
                if let Some(o) = open.as_mut().filter(|o| o.reveal) {
                    o.reveal = false;
                    ui.scroll_to_rect(rect.union(erect).translate(vec2(0.0, spacing)), None);
                }
            }
        }

        if last < n {
            let end = top + n as f32 * stride + if edit_row.is_some() { extra } else { 0.0 };
            ui.allocate_space(vec2(width, end - y_of(last) - spacing));
        }

        // Freie Fläche unter den Titeln: Ablegen ans Ende
        // So hoch, dass auch einer der letzten Titel ganz nach oben rücken kann
        let below_current = current_top.map_or(0.0, |top| top + view_h - ui.cursor().top());
        let rest = ui.available_height().max(below_current).max(row_h);
        let (rect, resp) = ui.allocate_exact_size(vec2(width, rest), Sense::hover());
        if pl.entries.is_empty() {
            ui.painter().text(rect.center_top() + vec2(0.0, 30.0), Align2::CENTER_CENTER, tr("Leer. Titel aus dem Ordner hierher ziehen."), FontId::proportional(12.5), theme::faint());
        } else if n == 0 {
            ui.painter().text(rect.center_top() + vec2(0.0, 30.0), Align2::CENTER_CENTER, tr("Kein Titel passt zur Suche."), FontId::proportional(12.5), theme::faint());
        }
        let foreign_here = foreign.is_some_and(|pt| rect.contains(pt));
        if resp.dnd_hover_payload::<DragItem>().is_some() || foreign_here {
            insert_line = Some(rect.top());
        }
        let dropped = resp.dnd_release_payload::<DragItem>().map(|item| (*item).clone()).or_else(|| foreign_here.then(|| crossdrag::take_drop(ui.ctx(), rect)).flatten());
        if let Some(item) = dropped {
            out.push(PlaylistAction::Drop { item, before: pl.entries.len() });
        }

        if let Some(y) = insert_line {
            ui.painter().line_segment([pos2(ui.min_rect().left(), y), pos2(ui.min_rect().left() + width, y)], Stroke::new(2.0, color));
        }
    });
    match cursor {
        Some(c) => ui.data_mut(|d| {
            if let Some(path) = pl.entries.get(c) {
                d.insert_temp(cursor_id, (c, path.clone()));
            }
        }),
        None => ui.data_mut(|d| d.remove::<(usize, PathBuf)>(cursor_id)),
    }
    ui.data_mut(|d| d.insert_temp(sel_id, sel));
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
    grid: Option<fw_library::BeatGrid>,
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
            let (seg_in, seg_out) = (grid.and_then(|g| g.at(0.0, duration as f64)), grid.and_then(|g| g.at(duration as f64, duration as f64)));
            if let Some(r) = edge_well(ui, id.with("in"), left, (0.0, span), &sk.head, 0.0, fade.fade_in, true, color, head(true), seg_in) {
                changed.fade_in = r;
            }
            let tail_t0 = duration - sk.tail.len() as f32 / EDGE_RATE;
            if let Some(r) = edge_well(ui, id.with("out"), right, (duration - span, duration), &sk.tail, tail_t0, fade.fade_out, false, color, head(false), seg_out) {
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
    let suggest = Rect::from_center_size(pos2(reset.left() - 8.0 - 48.0, foot_y), vec2(96.0, 20.0));
    let hint = if grid.is_some() { tr("Ziehen = Bereich (rastet auf Schläge, ⌥/Alt frei) · Klick = harter Schnitt · Rechtsklick = löschen") } else { tr("Ziehen = Bereich · Klick = harter Schnitt · Rechtsklick = löschen") };
    let tip = tr("Ein- und Ausblendbereich nach Beatraster und Lautstärke vorschlagen: Einstieg auf dem ersten Takt nach der Stille, Ausblenden vor dem Ausklang, so lang wie die Überblendzeit");
    if ui.place(suggest, |ui: &mut egui::Ui| hw::button(ui, suggest.size(), tr("VORSCHLAG"), fonts::bold(10.0), hw::Look::Plain, false, sk.is_some())).on_hover_text(tip).clicked() {
        out.push(PlaylistAction::SuggestFade(key.1));
    }
    p.with_clip_rect(Rect::from_min_max(pos2(inner.left(), foot_y - 10.0), pos2(suggest.left() - 8.0, foot_y + 10.0)))
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
    // Beatraster an dieser Stelle: Beat-Linien, Kanten rasten auf Schläge (mit ⌥/Alt frei)
    beats: Option<fw_library::Segment>,
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

    // Beat-Linien über der Wellenform, Taktbeginn kräftiger
    if let Some(seg) = beats {
        let (first, last) = (seg.beats_at(t0 as f64).ceil() as i64, seg.beats_at(t1 as f64).floor() as i64);
        for b in first..=last {
            let x = to_x((seg.downbeat + b as f64 * seg.beat_secs()) as f32);
            let bar = b.rem_euclid(4) == 0;
            p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(if bar { 1.2 } else { 0.6 }, egui::Color32::from_black_alpha(if bar { 130 } else { 70 })));
        }
    }

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

    // Nahe am Rand rastet die Zeit am Rand ein (Titelanfang bzw. -ende), sonst mit Beatraster auf den nächsten Schlag
    // (mit ⌥/Alt frei)
    let free = ui.input(|i| i.modifiers.alt);
    let snap = |x: f32| {
        if x <= rect.left() + EDGE_PX {
            t0
        } else if x >= rect.right() - EDGE_PX {
            t1
        } else {
            match beats.filter(|_| !free) {
                Some(seg) => crate::fade_suggest::snap_to_beat(to_t(x), seg).clamp(t0, t1),
                None => to_t(x),
            }
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

/// Kennzeichen für eigene Fades: links ein steigendes Dreieck (Einblenden), rechts ein fallendes (Ausblenden).
/// Gesetzte Seiten leuchten gelb, die andere bleibt schwach sichtbar.
/// Liefert den Text für den Tooltip.
fn fade_mark(p: &egui::Painter, rect: Rect, fade: TrackFade, alpha: f32) -> String {
    let on = theme::yellow().gamma_multiply(alpha);
    let off = theme::faint().gamma_multiply(0.45 * alpha);
    let w = (rect.width() - 2.0) / 2.0;
    let (top, bottom) = (rect.top(), rect.bottom());
    let l = rect.left();
    let rising = vec![pos2(l, bottom), pos2(l + w, bottom), pos2(l + w, top)];
    let r = rect.right();
    let falling = vec![pos2(r - w, top), pos2(r - w, bottom), pos2(r, bottom)];
    p.add(egui::Shape::convex_polygon(rising, if fade.fade_in.is_some() { on } else { off }, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(falling, if fade.fade_out.is_some() { on } else { off }, Stroke::NONE));
    let text = |r: Option<Region>, head: &str| match r {
        None => format!("{head}: {}", tr("nicht gesetzt (halbe Überblendzeit)")),
        Some(r) if r.len() < 0.05 => format!("{head}: {}", trf("harter Schnitt bei {}", &[&fmt_point(r.start)])),
        Some(r) => format!("{head}: {}", trf("{} bis {} · {} s", &[&fmt_point(r.start), &fmt_point(r.end), &fmt_secs(r.len())])),
    };
    format!("{}\n{}\n{}", tr("Eigene Fade-Bereiche"), text(fade.fade_in, tr("Einblenden")), text(fade.fade_out, tr("Ausblenden")))
}

/// Waagerechter Bereich der Wellenform in einer Zeile: hinter Titel und Interpret, bis vor die Kopfhörertaste.
fn wave_span(rect: Rect) -> (f32, f32) {
    // Endet vor dem Platz für das Fade-Kennzeichen, damit beide nicht übereinanderliegen
    (rect.left() + 32.0, rect.right() - 202.0)
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

/// Lupe in schmalen Playlist-Köpfen: öffnet die Schnellsuche.
fn lupe_button(ui: &mut egui::Ui) -> egui::Response {
    let resp = ui.add(egui::Button::new("").min_size(vec2(theme::ICON_BTN_W, HEADER_CTRL_H)));
    let color = ui.style().interact(&resp).fg_stroke.color;
    crate::browser::paint_lupe(ui.painter(), resp.rect.center(), color);
    resp
}

/// ⋯ für das Menü der Playlist (Sortieren, Leeren). Während Tonarten ermittelt werden, laufen die Punkte.
fn more_button(ui: &mut egui::Ui, busy: bool) -> egui::Response {
    let resp = ui.add(egui::Button::new("").min_size(vec2(theme::ICON_BTN_W, HEADER_CTRL_H)));
    let color = ui.style().interact(&resp).fg_stroke.color;
    let c = resp.rect.center();
    let lit = busy.then(|| (ui.input(|i| i.time) * 3.0) as usize % 3);
    for k in 0..3usize {
        let col = if lit == Some(k) { theme::yellow() } else { color };
        ui.painter().circle_filled(c + vec2((k as f32 - 1.0) * 6.0, 0.0), 2.0, col);
    }
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(330));
    }
    resp
}

/// Kreuz zum Schließen der geöffneten Schnellsuche.
fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let resp = ui.add(egui::Button::new("").min_size(vec2(theme::ICON_BTN_W, HEADER_CTRL_H)));
    let color = ui.style().interact(&resp).fg_stroke.color;
    let c = resp.rect.center();
    let r = 4.5;
    let stroke = egui::Stroke::new(1.8, color);
    let p = ui.painter();
    p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], stroke);
    p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], stroke);
    resp
}

/// Stellen der markierten Titel, aufsteigend.
fn marked_indices(sel: &Marked) -> Vec<usize> {
    let mut v: Vec<usize> = sel.items().map(|(i, _)| *i).collect();
    v.sort_unstable();
    v
}

/// Breite der FADEWERK-Taste und kleinste Breite der Schnellsuche in der Kopfzeile.
const FADEWERK_W: f32 = 84.0;
const SEARCH_MIN_W: f32 = 80.0;

/// Schnellsuche in der Kopfzeile einer Playlist. Liefert den Titel, zu dem gesprungen werden soll: beim Tippen
/// der erste Treffer ab dem geladenen Titel (die kommenden zuerst), mit Enter der nächste, mit ⇧Enter der vorige.
/// Esc leert das Feld. Gesucht wird mit allen Wörtern in Titel, Interpret, Album und Dateiname.
/// Dazu, falls „Alle markieren“ (oder ⌘A im Feld) gedrückt wurde, alle Treffer.
/// `fill` = so breit wie der ganze restliche Platz (eigene Zeile in schmalen Köpfen).
/// `focus` = Feld gleich zum Tippen bereit machen (nach Klick auf die Lupe).
/// `limit` = Platz für Feld, Trefferzahl und „Alle markieren“ zusammen; reicht er nicht, entfällt die Taste
/// (⌘A im Feld geht weiter).
#[allow(clippy::too_many_arguments)]
fn quick_search(ui: &mut egui::Ui, list: usize, pl: &Playlist, metas: &HashMap<PathBuf, TrackMeta>, cursor: Option<usize>, fill: bool, focus: bool, limit: f32) -> (Option<usize>, Option<Vec<usize>>) {
    let id = egui::Id::new(("pl_search", list));
    let mut query: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // Trefferzahl der letzten Suche, als (Stelle, Anzahl, Länge der Playlist dabei)
    let hits_id = id.with("hits");
    // In schmalen Köpfen nur so breit wie der Platz vor der FADEWERK-Taste, ganz ohne Platz gar nicht
    let room = ui.available_width() - 6.0;
    if room < 40.0 {
        return (None, None);
    }
    let searching = !query.trim().is_empty();
    let limit = limit.min(room);
    // Platz für die Trefferzahl „k/n“ und „Alle markieren“ lassen, sobald etwas gesucht wird
    let (hits_w, mark_w) = hits_widths(ui);
    // Beim Suchen darf das Feld schmaler werden, damit die Taste noch Platz hat
    let mark_button = !searching || limit >= SEARCH_MIN_SEARCHING_W + hits_w + mark_w;
    let extra = if !searching { 0.0 } else if mark_button { hits_w + mark_w } else { hits_w };
    let w = if fill { (room - extra).max(40.0) } else { (ui.available_width() * 0.45).clamp(SEARCH_MIN_W, 200.0).min(limit - extra).max(SEARCH_MIN_SEARCHING_W.min(room)) };
    let field_id = egui::Id::new(("pl_search_field", list));
    // ⌘A im Feld markiert alle Treffer in der Liste (statt den Suchtext)
    let mut mark = searching && ui.memory(|m| m.has_focus(field_id)) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
    let resp = ui.add(
        egui::TextEdit::singleline(&mut query)
            .id(field_id)
            .hint_text(tr("Suchen"))
            .desired_width(w)
            .min_size(vec2(w, HEADER_CTRL_H))
            .vertical_align(egui::Align::Center),
    );
    if focus {
        resp.request_focus();
    }
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
        let hits = search_hits(pl, metas, &query);
        let from = if next { cursor } else { pl.current };
        jump = match (next, from) {
            _ if hits.is_empty() => None,
            (true, Some(c)) if shift => Some(hits.iter().rev().find(|&&h| h < c).copied().unwrap_or(hits[hits.len() - 1])),
            (true, Some(c)) => Some(hits.iter().find(|&&h| h > c).copied().unwrap_or(hits[0])),
            (false, Some(c)) => Some(hits.iter().find(|&&h| h >= c).copied().unwrap_or(hits[0])),
            (_, None) => Some(hits[0]),
        };
        let pos = jump.and_then(|j| hits.iter().position(|&h| h == j)).map_or(0, |p| p + 1);
        ui.data_mut(|d| d.insert_temp(hits_id, (pos, hits.len(), pl.entries.len())));
    }
    if query.trim().is_empty() {
        ui.data_mut(|d| d.remove::<(usize, usize, usize)>(hits_id));
    } else if let Some((pos, mut n, len)) = ui.data(|d| d.get_temp::<(usize, usize, usize)>(hits_id)) {
        // Titel entfernt oder hinzugekommen: Trefferzahl neu zählen
        if len != pl.entries.len() {
            n = search_hits(pl, metas, &query).len();
            ui.data_mut(|d| d.insert_temp(hits_id, (pos.min(n), n, pl.entries.len())));
        }
        let text = if n == 0 { tr("kein Treffer").to_owned() } else { format!("{}/{n}", pos.min(n)) };
        ui.label(RichText::new(text).size(11.5).color(if n == 0 { theme::cue() } else { theme::dim() }));
        if n > 0 && mark_button {
            let b = ui.add(egui::Button::new(RichText::new(tr("Alle markieren")).font(FontId::proportional(11.5))).min_size(vec2(0.0, HEADER_CTRL_H)));
            mark |= b.on_hover_text(tr("Markiert alle Treffer in der Playlist (⌘A im Suchfeld). Danach mit Entf entfernen oder gemeinsam verschieben.")).clicked();
        }
    }
    resp.on_hover_text(tr("Zeigt nur passende Titel. Enter: nächster Treffer, ⇧Enter: voriger, Esc: leeren"));
    let marked = mark.then(|| search_hits(pl, metas, &query)).filter(|h| !h.is_empty());
    if marked.is_some() {
        // Das Feld gibt die Tasten ab, damit Entf die markierten Titel entfernt
        ui.memory_mut(|m| m.surrender_focus(field_id));
    }
    ui.data_mut(|d| d.insert_temp(id, query));
    (jump, marked)
}

/// Platz für die Trefferzahl „k/n“ und die Taste „Alle markieren“ daneben.
const HITS_W: f32 = 56.0;
/// Kleinste Breite des Suchfelds, solange gesucht wird.
const SEARCH_MIN_SEARCHING_W: f32 = 56.0;

/// Breiten von Trefferzahl und „Alle markieren“ neben dem Suchfeld (samt Abstand).
fn hits_widths(ui: &egui::Ui) -> (f32, f32) {
    let gap = ui.spacing().item_spacing.x;
    let text_w = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), FontId::proportional(11.5), egui::Color32::WHITE).size().x;
    (HITS_W.min(text_w("888/888") + gap), text_w(tr("Alle markieren")) + 2.0 * ui.spacing().button_padding.x + gap)
}

/// Stellen aller Titel, die alle Wörter von `query` in Titel, Interpret, Album oder Dateiname enthalten.
/// Treffer der Schnellsuche für die gefilterte Ansicht. Gemerkt mit Suchtext, Länge der Liste und Zahl der
/// Tags; bleiben alle gemerkten Treffer an ihrer Stelle, gilt die Liste weiter (Umsortieren verschiebt sonst
/// Treffer), damit lange Playlists nicht jedes Bild durchsucht werden.
fn filtered(ui: &egui::Ui, list: usize, pl: &Playlist, metas: &HashMap<PathBuf, TrackMeta>, query: &str) -> Vec<usize> {
    type Cached = (String, usize, usize, Arc<Vec<(usize, PathBuf)>>);
    let id = egui::Id::new(("pl_filter", list));
    if let Some((q, len, n_meta, hits)) = ui.data(|d| d.get_temp::<Cached>(id))
        && q == query
        && len == pl.entries.len()
        && n_meta == metas.len()
        && hits.iter().all(|(i, p)| pl.entries.get(*i) == Some(p))
    {
        return hits.iter().map(|(i, _)| *i).collect();
    }
    let hits = search_hits(pl, metas, query);
    let keep = Arc::new(hits.iter().map(|&i| (i, pl.entries[i].clone())).collect());
    ui.data_mut(|d| d.insert_temp::<Cached>(id, (query.to_owned(), pl.entries.len(), metas.len(), keep)));
    hits
}

fn search_hits(pl: &Playlist, metas: &HashMap<PathBuf, TrackMeta>, query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(crate::browser::fold).collect();
    if words.is_empty() {
        return Vec::new();
    }
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
}
