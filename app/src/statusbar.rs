// SPDX-License-Identifier: GPL-3.0-or-later
//! Statuszeile über der ganzen Oberfläche: Marke mit Versionsnummer, was gerade auf Sendung ist und was als
//! Nächstes kommt (solange ein Hinweis aktiv ist, steht er dort), Arbeitsweise Live/Automatik, Befehlssuche,
//! laufende Aufnahme, Vollbild und Einstellungen. Zeichnet nur und gibt Aktionen zurück; Logik in `app.rs`.

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};

use crate::i18n::{tr, trf};
use crate::settings::ViewMode;
use crate::{fonts, hw, theme};

/// Höhe der Statuszeile.
pub const STATUS_H: f32 = 40.0;
/// Breite des Suchfelds für Befehle.
const SEARCH_W: f32 = 210.0;

/// Titel, der gerade über Master zu hören ist.
pub struct OnAir {
    pub deck: usize,
    pub title: String,
    /// Restzeit in Sekunden.
    pub left: f64,
    /// Titel endet bald: Restzeit blinkt rot (wie der Rahmen des Players).
    pub warn: bool,
}

pub struct StatusModel {
    pub on_air: Option<OnAir>,
    /// Kommt als Nächstes: Player, Titel und Sekunden bis zur Überblendung (`None` = blendet gerade über).
    pub next: Option<(usize, String, Option<f64>)>,
    /// Hinweis der App; steht statt „als Nächstes“, solange er gilt.
    pub notice: Option<String>,
    pub view: ViewMode,
    /// Laufende Aufnahme: Sekunden bisher.
    pub recording: Option<f64>,
    pub fullscreen: bool,
}

pub enum StatusAction {
    /// Einstellungen auf der Seite „Über“ (Klick auf die Versionsnummer).
    About,
    View(ViewMode),
    /// Befehlssuche öffnen.
    Palette,
    /// Laufende Aufnahme beenden.
    StopRecording,
    Fullscreen,
    Settings,
}

pub fn show(ui: &mut egui::Ui, m: &StatusModel, out: &mut Vec<StatusAction>) {
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.spacing_mut().interact_size.y = theme::CTRL_H;
        brand(ui, out);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let icon = vec2(theme::ICON_BTN_W, theme::CTRL_H);
            if ui.add_sized(icon, egui::Button::new(egui::RichText::new("⚙").size(15.0))).on_hover_text(tr("Einstellungen (⌘,)")).clicked() {
                out.push(StatusAction::Settings);
            }
            if crate::browser::fullscreen_button_sized(ui, icon, m.fullscreen).clicked() {
                out.push(StatusAction::Fullscreen);
            }
            if let Some(secs) = m.recording
                && recording(ui, secs).clicked()
            {
                out.push(StatusAction::StopRecording);
            }
            if search_field(ui).clicked() {
                out.push(StatusAction::Palette);
            }
            if let Some(v) = view_switch(ui, m.view) {
                out.push(StatusAction::View(v));
            }
            let w = ui.available_width();
            if w > 80.0 {
                on_air(ui, w, m);
            }
        });
    });
}

/// Marke links: Platte in den Farben der Player, „FADEWERK DJ“ und klein die Versionsnummer (Klick → „Über“).
fn brand(ui: &mut egui::Ui, out: &mut Vec<StatusAction>) {
    let (mark, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let p = ui.painter();
    let c = mark.center();
    let half = |from: f32, color: Color32| {
        let pts: Vec<_> = (0..=16).map(|i| {
            let a = from + i as f32 / 16.0 * std::f32::consts::PI;
            c + vec2(a.cos(), a.sin()) * 8.0
        }).collect();
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    };
    half(std::f32::consts::FRAC_PI_2, theme::deck_color(0));
    half(-std::f32::consts::FRAC_PI_2, theme::deck_color(1));
    p.circle_filled(c, 2.5, theme::ground());
    ui.add_space(-4.0);
    ui.label(egui::RichText::new("FADEWERK DJ").font(fonts::bold(13.0)).extra_letter_spacing(1.8).color(theme::dim()));
    ui.add_space(-6.0);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let g = ui.painter().layout_no_wrap(version, fonts::list(10.5), theme::faint());
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(4.0, 0.0), Sense::click());
    let resp = resp.on_hover_text(tr("Über FadeWerk DJ")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if resp.hovered() { theme::dim() } else { theme::faint() };
    // Grundlinie wie die Marke: etwas tiefer als die Mitte
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 2.0, rect.top() + 2.0), g, color);
    if resp.clicked() {
        out.push(StatusAction::About);
    }
}

/// Display mit dem Titel auf Sendung, rechts darin das Nächste oder ein Hinweis.
fn on_air(ui: &mut egui::Ui, w: f32, m: &StatusModel) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, theme::CTRL_H + 2.0), Sense::hover());
    // Abgeschnittene Hinweise ganz lesen
    if let Some(text) = &m.notice {
        resp.on_hover_text(text);
    }
    let p = ui.painter();
    hw::well(p, rect, 5.0);
    let ink = theme::well_ink();
    let inner = rect.shrink2(vec2(10.0, 0.0));
    let y = rect.center().y;
    let now = ui.input(|i| i.time);
    // Rechts: Hinweis oder was als Nächstes kommt, höchstens gut die Hälfte des Displays
    let right_max = inner.width() * 0.52;
    let mut right_w = 0.0;
    let small = fonts::bold(12.0);
    if let Some(text) = &m.notice {
        let mut job = egui::text::LayoutJob::single_section(text.clone(), egui::TextFormat::simple(small.clone(), ink));
        job.wrap = egui::text::TextWrapping::truncate_at_width(right_max - 14.0);
        let g = p.layout_job(job);
        right_w = g.size().x + 14.0;
        p.circle_filled(pos2(inner.right() - g.size().x - 9.0, y), 3.5, theme::yellow());
        p.galley(pos2(inner.right() - g.size().x, y - g.size().y / 2.0), g, ink);
    } else if let Some((d, title, until)) = &m.next {
        let mut job = egui::text::LayoutJob::default();
        let fmt = |c: Color32| egui::TextFormat::simple(small.clone(), c);
        match until {
            Some(_) => job.append("→ ", 0.0, fmt(ink.gamma_multiply(0.6))),
            None => job.append(tr("blendet über → "), 0.0, fmt(theme::yellow())),
        }
        job.append(theme::deck_letter(*d), 0.0, fmt(theme::deck_color(*d)));
        job.append(&format!(" · {title}"), 0.0, fmt(ink.gamma_multiply(0.85)));
        if let Some(secs) = until {
            job.append(&trf(" in {}", &[&crate::deck_ui::fmt_time(*secs)]), 0.0, fmt(ink.gamma_multiply(0.6)));
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(right_max);
        let g = p.layout_job(job);
        right_w = g.size().x;
        p.galley(pos2(inner.right() - g.size().x, y - g.size().y / 2.0), g, ink);
    }
    // Links: ON AIR, Player, Titel, Restzeit
    let Some(air) = &m.on_air else {
        p.text(pos2(inner.left(), y), Align2::LEFT_CENTER, tr("Nichts auf Sendung"), small, ink.gamma_multiply(0.45));
        return;
    };
    let tag = p.layout_no_wrap(tr("ON AIR").to_owned(), fonts::bold(9.5), theme::cue());
    let tag_rect = Rect::from_min_size(pos2(inner.left(), y - 8.0), vec2(tag.size().x + 10.0, 16.0));
    p.rect_stroke(tag_rect, 3.0, Stroke::new(1.2, theme::cue()), egui::StrokeKind::Inside);
    p.galley(tag_rect.center() - tag.size() / 2.0, tag, theme::cue());
    let mut x = tag_rect.right() + 10.0;
    let color = theme::deck_color(air.deck);
    let letter = p.layout_no_wrap(theme::deck_letter(air.deck).to_owned(), fonts::bold(15.0), color);
    let lw = letter.size().x;
    p.galley(pos2(x, y - letter.size().y / 2.0), letter, color);
    x += lw + 8.0;
    // Restzeit hinter dem Titel; blinkt rot kurz vor Titelende
    let time = format!("-{}", crate::deck_ui::fmt_time(air.left.max(0.0)));
    let time_w = p.layout_no_wrap(time.clone(), fonts::lcd(14.0), ink).size().x;
    let title_room = (inner.right() - right_w - 16.0 - time_w - 10.0 - x).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(air.title.clone(), egui::TextFormat::simple(fonts::bold(13.0), ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(title_room);
    let g = p.layout_job(job);
    let gw = g.size().x;
    p.galley(pos2(x, y - g.size().y / 2.0), g, ink);
    x += gw + 10.0;
    let blink_off = air.warn && now % 0.8 > 0.5;
    if air.warn {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if !blink_off {
        hw::lcd_text(p, pos2(x, y), Align2::LEFT_CENTER, &time, 14.0, if air.warn { theme::cue() } else { color });
    }
}

/// LIVE | AUTOMATIK als zwei Segment-Tasten, die gewählte leuchtet.
fn view_switch(ui: &mut egui::Ui, view: ViewMode) -> Option<ViewMode> {
    let mut picked = None;
    let font = fonts::bold(11.0);
    // Von rechts nach links: erst AUTOMATIK, dann LIVE
    ui.spacing_mut().item_spacing.x = 3.0;
    for (v, label, tip) in [
        (ViewMode::Auto, tr("AUTOMATIK"), tr("Automatik: Playlists und Übergang groß, für den Abend, der allein läuft")),
        (ViewMode::Live, tr("LIVE"), tr("Live: Teller, EQ, Pads und Loops zum Auflegen von Hand")),
    ] {
        let on = v == view;
        let w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::ink()).size().x + 22.0;
        if hw::button(ui, vec2(w, theme::CTRL_H), label, font.clone(), hw::Look::Color(theme::select_accent()), on, true).on_hover_text(tip).clicked() && !on {
            picked = Some(v);
        }
    }
    ui.spacing_mut().item_spacing.x = 10.0;
    picked
}

/// Suchfeld für Befehle und Einstellungen; ein Klick (oder ⌘K) öffnet die Befehlssuche.
fn search_field(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(SEARCH_W, theme::CTRL_H), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::Text);
    let p = ui.painter();
    let v = ui.visuals();
    p.rect_filled(rect, 4.0, v.extreme_bg_color);
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, if resp.hovered() { theme::dim() } else { theme::line() }), egui::StrokeKind::Inside);
    crate::browser::paint_lupe(p, pos2(rect.left() + 13.0, rect.center().y), theme::faint());
    p.text(pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, tr("Befehl suchen …"), egui::FontId::proportional(13.0), theme::faint());
    let key = if cfg!(target_os = "macos") { "⌘K" } else { "Strg+K" };
    let g = p.layout_no_wrap(key.to_owned(), fonts::bold(10.5), theme::dim());
    let kr = Rect::from_min_size(pos2(rect.right() - g.size().x - 14.0, rect.center().y - 8.5), vec2(g.size().x + 8.0, 17.0));
    p.rect_stroke(kr, 3.0, Stroke::new(1.0, theme::line()), egui::StrokeKind::Inside);
    p.galley(kr.center() - g.size() / 2.0, g, theme::dim());
    resp
}

/// Läuft eine Aufnahme: blinkender Punkt und Laufzeit; Klick beendet sie.
fn recording(ui: &mut egui::Ui, secs: f64) -> egui::Response {
    let s = secs as u64;
    let label = if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) };
    let blink = ui.input(|i| i.time) % 1.0 < 0.6;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    hw::custom_button(ui, vec2(86.0, theme::CTRL_H), hw::Look::Color(theme::cue()), true, true, |p, r, color| {
        let g = p.layout_no_wrap(format!("REC {label}"), fonts::bold(11.0), color);
        let left = r.center().x - (g.size().x + 12.0) / 2.0;
        p.circle_filled(pos2(left + 3.5, r.center().y), 3.5, if blink { theme::cue() } else { theme::cue().gamma_multiply(0.25) });
        p.galley(pos2(left + 12.0, r.center().y - g.size().y / 2.0), g, color);
    })
    .on_hover_text(tr("Aufnahme beenden"))
}
