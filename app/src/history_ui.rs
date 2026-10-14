// SPDX-License-Identifier: GPL-3.0-or-later
//! Fenster zur Historie: Playlist als Tag importieren (Datum und Name abfragen) und Tag löschen.

use eframe::egui::{self, Align2, Button, Frame, Key, Margin, RichText, vec2};

use crate::history::{Day, Entry, parse_date};
use crate::i18n::{tr, trf};
use crate::theme;

pub enum DialogEvent {
    /// Importierte Playlist als Tag übernehmen.
    Import(Day),
    /// Tag endgültig löschen.
    Delete(String),
    Close,
}

pub enum HistoryDialog {
    /// Importierte Playlist: Datum und Name fehlen noch. `found` = wiedergefundene Titel.
    Import { entries: Vec<Entry>, found: usize, date: String, name: String },
    /// Rückfrage vor dem Löschen eines Tages.
    Delete { date: String, label: String, count: usize },
}

fn window(ctx: &egui::Context, title: &str, id: &str) -> egui::Window<'static> {
    egui::Window::new(title.to_owned())
        .id(egui::Id::new(id.to_owned()))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
}

/// Hervorgehobene Taste in einer Signalfarbe, wie die Playlist-Tasten über der Titelliste.
fn accent_button(text: &str, c: egui::Color32) -> Button<'static> {
    Button::new(RichText::new(text.to_owned()).color(c).strong()).fill(c.gamma_multiply(0.14)).stroke(egui::Stroke::new(1.0, c.gamma_multiply(0.7)))
}

impl HistoryDialog {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<DialogEvent> {
        let mut event = None;
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        match self {
            Self::Import { entries, found, date, name } => {
                window(ctx, tr("Playlist als Historie importieren"), "historie-import").show(ctx, |ui| {
                    ui.set_width(380.0);
                    let missing = entries.iter().filter(|e| !e.path.is_file()).count();
                    ui.label(RichText::new(trf("{} Titel gelesen", &[&entries.len()])).strong());
                    if *found > 0 {
                        ui.label(RichText::new(trf("{} davon in deinen Musikordnern wiedergefunden", &[&*found])).size(12.0).color(theme::dim()));
                    }
                    if missing > 0 {
                        ui.label(RichText::new(trf("{} nicht gefunden (bleiben ausgegraut in der Liste)", &[&missing])).size(12.0).color(theme::yellow()));
                    }
                    ui.add_space(8.0);
                    let valid = parse_date(date).is_some();
                    egui::Grid::new("historie-import-felder").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                        ui.label(RichText::new(tr("Datum")).color(theme::dim()));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(date).hint_text("2026-10-14").desired_width(110.0));
                            if !valid {
                                ui.label(RichText::new(tr("Format JJJJ-MM-TT")).size(12.0).color(theme::cue()));
                            }
                        });
                        ui.end_row();
                        ui.label(RichText::new(tr("Name")).color(theme::dim()));
                        ui.add(egui::TextEdit::singleline(name).hint_text(tr("Name, z. B. Omas Geburtstag")).desired_width(240.0));
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tr("Gibt es den Tag schon, kommen die Titel dazu."))
                            .size(11.5)
                            .color(theme::faint()),
                    );
                    ui.add_space(8.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = accent_button(tr("Importieren"), theme::play());
                        if ui.add_enabled(valid, b).clicked()
                            && let Some(d) = parse_date(date)
                        {
                            event = Some(DialogEvent::Import(Day { date: d, name: name.trim().to_owned(), entries: std::mem::take(entries) }));
                        }
                        if ui.button(tr("Abbrechen")).clicked() || esc {
                            event = Some(DialogEvent::Close);
                        }
                    });
                });
            }
            Self::Delete { date, label, count } => {
                window(ctx, tr("Tag löschen?"), "historie-loeschen").show(ctx, |ui| {
                    ui.set_width(360.0);
                    ui.label(RichText::new(label.as_str()).strong());
                    ui.label(RichText::new(trf("{} Titel. Das Löschen lässt sich nicht rückgängig machen; die Musikdateien bleiben, wo sie sind.", &[&*count])).color(theme::dim()));
                    ui.add_space(10.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let b = accent_button(tr("Löschen"), theme::cue());
                        if ui.add(b).clicked() {
                            event = Some(DialogEvent::Delete(date.clone()));
                        }
                        if ui.button(tr("Abbrechen")).clicked() || esc {
                            event = Some(DialogEvent::Close);
                        }
                    });
                });
            }
        }
        event
    }
}
