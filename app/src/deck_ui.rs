// SPDX-License-Identifier: GPL-3.0-or-later
//! Ansicht eines Players: Kopfzeile, Wellenform, Plattenteller, Transport, Pitch.
//!
//! Zeichnet nur und meldet Bedienungen als [`DeckAction`] zurück. Die Logik steckt in `app.rs`.

use std::f32::consts::TAU;

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, UiBuilder,
    epaint::TextShape, pos2, vec2,
};
use fw_library::Waveform;

use crate::i18n::{tr, trf};
use crate::theme;
use crate::{fonts, hw};

/// Sichtbarer Ausschnitt der Wellenform in Sekunden.
pub const WAVE_SPAN: f64 = 10.0;
/// Umdrehungen pro Sekunde bei 33⅓.
pub const REV_PER_SEC: f64 = 33.333 / 60.0;
pub const PITCH_RANGE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    Track,
    List,
}

impl Repeat {
    pub fn next(self) -> Self {
        match self {
            Repeat::Off => Repeat::Track,
            Repeat::Track => Repeat::List,
            Repeat::List => Repeat::Off,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Repeat::Off => tr("AUS"),
            Repeat::Track => tr("TITEL"),
            Repeat::List => tr("LISTE"),
        }
    }
}

/// Alles, was die Ansicht zum Zeichnen braucht.
pub struct DeckModel<'a> {
    pub deck: usize,
    pub title: &'a str,
    pub artist: &'a str,
    pub bpm: Option<f32>,
    pub pitch_pct: f32,
    /// Auto-Sync steuert gerade den Pitch (SYNC-Taste leuchtet).
    pub auto_synced: bool,
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
    pub speed: f32,
    pub has_track: bool,
    pub loading: bool,
    pub waveform: Option<&'a Waveform>,
    /// Cover aus den Tags; füllt dann fast die ganze Platte (Picture-Disc).
    pub cover: Option<egui::TextureId>,
    pub pfl: bool,
    pub repeat: Repeat,
    pub loop_beats: Option<u32>,
    pub loop_range: Option<(f64, f64)>,
    pub on_air: bool,
    pub scratching: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeckAction {
    TogglePlay,
    Cue,
    Stop,
    Prev,
    Next,
    Eject,
    CycleRepeat,
    Pfl(bool),
    Sync,
    Loop(u32),
    Pitch(f32),
    Seek(f64),
    ScratchBegin,
    /// Bewegung der Hand in Sekunden Titelzeit.
    ScratchBy(f64),
    ScratchEnd,
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0);
    format!("{}:{:02}", (s / 60.0).floor() as u64, (s % 60.0).floor() as u64)
}

thread_local! {
    /// Vergrößerung des gerade gezeichneten Players (1.0 bei Standard-Fenstergröße).
    static SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

fn k() -> f32 {
    SCALE.with(|s| s.get())
}

/// Vergrößerung für einen Player mit dieser Innenbreite. Bei der Standard-Fenstergröße ist ein Player
/// etwa 560 Punkte breit; auf großen Monitoren wächst alles nur maßvoll mit (bis zum 1,2-Fachen),
/// damit Tasten nicht klobig werden und die Playlists Höhe behalten.
pub fn deck_scale(content_width: f32) -> f32 {
    (content_width / 560.0).clamp(1.0, 1.2)
}

/// Größte Breite des Bedienfelds zwischen Teller und Pitch; überschüssiger Platz bleibt links und rechts frei.
const CONTROLS_MAX_W: f32 = 330.0;

/// Kleinste Vergrößerung: so weit schrumpft ein Player auf niedrigen Bildschirmen.
const MIN_SCALE: f32 = 0.75;

pub fn show(ui: &mut egui::Ui, m: &DeckModel<'_>, out: &mut Vec<DeckAction>) {
    // Nicht größer, als die Höhe erlaubt: bei 1,0 braucht der Inhalt etwa 390 Punkte
    let scale = deck_scale(ui.available_width()).min((ui.available_height() / 390.0).max(MIN_SCALE));
    SCALE.with(|s| s.set(scale));
    let color = theme::deck_color(m.deck);
    let mirrored = m.deck == 1;

    header(ui, m, color, out);
    ui.add_space(4.0);
    waveform(ui, m, color, out);
    overview(ui, m, color, out);
    ui.add_space(6.0);

    // Teller | Bedienung | Pitch, bei Player B gespiegelt
    let avail = ui.available_rect_before_wrap();
    let pitch_w = 54.0 * k();
    let plat = (avail.height().min(avail.width() * 0.42)).clamp(120.0, 240.0 * k());
    let (plat_rect, ctrl_rect, pitch_rect) = if !mirrored {
        let p = Rect::from_min_size(avail.min, vec2(plat, plat));
        let pr = Rect::from_min_max(pos2(avail.right() - pitch_w, avail.top()), pos2(avail.right(), avail.top() + plat));
        let c = Rect::from_min_max(pos2(p.right() + 12.0, avail.top()), pos2(pr.left() - 8.0, avail.top() + plat));
        (p, c, pr)
    } else {
        let p = Rect::from_min_size(pos2(avail.right() - plat, avail.top()), vec2(plat, plat));
        let pr = Rect::from_min_size(avail.min, vec2(pitch_w, plat));
        let c = Rect::from_min_max(pos2(pr.right() + 8.0, avail.top()), pos2(p.left() - 12.0, avail.top() + plat));
        (p, c, pr)
    };
    // Auf breiten Monitoren das Bedienfeld nicht in die Breite ziehen, sondern mittig setzen
    let max_w = CONTROLS_MAX_W * k();
    let ctrl_rect = if ctrl_rect.width() > max_w { Rect::from_center_size(ctrl_rect.center(), vec2(max_w, ctrl_rect.height())) } else { ctrl_rect };
    ui.allocate_rect(Rect::from_min_max(avail.min, pos2(avail.right(), avail.top() + plat)), Sense::hover());

    platter(ui, m, color, plat_rect, out);
    // Nach dem Teller: liegt obenauf und bekommt Klicks auf den Tonkopf zuerst
    tonearm(ui, m, plat_rect, mirrored, out);
    ui.scope_builder(UiBuilder::new().max_rect(ctrl_rect), |ui| controls(ui, m, color, out));
    ui.scope_builder(UiBuilder::new().max_rect(pitch_rect), |ui| pitch(ui, m, out));
}

/// Einzeiliger Text, der mit „…“ abgeschnitten wird, wenn er nicht in `width` passt.
fn text_ellipsized(p: &egui::Painter, pos: Pos2, text: &str, font: FontId, color: Color32, width: f32) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = p.layout_job(job);
    p.galley(pos, galley, color);
}

fn header(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, out: &mut Vec<DeckAction>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0 * k()), Sense::hover());
    let p = ui.painter().clone();

    // Deck-Buchstabe als beleuchtetes Schild
    let tag = Rect::from_min_size(rect.min, vec2(40.0, 40.0) * k());
    hw::paint_button(&p, tag, theme::deck_letter(m.deck), fonts::bold(26.0 * k()), hw::Look::Color(color), true, false, false, true);

    let eject_rect = Rect::from_min_size(pos2(rect.right() - 36.0 * k(), rect.top() + 2.0), vec2(36.0, 36.0) * k());
    let eject = ui.put(eject_rect, |ui: &mut egui::Ui| hw::button(ui, eject_rect.size(), "⏏", FontId::proportional(17.0 * k()), hw::Look::Plain, false, m.has_track));
    if m.has_track && eject.on_hover_text(tr("Auswerfen: Player leeren")).clicked() {
        out.push(DeckAction::Eject);
    }

    // BPM als LCD vor der Auswurftaste
    let lcd = Rect::from_min_max(pos2(eject_rect.left() - 104.0 * k(), rect.top()), pos2(eject_rect.left() - 10.0, rect.bottom()));
    hw::well(&p, lcd, 4.0);
    let bpm = m.bpm.map(|b| format!("{:5.1}", b * (1.0 + m.pitch_pct / 100.0))).unwrap_or_else(|| "  -.-".into());
    hw::lcd_text(&p, pos2(lcd.right() - 7.0, lcd.top() + 6.0), Align2::RIGHT_TOP, &bpm, 17.0 * k(), color);
    p.text(pos2(lcd.left() + 6.0, lcd.bottom() - 4.0), Align2::LEFT_BOTTOM, "BPM", fonts::bold(9.0 * k()), theme::well_ink().gamma_multiply(0.55));
    let mut text_right = lcd.left() - 10.0;

    if m.on_air {
        let badge = Rect::from_min_size(pos2(text_right - 56.0 * k(), rect.top() + 3.0), vec2(54.0, 17.0) * k());
        hw::paint_button(&p, badge, "ON AIR", fonts::bold(10.0 * k()), hw::Look::Color(theme::cue()), true, false, false, true);
        text_right = badge.left() - 10.0;
    }

    let left = tag.right() + 12.0;
    let width = text_right - left;
    let title = if m.loading { tr("Lädt …") } else if m.has_track { m.title } else { tr("Kein Titel geladen") };
    let sub = if m.has_track { m.artist } else { tr("Doppelklick in der Playlist lädt einen Titel") };
    text_ellipsized(&p, pos2(left, rect.top()), title, fonts::bold(17.0 * k()), theme::ink(), width);
    text_ellipsized(&p, pos2(left, rect.top() + 24.0 * k()), sub, FontId::proportional(13.5 * k()), theme::dim(), width);
}

fn waveform(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, out: &mut Vec<DeckAction>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0 * k()), Sense::click_and_drag());
    hw::well(ui.painter(), rect, 5.0);
    let p = ui.painter_at(rect.shrink(1.0));
    let Some(wf) = m.waveform.filter(|_| m.has_track) else {
        p.text(rect.center(), Align2::CENTER_CENTER, if m.loading { tr("Lädt …") } else { tr("Kein Titel geladen") }, FontId::proportional(12.0), theme::faint());
        return;
    };
    let t0 = m.position - WAVE_SPAN / 2.0;
    let mid = rect.center().y;
    let to_x = |t: f64| rect.left() + ((t - t0) / WAVE_SPAN) as f32 * rect.width();

    if let Some((a, b)) = m.loop_range {
        p.rect_filled(Rect::from_min_max(pos2(to_x(a), rect.top()), pos2(to_x(b), rect.bottom())), 0.0, theme::play().gamma_multiply(0.15));
    }
    if let Some(bpm) = m.bpm {
        let beat = 60.0 / bpm as f64;
        let first = (t0 / beat).ceil() as i64;
        let last = ((t0 + WAVE_SPAN) / beat).floor() as i64;
        for b in first..=last {
            let x = to_x(b as f64 * beat);
            let w = if b % 4 == 0 { 1.5 } else { 0.75 };
            p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(w, Color32::from_white_alpha(14)));
        }
    }
    let cols = (rect.width() / 2.0) as usize;
    wave_mesh(&p, rect.left(), mid, rect.width() / cols as f32, cols, |i| {
        let t = t0 + (i as f64 / cols as f64) * WAVE_SPAN;
        if t < 0.0 || t > m.duration {
            return None;
        }
        let v = wf.max_in(t, t + WAVE_SPAN / cols as f64) * (rect.height() * 0.47);
        let c = if t < m.position { Color32::from_rgba_unmultiplied(230, 235, 245, 70) } else { color };
        Some((v, c))
    });
    p.line_segment([pos2(rect.center().x, rect.top()), pos2(rect.center().x, rect.bottom())], Stroke::new(2.0, Color32::WHITE));

    // Festhalten = anhalten, ziehen = scratchen, tippen = Play/Pause
    if resp.drag_started() {
        out.push(DeckAction::ScratchBegin);
    }
    if resp.dragged() {
        let dx = resp.drag_delta().x as f64;
        if dx != 0.0 {
            out.push(DeckAction::ScratchBy(-dx / rect.width() as f64 * WAVE_SPAN));
        }
    }
    if resp.drag_stopped() {
        out.push(DeckAction::ScratchEnd);
    }
    if resp.clicked() {
        out.push(DeckAction::TogglePlay);
    }
    resp.on_hover_text(tr("Festhalten hält an, ziehen scratcht, tippen startet oder pausiert"));
}

/// Wellenform als geschlossene Fläche: je Spalte ein lückenloses Rechteck (Ausschlag `v` um `mid`).
/// Einzelne Striche ließen feine Lücken stehen, die wie ein Gitter wirkten.
fn wave_mesh(p: &egui::Painter, left: f32, mid: f32, col_w: f32, cols: usize, mut column: impl FnMut(usize) -> Option<(f32, Color32)>) {
    let mut mesh = egui::Mesh::default();
    for i in 0..cols {
        let Some((v, c)) = column(i) else { continue };
        let x = left + i as f32 * col_w;
        let v = v.max(0.5);
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, mid - v), pos2(x + col_w, mid + v)), c);
    }
    p.add(egui::Shape::mesh(mesh));
}

fn overview(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, out: &mut Vec<DeckAction>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0 * k().min(1.3)), Sense::click_and_drag());
    hw::well(ui.painter(), rect, 3.0);
    let p = ui.painter_at(rect.shrink(1.0));
    let (Some(wf), true) = (m.waveform, m.has_track && m.duration > 0.0) else { return };
    let frac = (m.position / m.duration) as f32;
    let cols = (rect.width() / 2.0) as usize;
    wave_mesh(&p, rect.left(), rect.center().y, rect.width() / cols as f32, cols, |i| {
        let f0 = i as f64 / cols as f64;
        let v = wf.max_in(f0 * m.duration, (f0 + 1.0 / cols as f64) * m.duration) * rect.height() * 0.45;
        let c = if (f0 as f32) < frac { color } else { Color32::from_rgba_unmultiplied(230, 235, 245, 60) };
        Some((v, c))
    });
    let x = rect.left() + frac * rect.width();
    p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, Color32::WHITE));
    if (resp.clicked() || resp.dragged()) && let Some(pos) = resp.interact_pointer_pos() {
        let f = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        out.push(DeckAction::Seek(f * m.duration));
    }
}

fn platter(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, rect: Rect, out: &mut Vec<DeckAction>) {
    let id = ui.id().with(("platter", m.deck));
    let resp = ui.interact(rect, id, Sense::drag());
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5 - 2.0;
    let angle = (m.position * REV_PER_SEC * TAU as f64 % TAU as f64) as f32;
    paint_platter(ui, c, r, angle, color, m, resp.dragged());

    let pointer_angle = |p: Pos2| (p.y - c.y).atan2(p.x - c.x);
    if resp.drag_started() && m.has_track {
        out.push(DeckAction::ScratchBegin);
        if let Some(p) = resp.interact_pointer_pos() {
            ui.data_mut(|d| d.insert_temp(id, pointer_angle(p)));
        }
    }
    if resp.dragged() && m.has_track && let Some(p) = resp.interact_pointer_pos() {
        let a = pointer_angle(p);
        let last: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(a);
        let mut da = a - last;
        if da > std::f32::consts::PI { da -= TAU; }
        if da < -std::f32::consts::PI { da += TAU; }
        ui.data_mut(|d| d.insert_temp(id, a));
        if da != 0.0 {
            out.push(DeckAction::ScratchBy(da as f64 / TAU as f64 / REV_PER_SEC));
        }
    }
    if resp.drag_stopped() {
        out.push(DeckAction::ScratchEnd);
    }
    if m.has_track {
        resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tr("Teller ziehen = scratchen"));
    }
}

fn paint_platter(ui: &egui::Ui, c: Pos2, r: f32, angle: f32, color: Color32, m: &DeckModel<'_>, grabbing: bool) {
    let p = ui.painter();
    let rot = |a: f32, rr: f32| c + vec2((a + angle).cos(), (a + angle).sin()) * rr;

    // Schatten unter dem Teller, dann Metallrand mit Glanz oben links und Stroboskop-Punkten
    for k in 0..6 {
        p.circle_filled(c + vec2(0.0, 5.0), r + 4.0 - k as f32 * 1.5, Color32::from_black_alpha(18));
    }
    p.circle_filled(c, r, Color32::from_rgb(0x2c, 0x34, 0x46));
    let arc = |from: f32, to: f32, rr: f32, stroke: Stroke| {
        let pts: Vec<Pos2> = (0..=24).map(|i| { let t = from + (to - from) * i as f32 / 24.0; c + vec2(t.cos(), t.sin()) * rr }).collect();
        p.add(egui::Shape::line(pts, stroke));
    };
    arc(3.4, 5.4, r - 1.5, Stroke::new(2.5, Color32::from_white_alpha(60)));
    arc(0.3, 2.3, r - 1.5, Stroke::new(2.5, Color32::from_black_alpha(90)));
    p.circle_stroke(c, r * 0.915, Stroke::new(2.0, Color32::from_black_alpha(140)));
    for i in 0..72 {
        p.circle_filled(rot(i as f32 / 72.0 * TAU, r * 0.955), (r * 0.012).max(1.0), Color32::from_rgb(0xcf, 0xd6, 0xe4));
    }
    // Vinyl mit Rillen; mit Cover bleibt nur ein schmaler Rillenrand außen (Picture-Disc)
    let cover = m.cover.filter(|_| m.has_track);
    p.circle_filled(c, r * 0.91, Color32::from_rgb(0x0b, 0x0b, 0x0e));
    if let Some(tex) = cover {
        paint_cover(p, c, r * COVER_RADIUS, angle, tex);
        p.circle_stroke(c, r * COVER_RADIUS, Stroke::new((r * 0.008).max(1.0), Color32::from_black_alpha(200)));
    }
    let mut rr = if cover.is_some() { r * (COVER_RADIUS + 0.012) } else { r * 0.36 };
    let mut k = 0;
    while rr < r * 0.89 {
        p.circle_stroke(c, rr, Stroke::new(1.0, Color32::from_white_alpha(if k % 3 == 0 { 12 } else { 6 })));
        rr += r * 0.022;
        k += 1;
    }
    // Lichtreflex steht still, während die Platte darunter dreht
    for (a, alpha) in [(-2.4f32, 16u8), (0.7, 10)] {
        let pts: Vec<Pos2> = (0..=12)
            .map(|i| { let t = a - 0.35 + i as f32 / 12.0 * 0.7; c + vec2(t.cos(), t.sin()) * r * 0.86 })
            .chain((0..=12).rev().map(|i| { let t = a - 0.25 + i as f32 / 12.0 * 0.5; c + vec2(t.cos(), t.sin()) * r * 0.4 }))
            .collect();
        p.add(egui::Shape::convex_polygon(pts, Color32::from_white_alpha(alpha), Stroke::NONE));
    }
    // Label mit Titel, dreht mit (entfällt, wenn das Cover die Platte füllt)
    let label_color = if m.has_track { color } else { theme::panel2() };
    if cover.is_none() {
        p.circle_filled(c, r * 0.33, label_color);
    }
    let text_color = Color32::from_rgba_unmultiplied(10, 14, 22, 220);
    let label_text = |text: &str, size: f32, dy: f32| {
        let mut t: String = text.to_uppercase();
        if t.chars().count() > 18 {
            t = t.chars().take(17).collect::<String>() + "…";
        }
        let galley = p.layout_no_wrap(t, FontId::proportional(size), text_color);
        let anchor = c + egui::emath::Rot2::from_angle(angle) * vec2(0.0, dy);
        let pos = anchor - galley.size() / 2.0;
        p.add(TextShape::new(pos, galley, text_color).with_angle_and_anchor(angle, Align2::CENTER_CENTER));
    };
    if cover.is_some() {
        // Mittelloch der Picture-Disc
        p.circle_filled(c, r * 0.05, Color32::from_black_alpha(150));
    } else if m.has_track {
        label_text(m.title, (r * 0.085).max(8.0), -r * 0.1);
        label_text(m.artist, (r * 0.065).max(7.0), r * 0.1);
    } else {
        // Zweizeilig, damit das Mittelloch nichts verdeckt
        label_text("FADEWERK", (r * 0.08).max(8.0), -r * 0.1);
        label_text("DJ", (r * 0.08).max(8.0), r * 0.1);
    }
    // Markierung, damit die Drehung sichtbar ist
    let a = -std::f32::consts::FRAC_PI_2;
    let mark_from = if cover.is_some() { COVER_RADIUS + 0.012 } else { 0.4 };
    p.line_segment([rot(a, r * mark_from), rot(a, r * 0.88)], Stroke::new((r * 0.02).max(1.5), Color32::from_white_alpha(230)));
    p.circle_filled(c, r * 0.03, Color32::from_rgb(0xcf, 0xd6, 0xe4));
    if grabbing || m.scratching {
        p.circle_stroke(c, r + 1.0, Stroke::new(2.5, color));
    }
}

/// Tonarm, in Tellerradien: Abstand Drehpunkt–Tellermitte, Länge Drehpunkt–Nadel.
const ARM_PIVOT: f32 = 1.08;
const ARM_LEN: f32 = 1.0;
/// Nadelabstand von der Mitte: äußere Rille (Titelanfang), innere Rille (Titelende), Ruhe auf der Stütze.
const GROOVE_OUT: f32 = 0.88;
const GROOVE_IN: f32 = 0.38;
const ARM_REST: f32 = 1.0;
/// Dauer des Schwenks auf die Platte und zurück auf die Stütze.
const ARM_SWING_SECS: f32 = 0.6;
const ARM_RETURN_SECS: f32 = 0.25;

/// Richtung des Arms (Bogenmaß, Player A) für einen Nadelabstand `rho` (in Tellerradien) von der Mitte.
/// Der Drehpunkt liegt oben rechts neben dem Teller; die Nadel liegt auf dem Kreis um ihn.
fn arm_angle(rho: f32) -> f32 {
    let toward_pivot = -std::f32::consts::FRAC_PI_4;
    let cos = ((rho * rho - ARM_PIVOT * ARM_PIVOT - ARM_LEN * ARM_LEN) / (2.0 * ARM_PIVOT * ARM_LEN)).clamp(-1.0, 1.0);
    toward_pivot + cos.acos()
}

/// Nadelabstand (in Tellerradien) für eine Stelle im Titel: Anfang außen, Ende innen.
fn groove_at(frac: f32) -> f32 {
    GROOVE_OUT - (GROOVE_OUT - GROOVE_IN) * frac.clamp(0.0, 1.0)
}

/// Tonarm neben dem Teller (bei Player B gespiegelt). Steht der Player, liegt er auf seiner Stütze; bei Play
/// schwenkt er auf die Platte und senkt sich, die Nadel wandert mit dem Titel von außen nach innen. Bei Pause
/// hebt er sich über der Stelle. Den Tonkopf ziehen springt an die Stelle, an der die Nadel losgelassen wird.
fn tonearm(ui: &mut egui::Ui, m: &DeckModel<'_>, rect: Rect, mirrored: bool, out: &mut Vec<DeckAction>) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5 - 2.0;
    let sx = if mirrored { -1.0 } else { 1.0 };
    let id = ui.id().with(("tonearm", m.deck));
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);

    let frac = if m.duration > 0.0 { (m.position / m.duration) as f32 } else { 0.0 };
    let on_record = m.has_track && (m.playing || m.speed.abs() > 0.01 || m.scratching);
    let paused = m.has_track && !on_record && m.position > 0.5;
    let mut target = if on_record || paused { groove_at(frac) } else { ARM_REST };

    let pivot = c + vec2(sx, -1.0) * (ARM_PIVOT * r * std::f32::consts::FRAC_1_SQRT_2);
    let dir = |rho: f32| {
        let a = arm_angle(rho);
        vec2(sx * a.cos(), a.sin())
    };

    // Tonkopf greifen: Fläche um die Nadel aus dem letzten Bild
    let shown: f32 = ui.data(|d| d.get_temp(id.with("shown"))).unwrap_or(target);
    let head = pivot + dir(shown) * ARM_LEN * r;
    let grab = Rect::from_center_size(head, vec2(0.3, 0.3) * r);
    let resp = ui.interact(grab, id, if m.has_track && m.duration > 0.0 { Sense::drag() } else { Sense::hover() });
    let mut dragging = None;
    if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        dragging = Some(((pos - c).length() / r).clamp(GROOVE_IN, GROOVE_OUT));
    }
    if resp.drag_stopped()
        && let Some(rho) = ui.data(|d| d.get_temp::<f32>(id.with("drag")))
    {
        let f = (GROOVE_OUT - rho) / (GROOVE_OUT - GROOVE_IN);
        out.push(DeckAction::Seek(f as f64 * m.duration));
        // Bis die Engine die neue Stelle meldet, bleibt der Arm dort, statt kurz zurückzuschwingen
        ui.data_mut(|d| d.insert_temp(id.with("settle"), (rho, now + 0.5)));
    }
    if let Some(rho) = dragging {
        ui.data_mut(|d| d.insert_temp(id.with("drag"), rho));
    }
    if let Some((rho, until)) = ui.data(|d| d.get_temp::<(f32, f64)>(id.with("settle"))) {
        if now < until && (target - rho).abs() > 0.01 && dragging.is_none() {
            target = rho;
        } else {
            ui.data_mut(|d| d.remove::<(f32, f64)>(id.with("settle")));
        }
    }
    if m.has_track && m.duration > 0.0 {
        resp.clone().on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tr("Tonarm ziehen = an diese Stelle springen"));
    }

    // Schwenk und Heben weich, beim Ziehen folgt der Arm der Hand
    let rho = match dragging {
        Some(d) => ctx.animate_value_with_time(id.with("rho"), d, 0.0),
        // Zurück auf die Stütze geht es zügig, das Aufsetzen auf die Platte gemächlich
        None => ctx.animate_value_with_time(id.with("rho"), target, if target == ARM_REST { ARM_RETURN_SECS } else { ARM_SWING_SECS }),
    };
    let lift = ctx.animate_value_with_time(id.with("lift"), if on_record && dragging.is_none() { 0.0 } else { 1.0 }, 0.25);
    ui.data_mut(|d| d.insert_temp(id.with("shown"), rho));

    let p = ui.painter();
    let u = dir(rho);
    let n = vec2(-u.y, u.x);
    let stylus = pivot + u * ARM_LEN * r;
    let elbow = pivot + u * ARM_LEN * r * 0.8;
    let rest_post = pivot + dir(ARM_REST) * ARM_LEN * r * 0.72;
    let metal = Color32::from_rgb(0xc9, 0xcf, 0xd9);
    let metal_dark = Color32::from_rgb(0x5e, 0x66, 0x74);
    let tube_w = (r * 0.04).max(2.5);
    let head_w = (r * 0.085).max(5.0) * (1.0 + 0.08 * lift);

    // Stütze, auf der der Arm in Ruhe liegt
    p.circle_filled(rest_post + vec2(1.0, 2.0), r * 0.045, Color32::from_black_alpha(90));
    p.circle_filled(rest_post, r * 0.04, Color32::from_rgb(0x1c, 0x21, 0x2b));
    p.circle_stroke(rest_post, r * 0.04, Stroke::new(1.0, metal_dark));

    // Schatten: angehoben fällt er weiter weg
    let sh = vec2(1.0, 1.4) * (r * 0.02 + r * 0.05 * lift);
    let shadow = Color32::from_black_alpha(70);
    p.line_segment([pivot + sh, elbow + sh], Stroke::new(tube_w + 2.0, shadow));
    p.line_segment([elbow + sh, stylus + sh], Stroke::new(head_w + 2.0, shadow));

    // Sockel mit Lager
    p.circle_filled(pivot + vec2(0.0, 3.0), r * 0.15, Color32::from_black_alpha(80));
    p.circle_filled(pivot, r * 0.14, Color32::from_rgb(0x23, 0x29, 0x35));
    p.circle_stroke(pivot, r * 0.14, Stroke::new(1.5, metal_dark));
    p.circle_filled(pivot, r * 0.085, Color32::from_rgb(0x39, 0x41, 0x50));

    // Gegengewicht hinter dem Lager
    let w0 = pivot - u * r * 0.1;
    let w1 = pivot - u * r * 0.28;
    p.line_segment([w0, w1], Stroke::new(r * 0.13, metal_dark));
    p.line_segment([w0 - n * r * 0.03, w1 - n * r * 0.03], Stroke::new(r * 0.035, metal.gamma_multiply(0.6)));

    // Rohr mit Glanzkante
    p.line_segment([pivot, elbow], Stroke::new(tube_w + 1.5, metal_dark));
    p.line_segment([pivot, elbow], Stroke::new(tube_w, metal));
    p.line_segment([pivot - n * tube_w * 0.2, elbow - n * tube_w * 0.2], Stroke::new(tube_w * 0.3, Color32::from_white_alpha(170)));

    // Tonkopf mit Griff; die Nadel sitzt vorn
    p.line_segment([elbow, stylus], Stroke::new(head_w + 1.5, Color32::from_rgb(0x10, 0x13, 0x19)));
    p.line_segment([elbow, stylus], Stroke::new(head_w, Color32::from_rgb(0x2a, 0x30, 0x3c)));
    let finger = elbow + u * (stylus - elbow).length() * 0.35;
    p.line_segment([finger, finger + n * sx * r * 0.09], Stroke::new((r * 0.022).max(1.5), metal));
    p.circle_filled(stylus - u * r * 0.03, (r * 0.018).max(1.2), metal);
    p.circle_filled(pivot, r * 0.04, metal);
    if resp.hovered() || dragging.is_some() {
        p.circle_stroke(stylus - u * r * 0.06, r * 0.11, Stroke::new(1.5, theme::deck_color(m.deck).gamma_multiply(0.8)));
    }
}

/// Anteil des Tellerradius, den ein Cover bedeckt. Außen bleibt ein schmaler Rillenrand.
const COVER_RADIUS: f32 = 0.82;

/// Zeichnet das Cover als runde Scheibe, die mit der Platte dreht.
fn paint_cover(p: &egui::Painter, c: Pos2, radius: f32, angle: f32, tex: egui::TextureId) {
    const SEGMENTS: u32 = 96;
    let mut mesh = egui::Mesh::with_texture(tex);
    mesh.vertices.push(egui::epaint::Vertex { pos: c, uv: pos2(0.5, 0.5), color: Color32::WHITE });
    for i in 0..=SEGMENTS {
        // Bildwinkel φ erscheint auf dem Bildschirm bei φ + angle
        let phi = i as f32 / SEGMENTS as f32 * TAU;
        let t = phi + angle;
        mesh.vertices.push(egui::epaint::Vertex {
            pos: c + vec2(t.cos(), t.sin()) * radius,
            uv: pos2(0.5 + 0.5 * phi.cos(), 0.5 + 0.5 * phi.sin()),
            color: Color32::WHITE,
        });
        if i > 0 {
            mesh.add_triangle(0, i, i + 1);
        }
    }
    p.add(mesh);
}

fn controls(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, out: &mut Vec<DeckAction>) {
    let w = ui.available_width();
    let gap = 6.0;
    let enabled = m.has_track;

    // Zeitanzeigen als LCD
    ui.horizontal(|ui| {
        let half = (w - gap) / 2.0;
        time_box(ui, half, tr("GESPIELT"), &fmt_time(m.position), theme::well_ink());
        time_box(ui, half, tr("REST"), &format!("-{}", fmt_time(m.duration - m.position)), color);
    });

    // CUE und Play/Pause: große, beleuchtete Taster
    ui.horizontal(|ui| {
        let cue_w = (w - gap) * 0.43;
        if hw::button(ui, vec2(cue_w, 52.0 * k()), "CUE", fonts::bold(21.0 * k()), hw::Look::Color(theme::cue()), false, enabled).clicked() {
            out.push(DeckAction::Cue);
        }
        let spinning_down = !m.playing && m.speed > 0.0;
        // Beim Auslaufen blinkt die Play-Taste
        let blink = spinning_down && (ui.input(|i| i.time) * 4.0) as i64 % 2 == 0;
        if hw::play_pause_button(ui, vec2(w - gap - cue_w, 52.0 * k()), 17.0 * k(), hw::Look::Color(theme::play()), m.playing || blink, enabled).clicked() {
            out.push(DeckAction::TogglePlay);
        }
    });

    // Zurück, Stopp, Vor, Repeat
    ui.horizontal(|ui| {
        let unit = (w - 3.0 * gap) / 4.7;
        let icon = FontId::proportional(15.0 * k());
        if hw::button(ui, vec2(unit, 32.0 * k()), "⏮", icon.clone(), hw::Look::Plain, false, enabled).on_hover_text(tr("Zurück: Titelanfang, in den ersten 3 s voriger Titel")).clicked() {
            out.push(DeckAction::Prev);
        }
        if hw::button(ui, vec2(unit, 32.0 * k()), "⏹", icon.clone(), hw::Look::Plain, false, enabled).on_hover_text(tr("Stopp: Teller läuft aus, dann zurück zum Cue-Punkt")).clicked() {
            out.push(DeckAction::Stop);
        }
        if hw::button(ui, vec2(unit, 32.0 * k()), "⏭", icon, hw::Look::Plain, false, enabled).on_hover_text(tr("Nächster Titel der Playlist")).clicked() {
            out.push(DeckAction::Next);
        }
        let on = m.repeat != Repeat::Off;
        let label = format!("🔁 {}", m.repeat.label());
        if hw::button(ui, vec2(w - 3.0 * gap - 3.0 * unit, 32.0 * k()), &label, fonts::bold(12.0 * k()), hw::Look::Color(color), on, true)
            .on_hover_text(tr("Wiederholen: aus, Titel, ganze Playlist"))
            .clicked()
        {
            out.push(DeckAction::CycleRepeat);
        }
    });

    // Vorhören, Sync
    ui.horizontal(|ui| {
        let pfl_w = (w - gap) * 0.6;
        let label = format!("🎧 {}", tr("VORHÖREN"));
        if hw::button(ui, vec2(pfl_w, 28.0 * k()), &label, fonts::bold(12.0 * k()), hw::Look::Color(color), m.pfl, true)
            .on_hover_text(tr("Auf dem Kopfhörer vorhören, unabhängig vom Crossfader"))
            .clicked()
        {
            out.push(DeckAction::Pfl(!m.pfl));
        }
        let tip = if m.auto_synced { tr("Auto-Sync steuert den Pitch; Pitch anfassen übernimmt von Hand") } else { tr("Tempo an den anderen Player angleichen") };
        if hw::button(ui, vec2(w - gap - pfl_w, 28.0 * k()), "SYNC", fonts::bold(12.0 * k()), hw::Look::Color(theme::yellow()), m.auto_synced, enabled && m.bpm.is_some())
            .on_hover_text(tip)
            .clicked()
        {
            out.push(DeckAction::Sync);
        }
    });

    // Loops
    ui.horizontal(|ui| {
        ui.label(RichText::new("LOOP").font(fonts::bold(11.0 * k())).color(theme::dim()));
        let rest = ui.available_width();
        let bw = (rest - 2.0 * gap) / 3.0;
        for beats in [4u32, 8, 16] {
            let on = m.loop_beats == Some(beats);
            if hw::button(ui, vec2(bw, 26.0 * k()), &beats.to_string(), fonts::bold(12.0 * k()), hw::Look::Color(theme::play()), on, enabled && m.bpm.is_some())
                .on_hover_text(trf("Loop über {} Beats", &[&beats]))
                .clicked()
            {
                out.push(DeckAction::Loop(beats));
            }
        }
    });
}

fn time_box(ui: &mut egui::Ui, width: f32, label: &str, value: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 46.0 * k()), Sense::hover());
    let p = ui.painter();
    hw::well(p, rect, 5.0);
    p.text(rect.left_top() + vec2(8.0, 5.0), Align2::LEFT_TOP, label, fonts::bold(9.5 * k()), theme::well_ink().gamma_multiply(0.55));
    hw::lcd_text(p, rect.right_bottom() + vec2(-8.0, -6.0), Align2::RIGHT_BOTTOM, value, 17.0 * k(), color);
}

fn pitch(ui: &mut egui::Ui, m: &DeckModel<'_>, out: &mut Vec<DeckAction>) {
    let area = ui.max_rect();
    // Oben = schneller wie an Plattenspielern mit „+“ oben
    let value = (m.pitch_pct + PITCH_RANGE) / (2.0 * PITCH_RANGE);
    // Rechtsklick irgendwo auf den Pitch-Bereich lässt ihn langsam auf 0 laufen (über den Fader, damit es
    // nicht hörbar springt). Liegt unter dem Fader, damit dieser bedienbar bleibt.
    if ui.interact(area, ui.id().with(("pitch-reset", m.deck)), Sense::click()).secondary_clicked() {
        hw::start_glide(ui, ("pitch", m.deck), value, 0.5, 1.0);
    }
    let p = ui.painter().clone();
    p.text(pos2(area.center().x, area.top()), Align2::CENTER_TOP, "PITCH", fonts::bold(10.0 * k()), theme::dim());
    let fader = Rect::from_min_max(pos2(area.left() + 4.0, area.top() + 18.0), pos2(area.right() - 4.0, area.bottom() - 26.0));
    // Grundwert 0,5 = Mitte = 0 %
    let (resp, new) = hw::vertical_fader(ui, fader, ("pitch", m.deck), value, 0.5, true);
    resp.on_hover_text(tr("Rechtsklick oder Doppelklick setzt auf 0"));
    if let Some(v) = new {
        let pct = ((v * 2.0 - 1.0) * PITCH_RANGE * 10.0).round() / 10.0;
        out.push(DeckAction::Pitch(pct));
    }
    let lcd = Rect::from_min_max(pos2(area.left(), area.bottom() - 20.0), pos2(area.right(), area.bottom()));
    hw::well(&p, lcd, 3.0);
    let sign = if m.pitch_pct > 0.0 { "+" } else if m.pitch_pct < 0.0 { "-" } else { " " };
    hw::lcd_text(&p, lcd.center(), Align2::CENTER_CENTER, &format!("{sign}{:.1}", m.pitch_pct.abs()), 10.5, theme::well_ink());
}
