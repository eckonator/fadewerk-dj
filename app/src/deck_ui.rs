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
    /// Klingende Tonart (mit Pitch verschoben, solange kein Keylock) und die Verschiebung in Halbtönen.
    pub key: Option<fw_library::Camelot>,
    pub key_shift: i32,
    /// Passt harmonisch zum Titel, der gerade auf Sendung ist (Display zeigt „PASST“).
    pub key_fits: bool,
    pub pitch_pct: f32,
    /// Filter, −1 Tiefpass bis +1 Hochpass, 0 = aus.
    pub filter: f32,
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
    /// Läuft rückwärts (REV-Taste).
    pub reverse: bool,
    pub loop_beats: Option<u32>,
    pub loop_range: Option<(f64, f64)>,
    pub on_air: bool,
    pub scratching: bool,
    /// Plattenteller und Tonarm zeigen (Einstellung); ohne sie bekommt das Bedienfeld den Platz.
    pub platter: bool,
    /// Gerade ausgeworfene Platte, die noch aus dem Player gleitet.
    pub leaving: Option<Leaving<'a>>,
}

/// Label bzw. Cover einer ausgeworfenen Platte, damit sie beim Hinausgleiten noch so aussieht wie vorher.
pub struct Leaving<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub cover: Option<egui::TextureId>,
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
    Reverse(bool),
    Pfl(bool),
    Sync,
    Loop(u32),
    Pitch(f32),
    Filter(f32),
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
/// Ohne Plattenteller darf das Bedienfeld breiter werden.
const CONTROLS_MAX_W_NO_PLATTER: f32 = 440.0;

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

    if !m.platter {
        // Ohne Teller: Bedienung | Pitch, Pitch außen (bei B links), Bedienfeld mittig und etwas breiter
        let avail = ui.available_rect_before_wrap();
        let pitch_w = 54.0 * k();
        let h = avail.height().clamp(120.0, 240.0 * k());
        let (ctrl_rect, pitch_rect) = if !mirrored {
            let pr = Rect::from_min_max(pos2(avail.right() - pitch_w, avail.top()), pos2(avail.right(), avail.top() + h));
            (Rect::from_min_max(avail.min, pos2(pr.left() - 12.0, avail.top() + h)), pr)
        } else {
            let pr = Rect::from_min_size(avail.min, vec2(pitch_w, h));
            (Rect::from_min_max(pos2(pr.right() + 12.0, avail.top()), pos2(avail.right(), avail.top() + h)), pr)
        };
        let max_w = CONTROLS_MAX_W_NO_PLATTER * k();
        let ctrl_rect = if ctrl_rect.width() > max_w { Rect::from_center_size(ctrl_rect.center(), vec2(max_w, ctrl_rect.height())) } else { ctrl_rect };
        let pitch_rect = centered_pitch(pitch_rect, ctrl_rect, avail, mirrored);
        ui.allocate_rect(Rect::from_min_max(avail.min, pos2(avail.right(), avail.top() + h)), Sense::hover());
        ui.scope_builder(UiBuilder::new().max_rect(ctrl_rect), |ui| controls(ui, m, color, out));
        ui.scope_builder(UiBuilder::new().max_rect(pitch_rect), |ui| pitch(ui, m, out));
        return;
    }

    // Teller | Bedienung | Pitch, bei Player B gespiegelt
    let avail = ui.available_rect_before_wrap();
    let pitch_w = 54.0 * k();
    let plat = (avail.height().min(avail.width() * 0.42)).clamp(120.0, 240.0 * k());
    // Ist der Teller durch die Breite begrenzt, nutzen Bedienfeld und Pitch trotzdem die verfügbare Höhe
    // (bis zur vollen Tellerhöhe bei großem Player); der Teller steht dann mittig daneben
    let row = avail.height().min(240.0 * k()).max(plat);
    let plat_top = avail.top() + (row - plat) / 2.0;
    let (plat_rect, ctrl_rect, pitch_rect) = if !mirrored {
        let p = Rect::from_min_size(pos2(avail.left(), plat_top), vec2(plat, plat));
        let pr = Rect::from_min_max(pos2(avail.right() - pitch_w, avail.top()), pos2(avail.right(), avail.top() + row));
        let c = Rect::from_min_max(pos2(p.right() + 12.0, avail.top()), pos2(pr.left() - 8.0, avail.top() + row));
        (p, c, pr)
    } else {
        let p = Rect::from_min_size(pos2(avail.right() - plat, plat_top), vec2(plat, plat));
        let pr = Rect::from_min_size(avail.min, vec2(pitch_w, row));
        let c = Rect::from_min_max(pos2(pr.right() + 8.0, avail.top()), pos2(p.left() - 12.0, avail.top() + row));
        (p, c, pr)
    };
    // Auf breiten Monitoren das Bedienfeld nicht in die Breite ziehen, sondern mittig setzen
    let max_w = CONTROLS_MAX_W * k();
    let ctrl_rect = if ctrl_rect.width() > max_w { Rect::from_center_size(ctrl_rect.center(), vec2(max_w, ctrl_rect.height())) } else { ctrl_rect };
    let pitch_rect = centered_pitch(pitch_rect, ctrl_rect, avail, mirrored);
    ui.allocate_rect(Rect::from_min_max(avail.min, pos2(avail.right(), avail.top() + row)), Sense::hover());

    platter(ui, m, color, plat_rect, out);
    // Nach dem Teller: liegt obenauf und bekommt Klicks auf den Tonkopf zuerst
    tonearm(ui, m, plat_rect, out);
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
    // Tonart als zweites Display; „PASST“ in Grün, wenn sie zum Titel auf Sendung passt
    let klcd = Rect::from_min_max(pos2(lcd.left() - 62.0 * k(), rect.top()), pos2(lcd.left() - 8.0, rect.bottom()));
    hw::well(&p, klcd, 4.0);
    if let Some(key) = m.key.filter(|_| m.has_track) {
        p.text(pos2(klcd.right() - 7.0, klcd.top() + 4.0), Align2::RIGHT_TOP, key.label(), fonts::bold(18.0 * k()), theme::key_well_color(key));
        // Durch Pitch verschoben: kleines Dreieck nach oben bzw. unten neben dem Wert
        if m.key_shift != 0 {
            let c = pos2(klcd.right() - 7.0 - 34.0 * k(), klcd.top() + 13.0 * k());
            let (w, h) = (3.5 * k(), 3.0 * k() * m.key_shift.signum() as f32);
            let col = theme::well_ink().gamma_multiply(0.7);
            p.add(egui::Shape::convex_polygon(vec![c + vec2(-w, h), c + vec2(w, h), c + vec2(0.0, -h)], col, Stroke::NONE));
        }
    }
    let (klabel, kcol) = if m.key_fits { (tr("PASST"), theme::key_match()) } else { (tr("TONART"), theme::well_ink().gamma_multiply(0.55)) };
    p.text(pos2(klcd.left() + 6.0, klcd.bottom() - 4.0), Align2::LEFT_BOTTOM, klabel, fonts::bold(8.0 * k()), kcol);
    let key_tip = match m.key.filter(|_| m.has_track) {
        None if m.has_track => tr("Tonart unbekannt (im Tag steht keine, die Analyse läuft noch oder war unsicher)").to_owned(),
        None => String::new(),
        Some(key) if m.key_shift != 0 => {
            trf("Klingt in {}, original {} (Pitch verschiebt die Tonhöhe um {} Halbton)", &[&key.label(), &key.shifted(-m.key_shift).label(), &format!("{:+}", m.key_shift)])
        }
        Some(key) => trf("Tonart {} (Camelot)", &[&key.label()]),
    };
    if !key_tip.is_empty() {
        ui.interact(klcd, ui.id().with("key_lcd"), Sense::hover()).on_hover_text(key_tip);
    }
    let mut text_right = klcd.left() - 10.0;

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

/// So lange stillgehalten, greift die Hand die Platte (hält sie an), auch ohne zu ziehen (Sekunden).
const GRAB_HOLD_SECS: f64 = 0.15;
/// So weit bewegt (Punkte), greift die Hand sofort und scratcht.
const GRAB_MOVE_PX: f32 = 3.0;

/// Gedrückte linke Maustaste auf dem Teller: wann und wo, und ob die Hand die Platte schon greift.
#[derive(Clone, Copy)]
struct Press {
    at: f64,
    origin: Pos2,
    grabbed: bool,
    /// Winkel des Zeigers im letzten Bild (für das Scratchen).
    angle: f32,
}

/// Teller: kurz klicken = Play bzw. Pause, festhalten = anhalten, ziehen = scratchen, Rechtsklick = Stopp,
/// Mausrad = scratchen. Die Hand greift erst nach kurzer Zeit oder Bewegung, damit ein Klick nicht vorher
/// schon die Platte bremst (sonst liefe sie beim Pausieren nicht mehr aus).
fn platter(ui: &mut egui::Ui, m: &DeckModel<'_>, color: Color32, rect: Rect, out: &mut Vec<DeckAction>) {
    let id = ui.id().with(("platter", m.deck));
    let resp = ui.interact(rect, id, Sense::drag());
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5 - 2.0;
    let angle = (m.position * REV_PER_SEC * TAU as f64 % TAU as f64) as f32;
    paint_platter(ui, c, r, angle, color, m, resp.dragged());

    let pointer_angle = |p: Pos2| (p.y - c.y).atan2(p.x - c.x);
    let primary = egui::PointerButton::Primary;
    let now = ui.input(|i| i.time);
    let mut press: Option<Press> = ui.data(|d| d.get_temp(id));
    if resp.drag_started_by(primary) && m.has_track
        && let Some(p) = resp.interact_pointer_pos()
    {
        press = Some(Press { at: now, origin: p, grabbed: false, angle: pointer_angle(p) });
    }
    if let Some(pr) = press.as_mut()
        && resp.dragged_by(primary)
        && m.has_track
        && let Some(p) = resp.interact_pointer_pos()
    {
        if !pr.grabbed && (now - pr.at >= GRAB_HOLD_SECS || p.distance(pr.origin) > GRAB_MOVE_PX) {
            pr.grabbed = true;
            out.push(DeckAction::ScratchBegin);
        }
        if pr.grabbed {
            let a = pointer_angle(p);
            let mut da = a - pr.angle;
            if da > std::f32::consts::PI { da -= TAU; }
            if da < -std::f32::consts::PI { da += TAU; }
            pr.angle = a;
            if da != 0.0 {
                out.push(DeckAction::ScratchBy(da as f64 / TAU as f64 / REV_PER_SEC));
            }
        } else {
            // Ohne Bewegung kommt sonst kein neues Bild, das das Greifen auslöst
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(GRAB_HOLD_SECS / 3.0));
        }
    }
    if resp.drag_stopped_by(primary) {
        match press.take() {
            Some(pr) if pr.grabbed => out.push(DeckAction::ScratchEnd),
            // Kurz angetippt: wie die Play/Pause-Taste (Pause läuft mit dem Motor aus)
            Some(_) if m.has_track => out.push(DeckAction::TogglePlay),
            _ => {}
        }
    }
    ui.data_mut(|d| match press {
        Some(pr) => {
            d.insert_temp(id, pr);
        }
        None => d.remove::<Press>(id),
    });
    wheel_scratch(ui, &resp, m.has_track, out);
    // Rechtsklick wie die Stopp-Taste: läuft aus, danach zurück zum Cue-Punkt
    let right_click = resp.hovered() && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
    if right_click && m.has_track {
        out.push(DeckAction::Stop);
    }
    if m.has_track {
        resp.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text(tr("Klick = Play/Pause, Rechtsklick = Stopp, festhalten, ziehen oder Mausrad = scratchen"));
    }
}

/// Trackpad-Weg (Punkte) für eine volle Umdrehung beim Scratchen mit dem Rad.
const WHEEL_POINTS_PER_REV: f32 = 400.0;
/// Eine Rastung des Mausrads dreht die Platte um diesen Anteil einer Umdrehung.
const WHEEL_REV_PER_NOTCH: f32 = 1.0 / 16.0;
/// So lange ohne Radbewegung, dann gilt die Platte als losgelassen (Sekunden).
const WHEEL_RELEASE_SECS: f64 = 0.15;

/// Mausrad bzw. Trackpad über dem Teller scratcht: nach oben oder rechts dreht vorwärts. Die erste Bewegung
/// legt die Hand auf die Platte, nach `WHEEL_RELEASE_SECS` Ruhe lässt sie los; der Schwung des Trackpads
/// (auch das Nachgleiten des Systems) rollt dann wie beim Ziehen nach.
fn wheel_scratch(ui: &egui::Ui, resp: &egui::Response, has_track: bool, out: &mut Vec<DeckAction>) {
    let id = resp.id.with("wheel-scratch");
    let now = ui.input(|i| i.time);
    let mut last: Option<f64> = ui.data(|d| d.get_temp(id));
    if has_track && !resp.dragged() {
        let w = hw::take_wheel(ui, resp);
        let revs = w.points / WHEEL_POINTS_PER_REV + w.notches * WHEEL_REV_PER_NOTCH;
        if revs != 0.0 {
            if last.is_none() {
                out.push(DeckAction::ScratchBegin);
            }
            out.push(DeckAction::ScratchBy(revs as f64 / REV_PER_SEC));
            last = Some(now);
        }
    }
    if let Some(t) = last {
        if resp.dragged() || !has_track || now - t > WHEEL_RELEASE_SECS {
            // Ziehen mit der Maus übernimmt selbst; sonst Hand weg
            if !resp.dragged() {
                out.push(DeckAction::ScratchEnd);
            }
            last = None;
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(WHEEL_RELEASE_SECS / 2.0));
        }
    }
    ui.data_mut(|d| match last {
        Some(t) => {
            d.insert_temp(id, t);
        }
        None => d.remove::<f64>(id),
    });
}

/// Dauer, in der die Platte beim Auswerfen hinaus- bzw. beim Laden hereingleitet (Sekunden).
const RECORD_SLIDE_SECS: f32 = 0.5;

/// Teller mit Gummimatte, darauf die Platte. Ohne Titel liegt keine Platte auf; beim Auswerfen gleitet sie
/// nach außen aus dem Player (A nach links, B nach rechts), beim Laden von dort wieder herein.
fn paint_platter(ui: &egui::Ui, c: Pos2, r: f32, angle: f32, color: Color32, m: &DeckModel<'_>, grabbing: bool) {
    let p = ui.painter();
    let rot = |a: f32, rr: f32| c + vec2((a + angle).cos(), (a + angle).sin()) * rr;
    // Sparsame Darstellung: ohne Schatten und Lichtreflex, weniger Stroboskop-Punkte und Rillen
    let lite = theme::lite();

    // Schatten unter dem Teller, dann Metallrand mit Glanz oben links und Stroboskop-Punkten
    for k in 0..if lite { 0 } else { 6 } {
        p.circle_filled(c + vec2(0.0, 5.0), r + 4.0 - k as f32 * 1.5, Color32::from_black_alpha(18));
    }
    p.circle_filled(c, r, hw::platter_rim());
    let arc = |from: f32, to: f32, rr: f32, stroke: Stroke| {
        let pts: Vec<Pos2> = (0..=24).map(|i| { let t = from + (to - from) * i as f32 / 24.0; c + vec2(t.cos(), t.sin()) * rr }).collect();
        p.add(egui::Shape::line(pts, stroke));
    };
    arc(3.4, 5.4, r - 1.5, Stroke::new(2.5, Color32::from_white_alpha(60)));
    arc(0.3, 2.3, r - 1.5, Stroke::new(2.5, Color32::from_black_alpha(90)));
    p.circle_stroke(c, r * 0.915, Stroke::new(2.0, Color32::from_black_alpha(140)));
    let dots = if lite { 24 } else { 72 };
    for i in 0..dots {
        p.circle_filled(rot(i as f32 / dots as f32 * TAU, r * 0.955), (r * 0.012).max(1.0), Color32::from_rgb(0xcf, 0xd6, 0xe4));
    }
    // Gummimatte mit feinen Ringen, sichtbar, wenn keine Platte aufliegt
    p.circle_filled(c, r * 0.91, Color32::from_rgb(0x1a, 0x1d, 0x24));
    for k in 1..=if lite { 2 } else { 5 } {
        p.circle_stroke(c, r * (0.18 * k as f32).min(0.88), Stroke::new(1.0, Color32::from_white_alpha(10)));
    }
    let pin = |p: &egui::Painter| p.circle_filled(c, r * 0.03, Color32::from_rgb(0xcf, 0xd6, 0xe4));
    pin(p);

    // Lage der Platte: 0 = auf dem Teller, 1 = ganz draußen; weich angefahren und abgebremst
    let id = ui.id().with(("record-slide", m.deck));
    let t = ui.ctx().animate_value_with_time(id, if m.has_track { 0.0 } else { 1.0 }, RECORD_SLIDE_SECS);
    let slide = t * t * (3.0 - 2.0 * t);
    // Beim Hinausgleiten dreht sie im Winkel weiter, in dem sie zuletzt stand
    let angle_id = id.with("angle");
    let angle = if m.has_track {
        ui.ctx().data_mut(|d| d.insert_temp(angle_id, angle));
        angle
    } else {
        ui.ctx().data(|d| d.get_temp(angle_id)).unwrap_or(angle)
    };
    let (title, artist, cover) = match (&m.leaving, m.has_track) {
        (_, true) => (m.title, m.artist, m.cover),
        (Some(l), false) => (l.title, l.artist, l.cover),
        (None, false) => ("", "", None),
    };
    if slide >= 1.0 {
        return;
    }
    // Nach außen aus dem Player: A nach links, B nach rechts; abgeschnitten am Rand des Player-Gehäuses
    let out_dir = if m.deck == 1 { 1.0 } else { -1.0 };
    let c = c + vec2(out_dir * slide * r * 2.3, 0.0);
    let clipped = p.with_clip_rect(ui.max_rect().expand(6.0).intersect(ui.clip_rect()));
    let p = &clipped;
    let rot = |a: f32, rr: f32| c + vec2((a + angle).cos(), (a + angle).sin()) * rr;
    if slide > 0.0 && !lite {
        // Schatten der angehobenen Platte
        p.circle_filled(c + vec2(0.0, 4.0), r * 0.92, Color32::from_black_alpha(70));
    }

    // Vinyl mit Rillen; mit Cover bleibt nur ein schmaler Rillenrand außen (Picture-Disc)
    p.circle_filled(c, r * 0.91, Color32::from_rgb(0x0b, 0x0b, 0x0e));
    if let Some(tex) = cover {
        paint_cover(p, c, r * COVER_RADIUS, angle, tex);
        p.circle_stroke(c, r * COVER_RADIUS, Stroke::new((r * 0.008).max(1.0), Color32::from_black_alpha(200)));
    }
    let mut rr = if cover.is_some() { r * (COVER_RADIUS + 0.012) } else { r * 0.36 };
    let mut k = 0;
    while rr < r * 0.89 {
        p.circle_stroke(c, rr, Stroke::new(1.0, Color32::from_white_alpha(if k % 3 == 0 { 12 } else { 6 })));
        rr += r * if lite { 0.066 } else { 0.022 };
        k += 1;
    }
    // Lichtreflex steht still, während die Platte darunter dreht
    let reflexes: &[(f32, u8)] = if lite { &[] } else { &[(-2.4, 16), (0.7, 10)] };
    for &(a, alpha) in reflexes {
        let pts: Vec<Pos2> = (0..=12)
            .map(|i| { let t = a - 0.35 + i as f32 / 12.0 * 0.7; c + vec2(t.cos(), t.sin()) * r * 0.86 })
            .chain((0..=12).rev().map(|i| { let t = a - 0.25 + i as f32 / 12.0 * 0.5; c + vec2(t.cos(), t.sin()) * r * 0.4 }))
            .collect();
        p.add(egui::Shape::convex_polygon(pts, Color32::from_white_alpha(alpha), Stroke::NONE));
    }
    // Label mit Titel, dreht mit (entfällt, wenn das Cover die Platte füllt)
    if cover.is_none() {
        p.circle_filled(c, r * 0.33, color);
    }
    let text_color = Color32::from_rgba_unmultiplied(10, 14, 22, 220);
    let label_text = |text: &str, size: f32, dy: f32| {
        // So breit ist das Label auf dieser Höhe (Sehne des Kreises, mit etwas Rand). Passt der Text nicht,
        // wird die Schrift bis auf drei Viertel kleiner und dann mit „…“ gekürzt (sonst verschwände der Rest
        // dunkel auf dem schwarzen Vinyl)
        let lr = r * 0.30;
        let max_w = 2.0 * (lr * lr - dy * dy).max(0.0).sqrt();
        let upper = text.to_uppercase();
        let mut size = size;
        let min_size = (size * 0.75).max(6.0);
        let width = |t: &str, size: f32| p.layout_no_wrap(t.to_owned(), FontId::proportional(size), text_color).size().x;
        while size > min_size && width(&upper, size) > max_w {
            size = (size * 0.92).max(min_size);
        }
        let mut t = upper.clone();
        let mut n = upper.chars().count();
        while n > 1 && width(&t, size) > max_w {
            n -= 1;
            t = upper.chars().take(n).collect::<String>().trim_end().to_owned() + "…";
        }
        let galley = p.layout_no_wrap(t, FontId::proportional(size), text_color);
        let anchor = c + egui::emath::Rot2::from_angle(angle) * vec2(0.0, dy);
        let pos = anchor - galley.size() / 2.0;
        p.add(TextShape::new(pos, galley, text_color).with_angle_and_anchor(angle, Align2::CENTER_CENTER));
    };
    if cover.is_some() {
        // Mittelloch der Picture-Disc
        p.circle_filled(c, r * 0.05, Color32::from_black_alpha(150));
    } else {
        label_text(title, (r * 0.085).max(8.0), -r * 0.1);
        label_text(artist, (r * 0.065).max(7.0), r * 0.1);
    }
    // Markierung, damit die Drehung sichtbar ist
    let a = -std::f32::consts::FRAC_PI_2;
    let mark_from = if cover.is_some() { COVER_RADIUS + 0.012 } else { 0.4 };
    p.line_segment([rot(a, r * mark_from), rot(a, r * 0.88)], Stroke::new((r * 0.02).max(1.5), Color32::from_white_alpha(230)));
    if slide <= 0.0 {
        // Der Dorn schaut durch das Mittelloch
        pin(p);
        if grabbing || m.scratching {
            p.circle_stroke(c, r + 1.0, Stroke::new(2.5, color));
        }
    } else {
        p.circle_filled(c, r * 0.03, Color32::from_black_alpha(200));
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

/// Tonarm rechts neben dem Teller, bei beiden Playern wie bei echten Plattenspielern. Steht der Player, liegt er auf seiner Stütze; bei Play
/// schwenkt er auf die Platte und senkt sich, die Nadel wandert mit dem Titel von außen nach innen. Bei Pause
/// hebt er sich über der Stelle. Den Tonkopf ziehen springt an die Stelle, an der die Nadel losgelassen wird.
fn tonearm(ui: &mut egui::Ui, m: &DeckModel<'_>, rect: Rect, out: &mut Vec<DeckAction>) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5 - 2.0;
    let id = ui.id().with(("tonearm", m.deck));
    let ctx = ui.ctx().clone();
    let now = ui.input(|i| i.time);

    let frac = if m.duration > 0.0 { (m.position / m.duration) as f32 } else { 0.0 };
    let on_record = m.has_track && (m.playing || m.speed.abs() > 0.01 || m.scratching);
    let paused = m.has_track && !on_record && m.position > 0.5;
    let mut target = if on_record || paused { groove_at(frac) } else { ARM_REST };

    let pivot = c + vec2(1.0, -1.0) * (ARM_PIVOT * r * std::f32::consts::FRAC_1_SQRT_2);
    let dir = |rho: f32| {
        let a = arm_angle(rho);
        vec2(a.cos(), a.sin())
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

    // Schwenk und Heben weich, beim Ziehen folgt der Arm der Hand. Dem langsamen Wandern in der Rille folgt
    // er direkt: Eine Animation mit jedes Bild neuem Ziel würde selbst ständig neue Bilder anfordern.
    let rho = match dragging {
        Some(d) => ctx.animate_value_with_time(id.with("rho"), d, 0.0),
        None if (shown - target).abs() < 0.01 => ctx.animate_value_with_time(id.with("rho"), target, 0.0),
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
    p.line_segment([finger, finger + n * r * 0.09], Stroke::new((r * 0.022).max(1.5), metal));
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

/// Pitch-Spalte (mit Filter) mittig zwischen Bedienfeld und Außenkante des Players setzen, statt an der Kante
/// zur Mittelspalte zu kleben, wenn das Bedienfeld auf breiten Playern schmaler bleibt als der Platz.
fn centered_pitch(pitch: Rect, ctrl: Rect, avail: Rect, mirrored: bool) -> Rect {
    let (a, b) = if mirrored { (avail.left(), ctrl.left()) } else { (ctrl.right(), avail.right()) };
    let cx = ((a + b) / 2.0).clamp(a + pitch.width() / 2.0, b - pitch.width() / 2.0);
    Rect::from_center_size(pos2(cx, pitch.center().y), pitch.size())
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

    // Vorhören, Sync, Rückwärts
    ui.horizontal(|ui| {
        let pfl_w = (w - 2.0 * gap) * 0.46;
        let rest_w = (w - 2.0 * gap - pfl_w) / 2.0;
        // In schmalen Playern erst ohne Symbol, dann kleiner, damit die Beschriftung nicht über SYNC ragt
        let fits = |text: &str, size: f32| ui.painter().layout_no_wrap(text.to_owned(), fonts::bold(size), Color32::WHITE).size().x <= pfl_w - 8.0;
        let full = format!("🎧 {}", tr("VORHÖREN"));
        let size = 12.0 * k();
        let (label, size) = if fits(&full, size) {
            (full, size)
        } else {
            let short = tr("VORHÖREN").to_owned();
            let mut s = size;
            while s > 8.0 && !fits(&short, s) {
                s -= 0.5;
            }
            (short, s)
        };
        if hw::button(ui, vec2(pfl_w, 28.0 * k()), &label, fonts::bold(size), hw::Look::Color(color), m.pfl, true)
            .on_hover_text(tr("Auf dem Kopfhörer vorhören, unabhängig vom Crossfader"))
            .clicked()
        {
            out.push(DeckAction::Pfl(!m.pfl));
        }
        let tip = if m.auto_synced { tr("Auto-Sync steuert den Pitch; Pitch anfassen übernimmt von Hand") } else { tr("Tempo an den anderen Player angleichen") };
        if hw::button(ui, vec2(rest_w, 28.0 * k()), "SYNC", fonts::bold(12.0 * k()), hw::Look::Color(theme::yellow()), m.auto_synced, enabled && m.bpm.is_some())
            .on_hover_text(tip)
            .clicked()
        {
            out.push(DeckAction::Sync);
        }
        if hw::button(ui, vec2(rest_w, 28.0 * k()), "REV", fonts::bold(12.0 * k()), hw::Look::Color(theme::cue()), m.reverse, true)
            .on_hover_text(tr("Rückwärts: der Motor dreht die Platte andersherum"))
            .clicked()
        {
            out.push(DeckAction::Reverse(!m.reverse));
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

/// Schritt des Filters je Pfeiltaste bzw. Rastung (Reglerweg −1 … +1).
const FILTER_STEP: f32 = 0.05;

/// Filterknopf oben in der Pitch-Spalte: Mitte aus, links Tiefpass, rechts Hochpass. Darunter „FILTER“,
/// sobald er arbeitet stattdessen Art und Grenzfrequenz (spart eine Zeile für den Pitch-Fader). Liefert die belegte Höhe.
fn filter_knob(ui: &mut egui::Ui, m: &DeckModel<'_>, area: Rect, out: &mut Vec<DeckAction>) -> f32 {
    let p = ui.painter().clone();
    let color = theme::deck_color(m.deck);
    let d = (area.width() - 4.0).min(36.0 * k());
    let kr = Rect::from_min_size(pos2(area.center().x - d / 2.0, area.top()), vec2(d, d));
    let (resp, new) = hw::knob(ui, kr, ("filter", m.deck), 0.5 + 0.5 * m.filter, 0.5, color);
    let keyed = hw::nudge(ui, &resp, m.filter, FILTER_STEP, -1.0, 1.0);
    resp.on_hover_text(tr("Filter: links Tiefpass, rechts Hochpass, Mitte aus (Rechtsklick)"));
    if let Some(v) = keyed {
        out.push(DeckAction::Filter(if v.abs() < 0.01 { 0.0 } else { v }));
    } else if let Some(n) = new {
        // Nahe der Mitte einrasten
        let v = 2.0 * n - 1.0;
        out.push(DeckAction::Filter(if v.abs() < 0.04 { 0.0 } else { v }));
    }
    let freq = |hz: f32| if hz >= 1000.0 { format!("{:.1}k", hz / 1000.0) } else { format!("{hz:.0}") };
    let state = fw_engine::filter::cutoff_hz(m.filter);
    let reading = match state {
        Some((true, hz)) => format!("{} {}", tr("TP"), freq(hz)),
        Some((false, hz)) => format!("HP {}", freq(hz)),
        None => tr("FILTER").to_owned(),
    };
    let reading_y = kr.bottom() + 2.0;
    p.text(pos2(area.center().x, reading_y), Align2::CENTER_TOP, reading, fonts::bold(9.0 * k()), if state.is_some() { color } else { theme::dim() });
    reading_y + 11.0 * k() - area.top()
}

/// Mindesthöhe des Pitch-Faders, ab der noch Platz für die Beschriftung darunter bleibt.
const PITCH_LABEL_MIN_FADER: f32 = 120.0;

fn pitch(ui: &mut egui::Ui, m: &DeckModel<'_>, out: &mut Vec<DeckAction>) {
    // Oben der Filter, darunter der Pitch-Fader
    let full = ui.max_rect();
    let used = filter_knob(ui, m, full, out);
    let area = Rect::from_min_max(pos2(full.left(), full.top() + used + 4.0 * k()), full.max);
    // Oben = schneller wie an Plattenspielern mit „+“ oben
    let value = (m.pitch_pct + PITCH_RANGE) / (2.0 * PITCH_RANGE);
    // Rechtsklick irgendwo auf den Pitch-Bereich lässt ihn langsam auf 0 laufen (über den Fader, damit es
    // nicht hörbar springt). Liegt unter dem Fader, damit dieser bedienbar bleibt.
    if ui.interact(area, ui.id().with(("pitch-reset", m.deck)), Sense::click()).secondary_clicked() {
        hw::start_glide(ui, ("pitch", m.deck), value, 0.5, 1.0);
    }
    let p = ui.painter().clone();
    // Beschriftung „PITCH“ unter dem Fader nur, wenn der Fader trotzdem genug Weg behält
    let lcd_h = 20.0 * k();
    let label_h = 14.0 * k();
    let show_label = area.height() - lcd_h - 4.0 - label_h >= PITCH_LABEL_MIN_FADER;
    let below = lcd_h + 4.0 + if show_label { label_h } else { 0.0 };
    let fader = Rect::from_min_max(pos2(area.left() + 4.0, area.top()), pos2(area.right() - 4.0, area.bottom() - below));
    if show_label {
        p.text(pos2(area.center().x, fader.bottom() + 2.0), Align2::CENTER_TOP, "PITCH", fonts::bold(9.5 * k()), theme::dim());
    }
    // Grundwert 0,5 = Mitte = 0 %
    let (resp, new) = hw::vertical_fader(ui, fader, ("pitch", m.deck), value, 0.5, true);
    // Pfeiltasten über dem Fader: 0,1 % je Druck
    let keyed = hw::nudge(ui, &resp, (m.pitch_pct * 10.0).round() / 10.0, 0.1, -PITCH_RANGE, PITCH_RANGE);
    resp.on_hover_text(tr("Pitch (Tempo). Rechtsklick oder Doppelklick setzt auf 0"));
    if let Some(pct) = keyed.or(new.map(|v| ((v * 2.0 - 1.0) * PITCH_RANGE * 10.0).round() / 10.0)) {
        out.push(DeckAction::Pitch((pct * 10.0).round() / 10.0));
    }
    let lcd = Rect::from_min_max(pos2(area.left(), area.bottom() - lcd_h), pos2(area.right(), area.bottom()));
    hw::well(&p, lcd, 3.0);
    let sign = if m.pitch_pct > 0.0 { "+" } else if m.pitch_pct < 0.0 { "-" } else { " " };
    hw::lcd_text(&p, lcd.center(), Align2::CENTER_CENTER, &format!("{sign}{:.1}%", m.pitch_pct.abs()), 10.5, theme::well_ink());
}
