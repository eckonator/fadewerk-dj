// SPDX-License-Identifier: GPL-3.0-or-later
//! Befehlssuche (⌘K bzw. Strg+K, oder das Suchfeld in der Statuszeile): findet Befehle und Einstellungen, die
//! sonst in Menüs und im Einstellungsfenster stecken. Die Einträge baut `app.rs` beim Öffnen; hier wird nur
//! gesucht, gewählt und der gewählte Index zurückgegeben.

use eframe::egui::{self, Align2, RichText, Sense, vec2};

use crate::i18n::tr;
use crate::{fonts, theme};

/// Ein Eintrag: Name, Bereich (z. B. „Einstellungen › Leistung“) und optional ein Tastenkürzel.
pub struct Item {
    pub label: String,
    pub group: String,
    pub keys: &'static str,
}

/// So viele Treffer stehen höchstens in der Liste.
const MAX_SHOWN: usize = 12;
const WIDTH: f32 = 560.0;
const ROW_H: f32 = 30.0;

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    sel: usize,
    /// Im ersten Bild nach dem Öffnen den Fokus ins Feld setzen.
    focus: bool,
}

impl Palette {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.sel = 0;
        self.focus = true;
    }

    /// Mit vorgegebenem Suchtext öffnen (Entwicklerhilfe `FW_PALETTE`).
    pub fn open_with(&mut self, query: &str) {
        self.open();
        self.query = query.to_owned();
    }

    /// Zeigt die Suche; liefert den Index des gewählten Eintrags in `items`.
    pub fn show(&mut self, ctx: &egui::Context, items: &[Item]) -> Option<usize> {
        if !self.open {
            return None;
        }
        let words: Vec<String> = self.query.split_whitespace().map(fold).collect();
        let hits: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                let hay = fold(&format!("{} {}", it.label, it.group));
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .take(MAX_SHOWN)
            .collect();
        self.sel = self.sel.min(hits.len().saturating_sub(1));
        let (up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            )
        });
        if up {
            self.sel = self.sel.saturating_sub(1);
        }
        if down && self.sel + 1 < hits.len() {
            self.sel += 1;
        }
        let mut picked = enter.then(|| hits.get(self.sel).copied()).flatten();

        let modal = egui::Modal::new(egui::Id::new("befehlssuche"))
            .frame(egui::Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                let edit = egui::TextEdit::singleline(&mut self.query)
                    .hint_text(tr("Befehl oder Einstellung suchen …"))
                    .font(egui::FontId::proportional(17.0))
                    .desired_width(WIDTH)
                    .min_size(vec2(WIDTH, 36.0))
                    .vertical_align(egui::Align::Center);
                let r = ui.add(edit);
                if std::mem::take(&mut self.focus) {
                    r.request_focus();
                }
                if r.changed() {
                    self.sel = 0;
                }
                ui.add_space(6.0);
                if hits.is_empty() {
                    ui.label(RichText::new(tr("Nichts gefunden")).color(theme::faint()));
                }
                for (row, &i) in hits.iter().enumerate() {
                    let it = &items[i];
                    let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, ROW_H), Sense::click());
                    if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                        self.sel = row;
                    }
                    let p = ui.painter();
                    if row == self.sel {
                        p.rect_filled(rect, 5.0, theme::row_selected());
                    }
                    p.text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, &it.label, egui::FontId::proportional(14.0), theme::ink());
                    let g = p.layout_no_wrap(it.group.clone(), egui::FontId::proportional(12.0), theme::faint());
                    let gx = rect.right() - 10.0 - g.size().x;
                    p.galley(egui::pos2(gx, rect.center().y - g.size().y / 2.0), g, theme::faint());
                    if !it.keys.is_empty() {
                        let k = p.layout_no_wrap(it.keys.to_owned(), fonts::bold(10.5), theme::dim());
                        let kr = egui::Rect::from_min_size(egui::pos2(gx - k.size().x - 22.0, rect.center().y - 9.0), vec2(k.size().x + 8.0, 18.0));
                        p.rect_stroke(kr, 3.0, egui::Stroke::new(1.0, theme::line()), egui::StrokeKind::Inside);
                        p.galley(kr.center() - k.size() / 2.0, k, theme::dim());
                    }
                    if resp.clicked() {
                        picked = Some(i);
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new(tr("↑↓ wählen · Enter ausführen · Esc schließen")).size(11.5).color(theme::faint()));
            });
        if modal.should_close() || picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// Klein und ohne Umlaute vergleichen: „ubergang“ findet „Übergang“.
fn fold(s: &str) -> String {
    s.to_lowercase().replace('ä', "a").replace('ö', "o").replace('ü', "u").replace('ß', "ss")
}
