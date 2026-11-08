// SPDX-License-Identifier: GPL-3.0-or-later
//! Aufbau der beiden Arbeitsweisen im Hauptbereich: Live (Player, Mixer, Playlists) und Automatik (Leiste mit
//! Jetzt, Pegeln, Übergang, Bedienung und Als Nächstes, darunter die Playlists). Zeichnet nur und sammelt Aktionen;
//! ausgeführt werden sie in `FadeWerkDjApp::ui`.

use super::*;
use crate::fade_preview::EditorAudio;

/// Was die Ansichten zum Zeichnen brauchen (in `ui` berechnet).
pub(super) struct Views<'a> {
    pub decks: &'a [DeckModel<'a>],
    pub mixer: &'a MixerModel<'a>,
    pub fade: &'a FadeModel<'a>,
    pub transition: &'a mixer_ui::TransitionView,
    pub playlists: &'a [Playlist; 2],
    pub info: playlist_ui::TrackInfo<'a>,
    /// Je Player: Titel geladen, spielt gerade.
    pub has: [bool; 2],
    pub playing: [bool; 2],
    /// Titel, der gerade vorgehört wird.
    pub preview: Option<&'a Path>,
    /// Wellenform-Skizzen im FadeWerk-Modus, sonst `None`.
    pub sketches: Option<&'a HashMap<PathBuf, Option<Arc<WaveSketch>>>>,
    pub editor_audio: &'a EditorAudio,
    pub compact: bool,
    /// Player auf Sendung (Automatik: Karte „Jetzt“).
    pub air_deck: usize,
    /// Je Player: nur der andere ist zu hören (tritt zurück).
    pub off_air: [bool; 2],
    /// Gezogene Höhe der Player, 0 = automatisch.
    pub deck_height: f32,
    /// Offener Reiter unter dem Crossfader.
    pub center_tab: Option<mixer_ui::CenterTab>,
}

/// Was die Ansichten zurückgeben.
#[derive(Default)]
pub(super) struct ViewOut {
    pub deck: [Vec<DeckAction>; 2],
    pub mixer: Vec<MixerAction>,
    pub fade: Vec<FadeAction>,
    pub playlist: [Vec<PlaylistAction>; 2],
    /// Auf einen Player gezogene Titel.
    pub drops: Vec<(usize, DragItem)>,
    /// Reiter geöffnet oder geschlossen.
    pub center_tab: Option<Option<mixer_ui::CenterTab>>,
    /// Höhe der Player per Griff geändert (0 = wieder automatisch).
    pub deck_height: Option<f32>,
}

/// Automatik: oben was läuft, Pegel, der Übergang groß, Bedienung und was kommt; darunter die Playlists mit
/// einer Übersicht je Titel über die volle Breite.
pub(super) fn automatic(ui: &mut egui::Ui, v: &Views<'_>, out: &mut ViewOut) {
    let compact = v.compact;
    let hero_h = if compact { 136.0 } else { 196.0 };
    StripBuilder::new(ui).size(Size::exact(hero_h)).size(Size::remainder()).vertical(|mut rows| {
        rows.cell(|ui| {
            panel_frame(ui).show(ui, |ui| {
                // Ein Raster: alle Teile teilen Ober- und Unterkante. Platte | Display aus Pegeln und
                // Übergang | Tasten und Master | Platte
                let r = ui.available_rect_before_wrap();
                ui.allocate_rect(r, Sense::hover());
                let band = r.shrink2(vec2(0.0, if compact { 2.0 } else { 6.0 }));
                let bh = band.height();
                let gap = if compact { 14.0 } else { 22.0 };
                // Ohne Plattenspieler brauchen die Karten nur Platz für den Text
                let card_w = if v.decks[0].platter { (r.width() * 0.25).clamp(200.0, 420.0) } else { (r.width() * 0.17).clamp(160.0, 300.0) };
                let controls_w = if compact { 190.0 } else { 236.0 };
                let left = Rect::from_min_size(band.min, vec2(card_w, bh));
                let right = Rect::from_min_max(pos2(band.right() - card_w, band.top()), band.max);
                let controls = Rect::from_min_max(pos2(right.left() - gap - controls_w, band.top()), pos2(right.left() - gap, band.bottom()));
                let display = Rect::from_min_max(pos2(left.right() + gap, band.top()), pos2(controls.left() - gap, band.bottom()));
                // A sitzt immer links, B immer rechts; nur die Beschriftung zeigt, wer gerade spielt. Jede Karte
                // bedient ihren eigenen Player (Tasten nur in einer wirkten, als gehörten sie zu A)
                let caption = |d: usize| if d == v.air_deck { tr("JETZT") } else { tr("ALS NÄCHSTES") };
                let [a_out, b_out] = &mut out.deck;
                deck_ui::hero_card(ui, left, &v.decks[0], false, caption(0), Some(a_out));
                deck_ui::hero_card(ui, right, &v.decks[1], true, caption(1), Some(b_out));
                // Display: Pegel links, feine Trennlinie, rechts der Übergang
                crate::hw::well(ui.painter(), display, 6.0);
                // Kompakt schmaler, aber nur ohne Frequenzbänder: zwischen L und R von MAIN passen sonst keine Säulen
                let meters_w = if compact && theme::lite().no_bands { 56.0 } else { 70.0 };
                let meters = Rect::from_min_size(display.min + vec2(12.0, 9.0), vec2(meters_w, bh - 15.0));
                mixer_ui::display_meters(ui.painter(), meters, v.mixer.meters, v.mixer.bands);
                let sep_x = meters.right() + 12.0;
                ui.painter().vline(sep_x, display.y_range().shrink(10.0), Stroke::new(1.0, egui::Color32::from_white_alpha(22)));
                let trans = Rect::from_min_max(pos2(sep_x + 2.0, display.top()), display.max);
                if let Some(secs) = ui.scope_builder(egui::UiBuilder::new().max_rect(trans), |ui| mixer_ui::transition_bar(ui, v.transition, Some(bh), true, false)).inner {
                    out.fade.push(FadeAction::FadeSecs(secs));
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
                    mixer_ui::auto_controls(ui, v.fade, v.mixer, &mut out.fade, &mut out.mixer);
                });
            });
        });
        rows.strip(|b| {
            b.size(Size::remainder()).size(Size::remainder()).horizontal(|mut cols| {
                for d in 0..2 {
                    cols.cell(|ui| {
                        let cell = ui.max_rect();
                        panel_frame(ui).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            let list_h = ui.available_height() - deck_ui::MINI_H - 10.0;
                            ui.allocate_ui(vec2(ui.available_width(), list_h), |ui| {
                                ui.set_min_height(list_h);
                                playlist_ui::show(ui, d, &v.playlists[d], v.has[d], v.playing[d], v.preview, &v.info, v.sketches, v.editor_audio, &mut out.playlist[d]);
                            });
                            ui.separator();
                            let mini = ui.available_rect_before_wrap();
                            deck_ui::show_mini(ui, &v.decks[d], &mut out.deck[d]);
                            let mini = mini.with_max_y(mini.top() + deck_ui::MINI_H);
                            if let Some(item) = deck_drop_zone(ui, mini, d) {
                                out.drops.push((d, item));
                            }
                        });
                        end_warning(ui, cell.shrink(3.0), &v.decks[d]);
                    });
                }
            });
        });
    });
}

/// Live: links Player A über Playlist A, rechts B über B. Die Mitte (Mixer und Auto-Fade) läuft über die ganze
/// Höhe, damit keine Zeile die andere in die Länge zieht.
pub(super) fn live(ui: &mut egui::Ui, v: &Views<'_>, out: &mut ViewOut) {
    let compact = v.compact;
    let center_w = center_width(ui.available_width());
    // Player wachsen mit ihrer Breite auch in der Höhe, damit sie nicht zu flachen Streifen werden
    let deck_inner = (ui.available_width() - center_w) / 2.0 - 36.0;
    // Höchstens gut die Hälfte der Höhe, damit die Playlist darunter Platz behält; auf kleinen
    // Bildschirmen schrumpfen die Player. Hat der Nutzer die Kante gezogen, gilt seine Höhe.
    let avail_h = ui.available_height();
    let deck_max = DECK_ROW_HEIGHT * deck_ui::deck_scale(deck_inner);
    let wanted = if v.deck_height > 0.0 { v.deck_height } else { deck_max.min(avail_h * 0.58) };
    let deck_h = if compact { deck_ui::COMPACT_ROW } else { wanted.min(deck_max).min(avail_h - PLAYLIST_MIN).max(DECK_ROW_MIN) };
    // Je Seite: Zelle des Players und Oberkante der Playlist, für den Griff dazwischen
    let mut split_bounds = [(Rect::NOTHING, 0.0f32); 2];
    StripBuilder::new(ui).size(Size::remainder()).size(Size::exact(center_w)).size(Size::remainder()).horizontal(|mut cols| {
        for side in [0usize, 2] {
            let d = side / 2;
            if side == 2 {
                cols.cell(|ui| {
                    panel_frame(ui).show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        egui::ScrollArea::vertical().id_salt("center").auto_shrink([false, false]).show(ui, |ui| {
                            mixer_ui::show_mixer(ui, v.mixer, &mut out.mixer);
                            ui.separator();
                            mixer_ui::show_fade(ui, v.fade, &mut out.fade);
                            if let Some(t) = mixer_ui::show_tabs(ui, v.center_tab, v.mixer, &mut out.mixer, v.fade, &mut out.fade) {
                                out.center_tab = Some(t);
                            }
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
                            deck_ui::show(ui, &v.decks[d], &mut out.deck[d]);
                        });
                        // Player auf Sendung hervorheben: Ist nur der andere zu hören, tritt dieser zurück
                        // (beim Darüberfahren wieder voll da, zum Vorbereiten)
                        if v.off_air[d] && !ui.rect_contains_pointer(cell) {
                            ui.painter().rect_filled(cell.shrink(2.0), 8.0, theme::ground().gamma_multiply(OFF_AIR_DIM));
                        }
                        end_warning(ui, cell.shrink(3.0), &v.decks[d]);
                        if let Some(item) = deck_drop_zone(ui, cell.shrink(3.0), d) {
                            out.drops.push((d, item));
                        }
                    });
                    rows.cell(|ui| {
                        split_bounds[d].1 = ui.max_rect().top();
                        panel_frame(ui).show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            playlist_ui::show(ui, d, &v.playlists[d], v.has[d], v.playing[d], v.preview, &v.info, v.sketches, v.editor_audio, &mut out.playlist[d]);
                        });
                    });
                });
            });
        }
    });
    // Griff zwischen Player und Playlist: ziehen ändert die Höhe der Player, Doppelklick = automatisch
    for (d, (cell, list_top)) in split_bounds.iter().enumerate().filter(|_| !compact) {
        let y = (cell.bottom() + list_top) / 2.0;
        let rect = Rect::from_min_max(pos2(cell.left() + 6.0, y - 4.0), pos2(cell.right() - 6.0, y + 4.0));
        let resp = ui.interact(rect, egui::Id::new(("deck_split", d)), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeRow);
        if resp.hovered() || resp.dragged() {
            let line = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.3, 3.0));
            ui.painter().rect_filled(line, 1.5, theme::dim());
        }
        if resp.double_clicked() {
            out.deck_height = Some(0.0);
        } else if resp.dragged() && resp.drag_delta().y != 0.0 {
            out.deck_height = Some((deck_h + resp.drag_delta().y).max(DECK_ROW_MIN));
        }
        resp.on_hover_text(tr("Ziehen ändert die Höhe der Player, Doppelklick stellt sie wieder automatisch ein"));
    }
}
