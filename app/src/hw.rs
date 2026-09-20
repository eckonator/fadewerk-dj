// SPDX-License-Identifier: GPL-3.0-or-later
//! Plastische Hardware-Optik: Panels mit Licht von oben, eingelassene Displays, gewölbte Tasten,
//! die sich eindrücken und als LED-Taster leuchten, Fader mit Metallkappe und LCD-Ziffern.
//!
//! Alles leitet sich aus den Farben des aktiven Skins ab.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Shadow, Stroke, StrokeKind,
    epaint::{CornerRadiusF32, Mesh, tessellator::path::rounded_rectangle},
    pos2, vec2,
};

use crate::fonts;
use crate::theme::{self, mix};

pub fn lighten(c: Color32, t: f32) -> Color32 {
    mix(c, Color32::WHITE, t)
}

pub fn darken(c: Color32, t: f32) -> Color32 {
    mix(c, Color32::BLACK, t)
}

/// Schwarz oder Weiß, je nachdem, was auf `face` besser lesbar ist.
pub fn contrast(face: Color32) -> Color32 {
    let l = 0.299 * face.r() as f32 + 0.587 * face.g() as f32 + 0.114 * face.b() as f32;
    if l > 150.0 { Color32::from_rgb(12, 14, 20) } else { Color32::from_rgb(246, 248, 252) }
}

/// Abgerundetes Rechteck mit senkrechtem Verlauf (sparsame Darstellung: einfarbig).
pub fn gradient_rrect(p: &Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    if theme::lite() {
        p.rect_filled(rect, radius, mix(top, bottom, 0.5));
        return;
    }
    let pts = rrect_points(rect, radius);
    if pts.len() < 3 {
        return;
    }
    let col = |y: f32| mix(top, bottom, ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0));
    let mut mesh = Mesh::default();
    let c = rect.center();
    mesh.colored_vertex(c, col(c.y));
    for pt in &pts {
        mesh.colored_vertex(*pt, col(pt.y));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(mesh);
}

/// Kantenpunkte eines abgerundeten Rechtecks.
fn rrect_points(rect: Rect, radius: f32) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(64);
    rounded_rectangle(&mut pts, rect, CornerRadiusF32::same(radius));
    pts
}

/// Größe der Stahl-Textur in Pixeln. Breit, damit sich das Muster in der Länge nicht sichtbar wiederholt.
const BRUSH_W: usize = 2048;
const BRUSH_H: usize = 512;

/// Kastenfilter entlang einer Zeile mit Umlauf (die Textur kachelt nahtlos).
fn blur_row(src: &[f32], radius: usize, dst: &mut [f32]) {
    let n = src.len();
    let len = (2 * radius + 1) as f32;
    let mut sum: f32 = (0..=2 * radius).map(|i| src[(i + n - radius) % n]).sum();
    for x in 0..n {
        dst[x] = sum / len;
        sum += src[(x + radius + 1) % n] - src[(x + n - radius) % n];
    }
}

/// Bringt ein Feld auf Mittelwert 0 und Streuung 1, damit sich die Ebenen gezielt mischen lassen.
fn normalize(v: &mut [f32]) {
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n;
    let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n).sqrt().max(1e-6);
    for x in v.iter_mut() {
        *x = (*x - mean) / sd;
    }
}

/// Textur für gebürsteten Stahl, aus mehreren Ebenen wie beim echten Schliff:
/// Grundton je Pixelzeile (leicht mit den Nachbarn verwischt), sehr lange und mittellange Schleifspuren,
/// feines Korn und einzelne Kratzer. Gespeichert als vormultipliziertes Weiß (hell) bzw. Schwarz (dunkel)
/// mit Deckkraft, damit sie sich über jede Grundfarbe legen lässt. Wird einmal erzeugt.
fn brush_texture(ctx: &egui::Context) -> egui::TextureId {
    static TEX: std::sync::OnceLock<egui::TextureHandle> = std::sync::OnceLock::new();
    TEX.get_or_init(|| {
        // Kleiner fester Zufallsgenerator (xorshift), damit das Muster bei jedem Start gleich aussieht
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        let (w, h) = (BRUSH_W, BRUSH_H);

        // Grundton je Zeile, mit den Nachbarzeilen verwischt, damit es nicht nach Bildschirmzeilen aussieht
        let raw: Vec<f32> = (0..h).map(|_| rnd()).collect();
        let mut rows: Vec<f32> = (0..h).map(|y| 0.5 * raw[y] + 0.25 * (raw[(y + h - 1) % h] + raw[(y + 1) % h])).collect();
        normalize(&mut rows);

        let mut long = vec![0.0f32; w * h];
        let mut mid = vec![0.0f32; w * h];
        let mut fine = vec![0.0f32; w * h];
        let (mut noise, mut a, mut b) = (vec![0.0f32; w], vec![0.0f32; w], vec![0.0f32; w]);
        for y in 0..h {
            for v in noise.iter_mut() {
                *v = rnd();
            }
            fine[y * w..(y + 1) * w].copy_from_slice(&noise);
            // Zweimal filtern ergibt weiche Enden der Spuren statt harter Kanten
            blur_row(&noise, 200, &mut a);
            blur_row(&a, 200, &mut b);
            long[y * w..(y + 1) * w].copy_from_slice(&b);
            blur_row(&noise, 18, &mut a);
            blur_row(&a, 18, &mut b);
            mid[y * w..(y + 1) * w].copy_from_slice(&b);
        }
        normalize(&mut long);
        normalize(&mut mid);

        let mut field: Vec<f32> = (0..w * h).map(|i| 0.40 * rows[i / w] + 0.28 * long[i] + 0.30 * mid[i] + 0.20 * fine[i]).collect();
        normalize(&mut field);

        // Einzelne Kratzer: dünne, an den Enden auslaufende Linien, mal hell, mal dunkel
        for _ in 0..140 {
            let y = ((rnd() * 0.5 + 0.5) * h as f32) as usize % h;
            let x0 = ((rnd() * 0.5 + 0.5) * w as f32) as usize;
            let len = 80 + ((rnd() * 0.5 + 0.5) * 700.0) as usize;
            let amp = rnd() * 1.4;
            for k in 0..len {
                let env = (std::f32::consts::PI * k as f32 / len as f32).sin();
                field[y * w + (x0 + k) % w] += amp * env;
            }
        }

        let pixels = field
            .iter()
            .map(|v| {
                let a = ((v * 0.32).clamp(-1.0, 1.0).abs() * 255.0) as u8;
                if *v > 0.0 { Color32::from_white_alpha(a) } else { Color32::from_black_alpha(a) }
            })
            .collect();
        ctx.load_texture("gebuersteter-stahl", egui::ColorImage::new([w, h], pixels), egui::TextureOptions::LINEAR_REPEAT)
    })
    .id()
}

/// Legt Schleifspuren über eine Fläche. `dark` = dunkler Skin.
pub fn brushed_overlay(p: &Painter, rect: Rect, radius: f32, dark: bool) {
    let pts = rrect_points(rect, radius);
    if pts.len() < 3 {
        return;
    }
    // Ein Texturpixel auf ein Bildschirmpixel, damit die Spuren fein bleiben
    let ppp = p.pixels_per_point();
    let uv = |q: Pos2| pos2(q.x * ppp / BRUSH_W as f32, q.y * ppp / BRUSH_H as f32);
    let i = if dark { 40 } else { 58 };
    let tint = Color32::from_rgba_premultiplied(i, i, i, i);
    let mut mesh = Mesh::with_texture(brush_texture(p.ctx()));
    let c = rect.center();
    mesh.vertices.push(egui::epaint::Vertex { pos: c, uv: uv(c), color: tint });
    for q in &pts {
        mesh.vertices.push(egui::epaint::Vertex { pos: *q, uv: uv(*q), color: tint });
    }
    let n = pts.len() as u32;
    for k in 0..n {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    p.add(mesh);
}

/// Lichtreflex von gebürstetem Metall: Die Rillen laufen waagerecht, darum ziehen sich die Reflexe
/// senkrecht über die Fläche (helle und dunkle Lichtsäulen). Lage leicht je Fläche versetzt.
fn sheen(p: &Painter, rect: Rect, dark: bool) {
    let w = rect.width().max(1.0);
    // Versatz aus der Lage der Fläche, damit nicht alle Panels gleich aussehen
    let shift = ((rect.left() * 0.137 + rect.top() * 0.071).sin()) * 0.06;
    let g = |u: f32, c: f32, s: f32| (-((u - c - shift) / s).powi(2)).exp();
    let peak = if dark { 34.0 } else { 60.0 };
    let edge = 10.0;
    let col = |x: f32| {
        let u = (x - rect.left()) / w;
        let b = 1.0 * g(u, 0.24, 0.11) - 0.55 * g(u, 0.52, 0.17) + 0.6 * g(u, 0.82, 0.07);
        // Zum Rand hin auslaufen, damit die runden Ecken unberührt bleiben
        let fade = ((x - rect.left()).min(rect.right() - x) / edge).clamp(0.0, 1.0);
        let a = (b * fade * peak).clamp(-255.0, 255.0);
        if a >= 0.0 { Color32::from_white_alpha(a as u8) } else { Color32::from_black_alpha((-a * if dark { 1.4 } else { 0.6 }) as u8) }
    };
    let cols = 48;
    let mut mesh = Mesh::default();
    for k in 0..=cols {
        let x = rect.left() + w * k as f32 / cols as f32;
        let c = col(x);
        mesh.colored_vertex(pos2(x, rect.top()), c);
        mesh.colored_vertex(pos2(x, rect.bottom()), c);
        if k > 0 {
            let i = 2 * k as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    p.add(mesh);
}

/// Grundfarbe einer Stahlfläche: Panel-Farbe des Skins Richtung neutrales Metallgrau gezogen,
/// damit sie nach Metall aussieht und trotzdem den Farbton des Skins behält.
pub fn steel_face(panel: Color32, dark: bool) -> Color32 {
    if dark { mix(panel, Color32::from_rgb(0x50, 0x55, 0x5d), 0.4) } else { mix(panel, Color32::from_rgb(0xc9, 0xcd, 0xd3), 0.45) }
}

/// Ruhige Fläche unter Listen: Auf Stahl dämpft sie Schleifspuren und Reflexe, damit die Schrift
/// darüber gut lesbar bleibt (wie ein satiniertes Feld im gebürsteten Blech). Ohne Stahl nichts.
pub fn calm_plate(p: &Painter, rect: Rect) {
    if !theme::brushed() {
        return;
    }
    let s = theme::skin();
    let face = steel_face(s.panel, s.dark);
    p.rect_filled(rect, 4.0, face.gamma_multiply(if s.dark { 0.72 } else { 0.68 }));
    p.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_black_alpha(if s.dark { 70 } else { 30 })), egui::StrokeKind::Inside);
}

/// Grundfläche mit Verlauf; bei gebürstetem Stahl zusätzlich Schleifspuren und Lichtreflexe.
pub fn surface(p: &Painter, rect: Rect, radius: f32, top: Color32, bottom: Color32) {
    gradient_rrect(p, rect, radius, top, bottom);
    if theme::brushed() {
        let dark = theme::skin().dark;
        brushed_overlay(p, rect, radius, dark);
        sheen(p, rect, dark);
    }
}

fn shadow(p: &Painter, rect: Rect, radius: f32, dy: i8, blur: u8, alpha: u8) {
    if theme::lite() {
        return;
    }
    let s = Shadow { offset: [0, dy], blur, spread: 0, color: Color32::from_black_alpha(alpha) };
    p.add(s.as_shape(rect, CornerRadius::same(radius as u8)));
}

/// Gehäuse-Panel: Schlagschatten, Licht von oben, Glanzkante.
pub fn panel(p: &Painter, rect: Rect) {
    let s = theme::skin();
    let r = 8.0;
    shadow(p, rect, r, 5, 16, if s.dark { 150 } else { 60 });
    if theme::brushed() {
        // Stahl ist heller und glänzender als lackiertes Blech
        let face = steel_face(s.panel, s.dark);
        surface(p, rect, r, lighten(face, 0.08), darken(face, 0.22));
    } else {
        gradient_rrect(p, rect, r, lighten(s.panel, if s.dark { 0.07 } else { 0.5 }), darken(s.panel, if s.dark { 0.22 } else { 0.06 }));
    }
    p.rect_stroke(rect, r, Stroke::new(1.0, darken(s.line, 0.25)), StrokeKind::Inside);
    p.line_segment(
        [pos2(rect.left() + r, rect.top() + 1.5), pos2(rect.right() - r, rect.top() + 1.5)],
        Stroke::new(1.0, Color32::from_white_alpha(if s.dark { 28 } else { 160 })),
    );
}

/// Leuchtender Warnrahmen um ein Panel (z. B. Titel kurz vor dem Ende), mit Schein nach außen.
pub fn alert_border(p: &Painter, rect: Rect, color: Color32) {
    let r = 8.0;
    let glow = if theme::lite() { 0 } else { 4 };
    for k in 1..=glow {
        let a = 90.0 / k as f32 / 255.0;
        p.rect_stroke(rect.expand(k as f32 * 1.8), r + k as f32, Stroke::new(1.8, color.gamma_multiply(a)), StrokeKind::Outside);
    }
    p.rect_stroke(rect, r, Stroke::new(2.5, color), StrokeKind::Inside);
}

/// Eingelassenes Display: innen dunkler oben (Schatten der Kante), unten eine feine Lichtkante.
pub fn well(p: &Painter, rect: Rect, radius: f32) {
    let w = theme::well();
    gradient_rrect(p, rect, radius, darken(w, 0.45), lighten(w, 0.03));
    p.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_black_alpha(170)), StrokeKind::Inside);
    p.line_segment(
        [pos2(rect.left() + radius, rect.bottom() + 0.5), pos2(rect.right() - radius, rect.bottom() + 0.5)],
        Stroke::new(1.0, Color32::from_white_alpha(if theme::skin().dark { 22 } else { 140 })),
    );
}

/// Grundfarbe und Rolle einer Taste.
#[derive(Clone, Copy)]
pub enum Look {
    Plain,
    /// Farbige Taste (CUE rot, Play grün, Deck-Farbe …).
    Color(Color32),
}

/// Zeichnet eine Hardware-Taste in `rect`.
#[allow(clippy::too_many_arguments)]
pub fn paint_button(p: &Painter, rect: Rect, label: &str, font: FontId, look: Look, lit: bool, pressed: bool, hovered: bool, enabled: bool) {
    let (c, text) = paint_button_face(p, rect, look, lit, pressed, hovered, enabled);
    p.text(c, Align2::CENTER_CENTER, label, font, text);
}

/// Zeichnet die Taste ohne Beschriftung. Liefert Mitte und Farbe für Beschriftung oder Symbol.
fn paint_button_face(p: &Painter, rect: Rect, look: Look, lit: bool, pressed: bool, hovered: bool, enabled: bool) -> (Pos2, Color32) {
    let s = theme::skin();
    let r = 5.0;
    let base = match look {
        Look::Plain => s.panel2,
        Look::Color(c) => c,
    };
    let (mut top, mut bottom, mut text) = if lit {
        (lighten(base, 0.22), darken(base, 0.12), contrast(base))
    } else {
        match look {
            Look::Plain => (
                lighten(s.panel2, if s.dark { 0.10 } else { 0.55 }),
                darken(s.panel2, if s.dark { 0.28 } else { 0.08 }),
                s.ink,
            ),
            Look::Color(c) => {
                let face = mix(s.panel2, c, if s.dark { 0.20 } else { 0.14 });
                (lighten(face, if s.dark { 0.08 } else { 0.45 }), darken(face, if s.dark { 0.25 } else { 0.05 }), if s.dark { lighten(c, 0.35) } else { darken(c, 0.15) })
            }
        }
    };
    if hovered && enabled && !pressed {
        top = lighten(top, 0.06);
        bottom = lighten(bottom, 0.04);
    }
    if pressed {
        std::mem::swap(&mut top, &mut bottom);
    }
    if !enabled {
        text = text.gamma_multiply(0.4);
    }

    if !pressed {
        shadow(p, rect, r, 2, 5, if s.dark { 140 } else { 70 });
    }
    if lit && !theme::lite() {
        // LED-Schein um die Taste
        for k in 1..=4 {
            let a = (70 / k) as u8;
            p.rect_stroke(rect.expand(k as f32 * 1.6), r + k as f32, Stroke::new(1.6, base.gamma_multiply(a as f32 / 255.0)), StrokeKind::Outside);
        }
    }
    gradient_rrect(p, rect, r, top, bottom);
    p.rect_stroke(rect, r, Stroke::new(1.0, darken(mix(top, bottom, 0.5), 0.5)), StrokeKind::Inside);
    if !pressed {
        p.line_segment(
            [pos2(rect.left() + r, rect.top() + 1.5), pos2(rect.right() - r, rect.top() + 1.5)],
            Stroke::new(1.0, Color32::from_white_alpha(if lit { 90 } else if s.dark { 34 } else { 170 })),
        );
    }
    let c = rect.center() + if pressed { vec2(0.0, 1.0) } else { vec2(0.0, 0.0) };
    (c, text)
}

/// Play/Pause-Taste. Die Symbole werden gezeichnet statt aus der Schrift genommen: ▶ und ⏸ stammen
/// aus verschiedenen Schriften und säßen sonst unterschiedlich hoch. `icon` = Höhe der Symbole.
pub fn play_pause_button(ui: &mut egui::Ui, size: egui::Vec2, icon: f32, look: Look, lit: bool, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    let p = ui.painter();
    let (c, color) = paint_button_face(p, rect, look, lit, pressed, resp.hovered(), enabled);
    let h = icon;
    let gap = h * 0.55;
    // Dreieck links, optisch etwas breiter als hoch geteilt, damit es neben den Balken gleich schwer wirkt
    let tri_w = h * 0.85;
    let bar_w = h * 0.3;
    let total = tri_w + gap + 2.0 * bar_w + h * 0.22;
    let left = c.x - total / 2.0;
    let (top, bottom) = (c.y - h / 2.0, c.y + h / 2.0);
    p.add(egui::Shape::convex_polygon(
        vec![pos2(left, top), pos2(left + tri_w, c.y), pos2(left, bottom)],
        color,
        Stroke::NONE,
    ));
    let bx = left + tri_w + gap;
    for x in [bx, bx + bar_w + h * 0.22] {
        p.rect_filled(Rect::from_min_max(pos2(x, top), pos2(x + bar_w, bottom)), 1.0, color);
    }
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Hardware-Taste als Widget.
pub fn button(ui: &mut egui::Ui, size: egui::Vec2, label: &str, font: FontId, look: Look, lit: bool, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    paint_button(ui.painter(), rect, label, font, look, lit, pressed, resp.hovered(), enabled);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Vorhör-Taste: ruhend Kopfhörer-Symbol, während der Titel im Kopfhörer läuft rot leuchtend mit Stopp-Symbol.
pub fn preview_button(ui: &mut egui::Ui, size: egui::Vec2, previewing: bool) -> Response {
    let font = egui::FontId::proportional(size.y * 0.55);
    if previewing {
        button(ui, size, "⏹", font, Look::Color(theme::cue()), true, true).on_hover_text(crate::i18n::tr("Vorhören beenden"))
    } else {
        button(ui, size, "🎧", font, Look::Plain, false, true).on_hover_text(crate::i18n::tr("Auf dem Kopfhörer vorhören"))
    }
}

/// Text in 7-Segment-Schrift mit schwach sichtbaren, unbeleuchteten Segmenten dahinter.
pub fn lcd_text(p: &Painter, pos: Pos2, align: Align2, text: &str, size: f32, color: Color32) {
    if !theme::lite() {
        let ghost: String = text.chars().map(|c| if c.is_ascii_digit() || c == '-' || c == ' ' { '8' } else { c }).collect();
        p.text(pos, align, ghost, fonts::lcd(size), color.gamma_multiply(0.09));
    }
    p.text(pos, align, text, fonts::lcd(size), color);
}

/// Metallkappe eines Faders, mittig auf `center`.
fn fader_cap(p: &Painter, center: Pos2, size: egui::Vec2, vertical: bool) {
    let cap = Rect::from_center_size(center, size);
    shadow(p, cap, 3.0, 3, 5, 160);
    gradient_rrect(p, cap, 3.0, Color32::from_rgb(0xf4, 0xf6, 0xfa), Color32::from_rgb(0x8c, 0x94, 0xa3));
    p.rect_stroke(cap, 3.0, Stroke::new(1.0, Color32::from_rgb(0x4a, 0x50, 0x5c)), StrokeKind::Inside);
    // Griffrillen
    let line = Stroke::new(1.0, Color32::from_black_alpha(110));
    let light = Stroke::new(1.0, Color32::from_white_alpha(170));
    for k in -1..=1 {
        let o = k as f32 * 3.0;
        if vertical {
            let y = center.y + o;
            p.line_segment([pos2(cap.left() + 4.0, y), pos2(cap.right() - 4.0, y)], line);
            p.line_segment([pos2(cap.left() + 4.0, y + 1.0), pos2(cap.right() - 4.0, y + 1.0)], light);
        } else {
            let x = center.x + o;
            p.line_segment([pos2(x, cap.top() + 4.0), pos2(x, cap.bottom() - 4.0)], line);
            p.line_segment([pos2(x + 1.0, cap.top() + 4.0), pos2(x + 1.0, cap.bottom() - 4.0)], light);
        }
    }
}

/// Rechtsklick oder Doppelklick auf einen Fader setzt ihn auf seinen Grundwert zurück.
fn reset_requested(resp: &Response) -> bool {
    resp.secondary_clicked() || resp.double_clicked()
}

/// Dauer des Zurücklaufens auf den Grundwert, damit kein hörbarer Sprung entsteht: je weiter weg, desto langsamer.
/// Grunddauer plus Sekunden je voller Reglerweg, höchstens `RESET_GLIDE_MAX`.
pub const RESET_GLIDE_MIN: f64 = 0.4;
pub const RESET_GLIDE_PER_SPAN: f64 = 5.0;
pub const RESET_GLIDE_MAX: f64 = 4.0;

/// Dauer für den Weg von `from` nach `to` bei einem Regler mit Wertebereich der Breite `span`.
fn glide_secs(from: f32, to: f32, span: f32) -> f64 {
    (RESET_GLIDE_MIN + RESET_GLIDE_PER_SPAN * ((from - to).abs() / span) as f64).min(RESET_GLIDE_MAX)
}

/// Laufendes Zurückgleiten eines Reglers (im egui-Speicher unter der Regler-ID).
#[derive(Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    start: f64,
    secs: f64,
    /// Zuletzt ausgegebener Wert; weicht der Reglerwert davon ab, hat ihn jemand anders verstellt.
    last: f32,
}

/// Startet das langsame Zurückgleiten des Reglers `id` (wie beim Aufruf des Reglers angegeben) von `from` nach `to`;
/// `span` ist die Breite seines Wertebereichs (senkrechter Fader und Drehregler 1, waagerechter Fader 2).
pub fn start_glide(ui: &egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, from: f32, to: f32, span: f32) {
    start_glide_id(ui, ui.id().with(id), from, to, span);
}

fn start_glide_id(ui: &egui::Ui, id: egui::Id, from: f32, to: f32, span: f32) {
    let start = ui.input(|i| i.time);
    let secs = glide_secs(from, to, span);
    ui.ctx().data_mut(|d| d.insert_temp(id, Glide { from, to, start, secs, last: from }));
}

/// Gemeinsame Reset-Logik der Regler: Rechts-/Doppelklick startet das Zurückgleiten, Anfassen oder eine
/// Verstellung von außen bricht es ab. Liefert den Wert dieses Bilds (Bedienung hat Vorrang).
fn glide_reset(ui: &egui::Ui, resp: &Response, value: f32, default: f32, span: f32, touched: Option<f32>) -> Option<f32> {
    let id = resp.id;
    if reset_requested(resp) {
        if (value - default).abs() < 1e-4 {
            ui.ctx().data_mut(|d| d.remove::<Glide>(id));
            return Some(default);
        }
        start_glide_id(ui, id, value, default, span);
    } else if touched.is_some() {
        ui.ctx().data_mut(|d| d.remove::<Glide>(id));
        return touched;
    }
    let g = ui.ctx().data(|d| d.get_temp::<Glide>(id))?;
    if (value - g.last).abs() > 0.01 {
        ui.ctx().data_mut(|d| d.remove::<Glide>(id));
        return None;
    }
    let t = ((ui.input(|i| i.time) - g.start) / g.secs).clamp(0.0, 1.0) as f32;
    // Weich anfahren und weich ankommen
    let v = g.from + (g.to - g.from) * t * t * (3.0 - 2.0 * t);
    if t >= 1.0 {
        ui.ctx().data_mut(|d| d.remove::<Glide>(id));
        return Some(g.to);
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, Glide { last: v, ..g }));
    ui.ctx().request_repaint();
    Some(v)
}

/// Senkrechter Fader, Wert 0 (unten) bis 1 (oben). `detent` zeichnet eine Mittelmarke (Pitch 0 %).
/// `default` ist der Grundwert, auf den er bei Rechts- bzw. Doppelklick langsam zurückläuft. Liefert Antwort und, falls bedient, den neuen Wert.
pub fn vertical_fader(ui: &mut egui::Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, value: f32, default: f32, detent: bool) -> (Response, Option<f32>) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let p = ui.painter();
    let cap_h = 20.0;
    let travel = rect.height() - cap_h;
    let faint = theme::faint();
    for i in 0..=8 {
        let y = rect.top() + cap_h / 2.0 + travel * i as f32 / 8.0;
        let w = if detent && i == 4 { 9.0 } else { 5.0 };
        let col = if detent && i == 4 { theme::ink() } else { faint };
        p.line_segment([pos2(rect.left(), y), pos2(rect.left() + w, y)], Stroke::new(1.0, col));
        p.line_segment([pos2(rect.right() - w, y), pos2(rect.right(), y)], Stroke::new(1.0, col));
    }
    let slot = Rect::from_center_size(rect.center(), vec2(6.0, rect.height() - 10.0));
    gradient_rrect(p, slot, 3.0, Color32::from_rgb(0, 0, 0), Color32::from_rgb(24, 28, 36));
    let cap_y = rect.top() + cap_h / 2.0 + (1.0 - value.clamp(0.0, 1.0)) * travel;
    fader_cap(p, pos2(rect.center().x, cap_y), vec2(rect.width() - 8.0, cap_h), true);

    let mut out = None;
    if (resp.dragged() || resp.clicked()) && let Some(pos) = resp.interact_pointer_pos() {
        out = Some((1.0 - (pos.y - rect.top() - cap_h / 2.0) / travel).clamp(0.0, 1.0));
    }
    let out = glide_reset(ui, &resp, value, default, 1.0, out);
    (resp, out)
}

/// Kleiner waagerechter Regler für Werte von `min` bis `max` mit Grundwert `default`, nimmt die restliche Zeilenbreite.
#[allow(clippy::too_many_arguments)]
pub fn mini_fader(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, value: f32, default: f32, min: f32, max: f32, width: f32) -> (Response, Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(40.0), 20.0), Sense::hover());
    let to_x = |v: f32| (v - min) / (max - min) * 2.0 - 1.0;
    let (resp, new) = horizontal_fader(ui, rect, id, to_x(value), to_x(default));
    (resp, new.map(|n| min + (n + 1.0) / 2.0 * (max - min)))
}

/// Drehregler, Wert 0 bis 1, Grundwert `default`. Ziehen nach oben dreht auf, mit ⇧ fein;
/// Rechts- oder Doppelklick lässt ihn langsam auf den Grundwert zurücklaufen. Der Bogen zeigt die Abweichung vom Grundwert.
pub fn knob(ui: &mut egui::Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, value: f32, default: f32, color: Color32) -> (Response, Option<f32>) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
        out = Some((value - resp.drag_delta().y / KNOB_TRAVEL_PX * fine).clamp(0.0, 1.0));
    }
    let out = glide_reset(ui, &resp, value, default, 1.0, out);
    let v = out.unwrap_or(value);
    let s = theme::skin();
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    // Winkel: 0 = oben, von −135° (Wert 0) bis +135° (Wert 1)
    let angle = |t: f32| (-0.75 + 1.5 * t) * std::f32::consts::PI;
    let at = |a: f32, rr: f32| c + vec2(a.sin(), -a.cos()) * rr;
    let arc = |from: f32, to: f32, rr: f32, stroke: Stroke| {
        let n = 32;
        let pts: Vec<Pos2> = (0..=n).map(|i| at(angle(from + (to - from) * i as f32 / n as f32), rr)).collect();
        p.add(egui::Shape::line(pts, stroke));
    };
    let ring = r - 1.5;
    arc(0.0, 1.0, ring, Stroke::new(2.5, if s.dark { Color32::from_black_alpha(160) } else { Color32::from_black_alpha(40) }));
    arc(default, v, ring, Stroke::new(2.5, color));
    // Markierung am Grundwert
    p.line_segment([at(angle(default), ring - 2.5), at(angle(default), ring + 1.5)], Stroke::new(1.0, theme::faint()));

    let body = r * 0.68;
    p.circle_filled(c + vec2(0.0, 1.5), body, Color32::from_black_alpha(if s.dark { 150 } else { 60 }));
    let (top, bottom) = if s.dark { (Color32::from_rgb(0x5a, 0x61, 0x6d), Color32::from_rgb(0x2a, 0x2f, 0x38)) } else { (Color32::from_rgb(0xfd, 0xfd, 0xfe), Color32::from_rgb(0xc9, 0xce, 0xd6)) };
    p.circle_filled(c, body, bottom);
    p.circle_filled(c - vec2(0.0, body * 0.12), body * 0.86, top);
    p.circle_stroke(c, body, Stroke::new(1.0, darken(bottom, 0.4)));
    let pointer = if s.dark { theme::ink() } else { Color32::from_rgb(0x1a, 0x1f, 0x29) };
    p.line_segment([at(angle(v), body * 0.25), at(angle(v), body * 0.92)], Stroke::new(2.0, pointer));
    (resp.on_hover_cursor(egui::CursorIcon::ResizeVertical), out)
}

/// So viele Punkte Mausweg drehen einen Regler von ganz zu bis ganz auf.
const KNOB_TRAVEL_PX: f32 = 160.0;

/// Waagerechter Fader (Crossfader), Wert -1 (links) bis +1 (rechts), Grundwert `default`.
pub fn horizontal_fader(ui: &mut egui::Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, value: f32, default: f32) -> (Response, Option<f32>) {
    let resp = ui.interact(rect, ui.id().with(id), Sense::click_and_drag());
    let p = ui.painter();
    let cap_w = 26.0;
    let travel = rect.width() - cap_w;
    for i in 0..=8 {
        let x = rect.left() + cap_w / 2.0 + travel * i as f32 / 8.0;
        let col = if i == 4 { theme::ink() } else { theme::faint() };
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.top() + 4.0)], Stroke::new(1.0, col));
        p.line_segment([pos2(x, rect.bottom() - 4.0), pos2(x, rect.bottom())], Stroke::new(1.0, col));
    }
    let slot = Rect::from_center_size(rect.center(), vec2(rect.width() - 10.0, 6.0));
    gradient_rrect(p, slot, 3.0, Color32::from_rgb(0, 0, 0), Color32::from_rgb(24, 28, 36));
    let cap_x = rect.left() + cap_w / 2.0 + (value.clamp(-1.0, 1.0) + 1.0) / 2.0 * travel;
    fader_cap(p, pos2(cap_x, rect.center().y), vec2(cap_w, rect.height() - 6.0), false);

    let mut out = None;
    if (resp.dragged() || resp.clicked()) && let Some(pos) = resp.interact_pointer_pos() {
        out = Some(((pos.x - rect.left() - cap_w / 2.0) / travel * 2.0 - 1.0).clamp(-1.0, 1.0));
    }
    let out = glide_reset(ui, &resp, value, default, 2.0, out);
    (resp, out)
}
