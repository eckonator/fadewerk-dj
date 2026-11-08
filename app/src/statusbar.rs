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

/// Uhr und Systemwerte rechts in der Statuszeile; `None` = nicht gezeigt.
pub struct SysLine {
    pub clock: Option<String>,
    pub battery: Option<crate::sysmon::Battery>,
    /// CPU-Last in %, Auslastung des Audio-Threads (1 = voll) und ob gerade Aussetzer waren.
    pub cpu: Option<(f32, f32, bool)>,
    pub ram: Option<f32>,
    pub temp: Option<f32>,
}

pub struct StatusModel {
    pub sys: SysLine,
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
    /// Oberfläche sperren.
    Lock,
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
            // Wie Vollbild und ⚙: egui-Taste mit selbst gezeichnetem Schloss
            let tip = if cfg!(target_os = "macos") { tr("Oberfläche sperren (⌘L)") } else { tr("Oberfläche sperren (Strg+L)") };
            let lock = ui.add_sized(icon, egui::Button::new("")).on_hover_text(tip);
            crate::lock::paint_icon(ui.painter(), lock.rect.center(), 15.0, ui.style().interact(&lock).fg_stroke.color, false);
            if lock.clicked() {
                out.push(StatusAction::Lock);
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
    let name = egui::RichText::new("FADEWERK DJ").font(fonts::bold(13.0)).extra_letter_spacing(1.8).color(theme::dim());
    let name = egui::WidgetText::from(name).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let (name_rect, _) = ui.allocate_exact_size(name.size(), Sense::hover());
    let name_base = name_rect.top() + baseline(&name);
    ui.painter().galley(name_rect.min, name, theme::dim());
    ui.add_space(-6.0);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let g = ui.painter().layout_no_wrap(version, fonts::list(10.5), theme::faint());
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(4.0, 0.0), Sense::click());
    let resp = resp.on_hover_text(tr("Über FadeWerk DJ")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if resp.hovered() { theme::dim() } else { theme::faint() };
    // Auf derselben Grundlinie wie „FADEWERK DJ“ (die Schriften sind verschieden groß)
    let top = (name_base - baseline(&g)).round();
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 2.0, top), g, color);
    if resp.clicked() {
        out.push(StatusAction::About);
    }
}

/// Titel und „als Nächstes“ behalten im Display mindestens so viel Breite; sonst fallen Systemwerte weg (die Uhr zuletzt).
const ON_AIR_MIN_W: f32 = 320.0;
/// Akku: grün ab `BATTERY_ORANGE` %, darunter orange, unter `BATTERY_RED` rot, unter `BATTERY_BLINK` blinkend (ohne Netz).
const BATTERY_ORANGE: f32 = 40.0;
const BATTERY_RED: f32 = 20.0;
const BATTERY_BLINK: f32 = 10.0;

/// Uhr ganz rechts im Display, davor Akku, Temperatur, Arbeitsspeicher, CPU (von `right` nach links), höchstens
/// `budget` breit. Liefert die genutzte Breite (0 = nichts gezeigt).
fn sys_values(ui: &egui::Ui, p: &egui::Painter, right: f32, y: f32, budget: f32, s: &SysLine) -> f32 {
    let ink = theme::well_ink();
    let gap = 9.0;
    let mut x = right;
    let mut n = 0;
    // Nächsten Platz links vom bisherigen holen, wenn er noch ins Budget passt; mit Tooltip
    let mut place = |w: f32, tip: &str| -> Option<Rect> {
        let left = x - w;
        if right - left > budget {
            return None;
        }
        let r = Rect::from_min_max(pos2(left, y - 10.0), pos2(x, y + 10.0));
        ui.interact(r, ui.id().with(("sys", n)), Sense::hover()).on_hover_text(tip);
        n += 1;
        x = left - gap;
        Some(r)
    };
    if let Some(clock) = &s.clock {
        let g = p.layout_no_wrap(clock.clone(), fonts::bold(13.5), ink);
        if let Some(r) = place(g.size().x, tr("Uhrzeit")) {
            p.galley(pos2(r.left(), y - g.size().y / 2.0), g, ink);
        }
    }
    if let Some(b) = s.battery {
        // Farbe nach Ladestand; ganz leer und nicht am Netz blinkt es
        let color = match b.percent {
            p if p >= BATTERY_ORANGE => theme::play(),
            p if p >= BATTERY_RED => theme::orange(),
            _ => theme::cue(),
        };
        let blink = !b.plugged && b.percent < BATTERY_BLINK;
        if blink {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let shown = !blink || ui.input(|i| i.time) % 1.0 < 0.6;
        let g = p.layout_no_wrap(format!("{:.0}%", b.percent), fonts::bold(11.0), color);
        let icon = vec2(16.0, 8.0);
        // Am Netz: kleiner Blitz vor dem Akku
        let bolt_w = if b.plugged { 7.0 } else { 0.0 };
        let pct = format!("{:.0}", b.percent);
        let tip = if b.plugged { trf("Akku {} %, am Netz", &[&pct]) } else { trf("Akku {} %", &[&pct]) };
        if let Some(r) = place(bolt_w + icon.x + 3.0 + g.size().x, &tip)
            && shown
        {
            if b.plugged {
                let (bx, s) = (r.left() + 2.5, 1.0);
                let pts = [pos2(bx + 1.5 * s, y - 5.0), pos2(bx - 1.5 * s, y + 0.5), pos2(bx + 0.5, y + 0.5), pos2(bx - 1.0, y + 5.0), pos2(bx + 2.5, y - 1.0), pos2(bx + 0.3, y - 1.0)];
                p.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, color)));
            }
            // Akku selbst gezeichnet: Gehäuse, Pol, Füllung nach Ladestand
            let body = Rect::from_min_size(pos2(r.left() + bolt_w, y - icon.y / 2.0), vec2(icon.x - 2.0, icon.y));
            p.rect_stroke(body, 1.5, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_size(pos2(body.right(), y - 1.5), vec2(2.0, 3.0)), 0.5, color);
            let inner = body.shrink(2.0);
            p.rect_filled(Rect::from_min_size(inner.min, vec2(inner.width() * b.percent / 100.0, inner.height())), 1.0, color);
            p.galley(pos2(body.right() + 5.0, y - g.size().y / 2.0), g, color);
        }
    }
    // Kleine Beschriftung und Wert auf einer Grundlinie
    let mut value = |label: &str, text: String, color: Color32, tip: &str| {
        let lg = p.layout_no_wrap(label.to_owned(), fonts::bold(9.0), ink.gamma_multiply(0.45));
        let vg = p.layout_no_wrap(text, fonts::bold(11.0), color);
        if let Some(r) = place(lg.size().x + 3.0 + vg.size().x, tip) {
            let base = y + vg.size().y / 2.0 - (vg.size().y - baseline(&vg));
            p.galley(pos2(r.left(), base - baseline(&lg)), lg, ink);
            p.galley(pos2(r.left() + r.width() - vg.size().x, base - baseline(&vg)), vg, color);
        }
    };
    let normal = ink.gamma_multiply(0.75);
    let level = |v: f32, warn: f32, alarm: f32| if v >= alarm { theme::cue() } else if v >= warn { theme::yellow() } else { normal };
    if let Some(t) = s.temp {
        value("TEMP", format!("{t:.0}°"), level(t, 80.0, 95.0), tr("Temperatur des Prozessors"));
    }
    if let Some(r) = s.ram {
        value("RAM", format!("{r:.0}%"), level(r, 85.0, 95.0), tr("Belegter Arbeitsspeicher des Rechners"));
    }
    if let Some((cpu, audio, dropout)) = s.cpu {
        let tip = trf("CPU-Last aller Kerne {} %. Audio-Ausgabe braucht {} % ihrer Zeit.", &[&format!("{cpu:.0}"), &format!("{:.0}", audio * 100.0)]);
        if dropout {
            value("CPU", tr("AUSSETZER").to_owned(), theme::cue(), &tip);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        } else {
            let color = if audio >= 0.7 { theme::yellow() } else { level(cpu, 80.0, 95.0) };
            value("CPU", format!("{cpu:.0}%"), color, &tip);
        }
    }
    if n == 0 { 0.0 } else { right - (x + gap) }
}

/// Abstand der Grundlinie der ersten Zeile von der Oberkante eines Textes.
fn baseline(g: &egui::Galley) -> f32 {
    g.rows.first().map_or(0.0, |r| r.pos.y + r.row.glyphs.first().map_or(0.0, |gl| gl.pos.y))
}

/// Display mit dem Titel auf Sendung, rechts darin das Nächste oder ein Hinweis.
fn on_air(ui: &mut egui::Ui, w: f32, m: &StatusModel) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, theme::CTRL_H + 2.0), Sense::hover());
    // Abgeschnittene Hinweise ganz lesen
    if let Some(text) = &m.notice {
        resp.on_hover_text(text);
    }
    let p = ui.painter().clone();
    let p = &p;
    hw::well(p, rect, 5.0);
    let ink = theme::well_ink();
    let mut inner = rect.shrink2(vec2(10.0, 0.0));
    let y = rect.center().y;
    // Ganz rechts Uhr und Systemwerte, durch eine feine Linie vom Rest getrennt
    let sys_w = sys_values(ui, p, inner.right(), y, (inner.width() - ON_AIR_MIN_W).max(0.0), &m.sys);
    if sys_w > 0.0 {
        let line_x = inner.right() - sys_w - 8.0;
        p.line_segment([pos2(line_x, rect.top() + 7.0), pos2(line_x, rect.bottom() - 7.0)], Stroke::new(1.0, ink.gamma_multiply(0.18)));
        inner.max.x = line_x - 8.0;
    }
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
