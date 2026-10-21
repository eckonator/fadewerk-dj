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

/// Größe der Körnung für gestrahltes Aluminium (kachelt nahtlos).
const GRAIN: usize = 256;

/// Textur für feinkörnig gestrahltes, eloxiertes Aluminium: gleichmäßiges Korn ohne Richtung,
/// leicht verwischt, damit es samtig statt verrauscht wirkt. Wie die Stahl-Textur als Weiß/Schwarz mit Deckkraft.
fn grain_texture(ctx: &egui::Context) -> egui::TextureId {
    static TEX: std::sync::OnceLock<egui::TextureHandle> = std::sync::OnceLock::new();
    TEX.get_or_init(|| {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        };
        let n = GRAIN;
        let raw: Vec<f32> = (0..n * n).map(|_| rnd()).collect();
        // 3×3-Weichzeichner mit Umlauf, dazu etwas feines Korn obendrauf
        let mut field: Vec<f32> = (0..n * n)
            .map(|i| {
                let (x, y) = (i % n, i / n);
                let mut sum = 0.0;
                for dy in [n - 1, 0, 1] {
                    for dx in [n - 1, 0, 1] {
                        sum += raw[((y + dy) % n) * n + (x + dx) % n];
                    }
                }
                sum / 9.0 * 0.45 + raw[i] * 0.55
            })
            .collect();
        normalize(&mut field);
        let pixels = field
            .iter()
            .map(|v| {
                let a = ((v * 0.35).clamp(-1.0, 1.0).abs() * 255.0) as u8;
                if *v > 0.0 { Color32::from_white_alpha(a) } else { Color32::from_black_alpha(a) }
            })
            .collect();
        ctx.load_texture("gestrahltes-alu", egui::ColorImage::new([n, n], pixels), egui::TextureOptions::LINEAR_REPEAT)
    })
    .id()
}

/// Legt das Korn von gestrahltem Aluminium über eine Fläche. `dark` = dunkler Skin.
pub fn grain_overlay(p: &Painter, rect: Rect, radius: f32, dark: bool) {
    let pts = rrect_points(rect, radius);
    if pts.len() < 3 {
        return;
    }
    let ppp = p.pixels_per_point();
    let uv = |q: Pos2| pos2(q.x * ppp / GRAIN as f32, q.y * ppp / GRAIN as f32);
    let i = if dark { 17 } else { 20 };
    let tint = Color32::from_rgba_premultiplied(i, i, i, i);
    let mut mesh = Mesh::with_texture(grain_texture(p.ctx()));
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

/// Gefaste Lichtkante (wie diamantgedrehte Kanten an Aluminiumgehäusen): eine feine helle Linie
/// rundum, oben am hellsten, nach unten auslaufend. `top`/`bottom` = Deckkraft des Weiß.
fn chamfer(p: &Painter, rect: Rect, radius: f32, top: u8, bottom: u8) {
    let pts = rrect_points(rect, radius);
    let n = pts.len();
    if n < 2 {
        return;
    }
    let h = rect.height().max(1.0);
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let t = (((a.y + b.y) / 2.0 - rect.top()) / h).clamp(0.0, 1.0);
        // Oben kräftig, schnell auf den unteren Wert
        let alpha = bottom as f32 + (top as f32 - bottom as f32) * (1.0 - t).powi(3);
        p.line_segment([a, b], Stroke::new(1.0, Color32::from_white_alpha(alpha as u8)));
    }
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
    } else if theme::alu() {
        grain_overlay(p, rect, radius, theme::skin().dark);
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
    if theme::alu() {
        alu_panel(p, rect);
        return;
    }
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

/// Gehäuse aus eloxiertem Aluminium: weicher, tiefer Schatten, kaum Verlauf, gestrahltes Korn,
/// dunkle Außenkante und eine gefaste Lichtkante rundum.
fn alu_panel(p: &Painter, rect: Rect) {
    let s = theme::skin();
    let r = 10.0;
    shadow(p, rect, r, 8, 26, if s.dark { 190 } else { 70 });
    let face = if theme::brushed() { steel_face(s.panel, s.dark) } else { s.panel };
    let (top, bottom) = if s.dark { (lighten(face, 0.05), darken(face, 0.12)) } else { (lighten(face, 0.30), darken(face, 0.05)) };
    surface(p, rect, r, top, bottom);
    p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_black_alpha(if s.dark { 220 } else { 80 })), StrokeKind::Inside);
    chamfer(p, rect.shrink(1.0), r - 1.0, if s.dark { 46 } else { 235 }, if s.dark { 8 } else { 70 });
}

/// Grundfarbe des Plattentellers (Metallrand).
pub fn platter_rim() -> Color32 {
    let s = theme::skin();
    match (theme::alu(), s.dark) {
        (true, false) => Color32::from_rgb(0xbd, 0xc1, 0xc7),
        (true, true) => Color32::from_rgb(0x3a, 0x3c, 0x40),
        _ => Color32::from_rgb(0x2c, 0x34, 0x46),
    }
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
    if theme::alu() {
        // Schwarzes Glas hinter einer gefrästen Öffnung: Lichtkante unten außen, oben ein schwacher Spiegelglanz
        let dark = theme::skin().dark;
        p.rect_stroke(rect.expand(1.0), radius + 1.0, Stroke::new(1.0, Color32::from_black_alpha(if dark { 200 } else { 90 })), StrokeKind::Inside);
        gradient_rrect(p, rect, radius, lighten(w, 0.06), w);
        let gloss = Rect::from_min_max(rect.min + vec2(1.0, 1.0), pos2(rect.right() - 1.0, rect.top() + (rect.height() * 0.45).min(28.0)));
        gradient_rrect(p, gloss, (radius - 1.0).max(0.0), Color32::from_white_alpha(14), Color32::from_white_alpha(0));
        p.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha(10)), StrokeKind::Inside);
        p.line_segment(
            [pos2(rect.left() + radius, rect.bottom() + 1.5), pos2(rect.right() - radius, rect.bottom() + 1.5)],
            Stroke::new(1.0, Color32::from_white_alpha(if dark { 26 } else { 190 })),
        );
        return;
    }
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
    if theme::alu() {
        return paint_alu_face(p, rect, look, lit, pressed, hovered, enabled);
    }
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

/// Taste aus gefrästem Aluminium wie an High-End-HiFi: flache, satinierte Fläche mit Lichtkante, die Farbe
/// zeigt sich nur in der Beschriftung und in einem schmalen LED-Fenster unten (leuchtet bei `lit`). Kleine Tasten
/// ohne Platz für das Fenster bekommen beim Leuchten eine feine farbige Kontur.
fn paint_alu_face(p: &Painter, rect: Rect, look: Look, lit: bool, pressed: bool, hovered: bool, enabled: bool) -> (Pos2, Color32) {
    let s = theme::skin();
    let r = (rect.height() * 0.2).clamp(3.0, 6.0);
    let metal = s.panel2;
    let (mut top, mut bottom) = if s.dark { (lighten(metal, 0.09), darken(metal, 0.20)) } else { (lighten(metal, 0.65), darken(metal, 0.06)) };
    if hovered && enabled && !pressed {
        top = lighten(top, 0.05);
        bottom = lighten(bottom, 0.04);
    }
    if pressed {
        // Eingedrückt: Licht kommt nicht mehr von oben, Fläche flacher und etwas dunkler
        (top, bottom) = (darken(bottom, 0.04), mix(top, bottom, 0.5));
    }
    // Weiß für neutrale Tasten, die leuchten (das Fenster ist schwarz, darum auch auf Silber sichtbar)
    let led = match look {
        Look::Plain => Color32::from_rgb(0xea, 0xf3, 0xff),
        Look::Color(c) => c,
    };
    let accent = if s.dark { lighten(led, 0.12) } else { darken(led, 0.12) };
    let mut text = match look {
        Look::Plain => s.ink,
        Look::Color(_) if lit => accent,
        Look::Color(_) => mix(s.ink, accent, 0.7),
    };
    if !enabled {
        text = text.gamma_multiply(0.4);
    }

    if !pressed {
        shadow(p, rect, r, 2, 6, if s.dark { 170 } else { 60 });
    }
    gradient_rrect(p, rect, r, top, bottom);
    p.rect_stroke(rect, r, Stroke::new(1.0, Color32::from_black_alpha(if s.dark { 210 } else { 75 })), StrokeKind::Inside);
    if pressed {
        p.line_segment(
            [pos2(rect.left() + r, rect.top() + 1.5), pos2(rect.right() - r, rect.top() + 1.5)],
            Stroke::new(1.0, Color32::from_black_alpha(if s.dark { 120 } else { 40 })),
        );
    } else {
        chamfer(p, rect.shrink(1.0), r - 1.0, if s.dark { 52 } else { 255 }, if s.dark { 6 } else { 60 });
    }

    let window = matches!(look, Look::Color(_)) && rect.height() >= 24.0 && rect.width() >= 26.0;
    let mut c = rect.center() + if pressed { vec2(0.0, 0.5) } else { vec2(0.0, 0.0) };
    if window {
        // LED-Fenster: schwarzer Schlitz mit Lichtkante, darin der Leuchtstreifen
        let w = (rect.width() * 0.32).clamp(12.0, 30.0);
        let slot = Rect::from_center_size(pos2(c.x, rect.bottom() - 5.5), vec2(w, 3.0));
        p.rect_filled(slot.expand(1.0), 2.0, Color32::from_black_alpha(if s.dark { 230 } else { 150 }));
        p.line_segment([pos2(slot.left(), slot.bottom() + 1.5), pos2(slot.right(), slot.bottom() + 1.5)], Stroke::new(1.0, Color32::from_white_alpha(if s.dark { 22 } else { 160 })));
        if lit {
            if !theme::lite() {
                for k in 1..=4 {
                    p.rect_filled(slot.expand(k as f32 * 1.4), 2.0 + k as f32, led.gamma_multiply(0.16 / k as f32));
                }
            }
            p.rect_filled(slot, 1.5, lighten(led, 0.18));
            p.line_segment([pos2(slot.left() + 1.0, slot.top() + 0.5), pos2(slot.right() - 1.0, slot.top() + 0.5)], Stroke::new(1.0, lighten(led, 0.6)));
        } else {
            p.rect_filled(slot, 1.5, led.gamma_multiply(0.16));
        }
        c.y -= 3.0;
    } else if lit {
        p.rect_stroke(rect.shrink(1.0), r - 1.0, Stroke::new(1.5, led.gamma_multiply(0.85)), StrokeKind::Inside);
        if !theme::lite() {
            p.rect_filled(rect.shrink(2.0), r - 2.0, led.gamma_multiply(0.07));
        }
    }
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

/// Taste mit selbst gezeichnetem Inhalt: `draw` bekommt die Tastenfläche (beim Drücken leicht versetzt)
/// und die Farbe für Beschriftung oder Symbol.
pub fn custom_button(ui: &mut egui::Ui, size: egui::Vec2, look: Look, lit: bool, enabled: bool, draw: impl FnOnce(&Painter, Rect, Color32)) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    let p = ui.painter();
    let (c, color) = paint_button_face(p, rect, look, lit, pressed, resp.hovered(), enabled);
    draw(p, Rect::from_center_size(c, rect.size()), color);
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
    if theme::alu() {
        alu_fader_cap(p, cap, vertical);
        return;
    }
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

/// Faderkappe als gefräster Aluminiumblock: satinierte Fläche, Lichtkante, in der Mitte eine eingelassene Markierung.
/// Auf Silber hell mit dunkler Linie, auf Anthrazit dunkel eloxiert mit weißer Linie.
fn alu_fader_cap(p: &Painter, cap: Rect, vertical: bool) {
    let dark = theme::skin().dark;
    let r = 2.5;
    shadow(p, cap, r, 4, 8, if dark { 200 } else { 110 });
    let (top, bottom, mark) = if dark {
        (Color32::from_rgb(0x55, 0x58, 0x5e), Color32::from_rgb(0x2c, 0x2e, 0x32), Color32::from_rgb(0xf4, 0xf5, 0xf7))
    } else {
        (Color32::from_rgb(0xf7, 0xf8, 0xfa), Color32::from_rgb(0xb7, 0xbb, 0xc2), Color32::from_rgb(0x1d, 0x1d, 0x1f))
    };
    gradient_rrect(p, cap, r, top, bottom);
    grain_overlay(p, cap, r, dark);
    p.rect_stroke(cap, r, Stroke::new(1.0, Color32::from_black_alpha(if dark { 230 } else { 120 })), StrokeKind::Inside);
    chamfer(p, cap.shrink(1.0), r - 1.0, if dark { 70 } else { 255 }, if dark { 10 } else { 90 });
    // Quer laufende Fase in der Mitte (Griffmulde) mit Markierungslinie
    let c = cap.center();
    if vertical {
        let (l, rr) = (cap.left() + 2.0, cap.right() - 2.0);
        p.line_segment([pos2(l, c.y - 1.5), pos2(rr, c.y - 1.5)], Stroke::new(1.0, Color32::from_black_alpha(if dark { 120 } else { 45 })));
        p.line_segment([pos2(l, c.y), pos2(rr, c.y)], Stroke::new(1.5, mark));
        p.line_segment([pos2(l, c.y + 1.5), pos2(rr, c.y + 1.5)], Stroke::new(1.0, Color32::from_white_alpha(if dark { 30 } else { 200 })));
    } else {
        let (t, b) = (cap.top() + 2.0, cap.bottom() - 2.0);
        p.line_segment([pos2(c.x - 1.5, t), pos2(c.x - 1.5, b)], Stroke::new(1.0, Color32::from_black_alpha(if dark { 120 } else { 45 })));
        p.line_segment([pos2(c.x, t), pos2(c.x, b)], Stroke::new(1.5, mark));
        p.line_segment([pos2(c.x + 1.5, t), pos2(c.x + 1.5, b)], Stroke::new(1.0, Color32::from_white_alpha(if dark { 30 } else { 200 })));
    }
}

/// Schlitz, in dem ein Fader läuft. Aluminium: schmaler, wie gefräst.
fn fader_slot(p: &Painter, rect: Rect, vertical: bool) {
    if theme::alu() {
        let slot = if vertical { Rect::from_center_size(rect.center(), vec2(4.0, rect.height())) } else { Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0)) };
        p.rect_filled(slot, 2.0, Color32::from_rgb(0x05, 0x05, 0x06));
        let light = Color32::from_white_alpha(if theme::skin().dark { 24 } else { 170 });
        if vertical {
            p.line_segment([pos2(slot.right() + 0.5, slot.top() + 2.0), pos2(slot.right() + 0.5, slot.bottom() - 2.0)], Stroke::new(1.0, light));
        } else {
            p.line_segment([pos2(slot.left() + 2.0, slot.bottom() + 0.5), pos2(slot.right() - 2.0, slot.bottom() + 0.5)], Stroke::new(1.0, light));
        }
        return;
    }
    let slot = if vertical { Rect::from_center_size(rect.center(), vec2(6.0, rect.height())) } else { Rect::from_center_size(rect.center(), vec2(rect.width(), 6.0)) };
    gradient_rrect(p, slot, 3.0, Color32::from_rgb(0, 0, 0), Color32::from_rgb(24, 28, 36));
}

/// Mausrad bzw. Trackpad: so viele Punkte Weg ergeben einen Schritt (eine Rastung des Rads ist immer einer).
const WHEEL_POINTS_PER_STEP: f32 = 24.0;
/// Begann das Scrollen außerhalb eines Reglers (Mittelspalte, Liste), scrollt es so lange weiter durch,
/// bis so viele Sekunden Ruhe war; erst dann greift das Rad über einem Regler.
const WHEEL_LATCH_SECS: f64 = 0.4;

/// Zeitpunkt des letzten Scrollens, das kein Regler genommen hat.
#[derive(Clone, Copy)]
struct ForeignScroll(f64);

/// Am Ende jedes Bilds aufrufen: übrig gebliebene Rad-Ereignisse gehören einer Liste oder Spalte,
/// die gerade gescrollt wird. Solange das anhält, lassen die Regler das Rad durch.
pub fn note_foreign_scroll(ctx: &egui::Context) {
    let (scrolled, now) = ctx.input(|i| (i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })), i.time));
    if scrolled {
        ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, ForeignScroll(now)));
    }
}

/// Ist unter macOS „natürliches Scrollen“ an? (Standard dort; anderswo meist aus.)
/// Davon hängt ab, welches Vorzeichen „Rad/Finger nach oben“ hat.
fn natural_scrolling() -> bool {
    static NATURAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NATURAL.get_or_init(|| {
        if !cfg!(target_os = "macos") {
            return false;
        }
        // Fehlt der Eintrag, ist die Voreinstellung (an) aktiv
        std::process::Command::new("defaults")
            .args(["read", "-g", "com.apple.swipescrolldirection"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() != "0")
            .unwrap_or(true)
    })
}

/// Bewegung von Mausrad bzw. Trackpad über `resp` in diesem Bild, nach oben oder rechts positiv
/// (wie die Finger sich bewegen). `points` = Trackpad-Weg, `notches` = Rastungen eines Mausrads
/// (je Ereignis höchstens 3, auch wenn das System mehrere Zeilen meldet). Die Ereignisse werden verbraucht,
/// damit dahinter nichts scrollt. Nichts, solange ein woanders begonnenes Scrollen noch läuft, und nicht mit ⌘/Strg/⌥.
#[derive(Default, Clone, Copy)]
pub struct Wheel {
    pub points: f32,
    pub notches: f32,
}

pub fn take_wheel(ui: &egui::Ui, resp: &Response) -> Wheel {
    let mut w = Wheel::default();
    if !resp.hovered() {
        return w;
    }
    let latched = ui.ctx().data(|d| d.get_temp::<ForeignScroll>(egui::Id::NULL)).is_some_and(|f| ui.input(|i| i.time) - f.0 < WHEEL_LATCH_SECS);
    if latched {
        return w;
    }
    let natural = natural_scrolling();
    ui.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::MouseWheel { unit, delta, modifiers, .. } = e else { return true };
            if modifiers.command || modifiers.ctrl || modifiers.alt {
                return true;
            }
            // Inhalt-Richtung → Finger-Richtung: mit natürlichem Scrollen folgt der Inhalt den Fingern
            let up = if natural { -delta.y + delta.x } else { delta.y - delta.x };
            match unit {
                egui::MouseWheelUnit::Point => w.points += up,
                _ => w.notches += up.signum() * up.abs().round().clamp(1.0, 3.0),
            }
            false
        });
        // Nachlaufendes, geglättetes Scrollen soll die Spalte dahinter nicht bewegen
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    w
}

/// Schritte für einen Regler, solange die Maus darüber steht: Pfeiltasten (↑/→ = +1, ↓/← = −1, gehalten
/// mit Wiederholung) und Mausrad/Trackpad (siehe [`take_wheel`]; eine Rastung = ein Schritt).
/// Pfeiltasten nicht mit ⌘/Strg/⌥ und nicht, während ein Textfeld tippt.
pub fn nudge_steps(ui: &egui::Ui, resp: &Response) -> i32 {
    if !resp.hovered() {
        return 0;
    }
    let mut n = 0;
    if !ui.ctx().egui_wants_keyboard_input() {
        ui.input_mut(|i| {
            i.events.retain(|e| {
                let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { return true };
                if modifiers.command || modifiers.ctrl || modifiers.alt {
                    return true;
                }
                match key {
                    egui::Key::ArrowUp | egui::Key::ArrowRight => n += 1,
                    egui::Key::ArrowDown | egui::Key::ArrowLeft => n -= 1,
                    _ => return true,
                }
                false
            });
        });
    }
    let wheel = take_wheel(ui, resp);
    let acc_id = resp.id.with("wheel");
    let mut acc: f32 = ui.ctx().data(|d| d.get_temp(acc_id)).unwrap_or(0.0);
    acc += wheel.points / WHEEL_POINTS_PER_STEP;
    let whole = acc.trunc();
    ui.ctx().data_mut(|d| d.insert_temp(acc_id, acc - whole));
    n + whole as i32 + wheel.notches as i32
}

/// Neuer Wert nach Pfeiltasten oder Mausrad über `resp`: `value` um `step` je Schritt, begrenzt auf `min`…`max`.
pub fn nudge(ui: &egui::Ui, resp: &Response, value: f32, step: f32, min: f32, max: f32) -> Option<f32> {
    let n = nudge_steps(ui, resp);
    (n != 0).then(|| (value + n as f32 * step).clamp(min, max))
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
    fader_slot(p, Rect::from_center_size(rect.center(), vec2(6.0, rect.height() - 10.0)), true);
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
    if theme::alu() {
        let ring = r - 1.0;
        arc(0.0, 1.0, ring, Stroke::new(1.5, if s.dark { Color32::from_black_alpha(200) } else { Color32::from_black_alpha(55) }));
        arc(default, v, ring, Stroke::new(1.8, color));
        p.line_segment([at(angle(default), ring - 2.0), at(angle(default), ring + 1.0)], Stroke::new(1.0, theme::faint()));
        turned_knob(p, c, r * 0.74, angle(v), s.dark);
        return (resp.on_hover_cursor(egui::CursorIcon::ResizeVertical), out);
    }
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

/// Gedrehter Aluminiumknopf: kreisförmige Drehriefen werfen das Licht als Kreuz zurück (anisotroper Glanz),
/// außen eine gefaste Kante, innen eine eingelassene Punktmarke bei Winkel `a` (0 = oben).
fn turned_knob(p: &Painter, c: Pos2, body: f32, a: f32, dark: bool) {
    let lite = theme::lite();
    let (base, rim_light) = if dark { (Color32::from_rgb(0x3e, 0x40, 0x45), 70) } else { (Color32::from_rgb(0xd2, 0xd5, 0xda), 255) };
    // Schatten unter dem Knopf
    if !lite {
        for k in 0..4 {
            p.circle_filled(c + vec2(0.0, 2.0 + k as f32 * 0.6), body + 2.0 - k as f32 * 0.6, Color32::from_black_alpha(if dark { 45 } else { 22 }));
        }
    }
    // Seitenwand (Rändel): dunkler Ring, oben etwas Licht
    p.circle_filled(c, body, darken(base, if dark { 0.45 } else { 0.30 }));
    let face_r = body * 0.86;
    // Stirnfläche als Fächer mit Farbe je Winkel: hell bei ±45°/±135° (Licht von oben links), dunkel dazwischen
    let n = if lite { 24 } else { 72 };
    let light = |t: f32| {
        let k = (2.0 * (t + 0.35)).cos();
        let k = k * k.abs();
        if k >= 0.0 { lighten(base, k * if dark { 0.30 } else { 0.55 }) } else { darken(base, -k * if dark { 0.35 } else { 0.18 }) }
    };
    let mut mesh = Mesh::default();
    mesh.colored_vertex(c, base);
    for i in 0..=n {
        let t = i as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(c + vec2(t.cos(), t.sin()) * face_r, light(t));
        if i > 0 {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
    }
    p.add(mesh);
    // Feine Drehriefen
    if !lite {
        let rings = ((face_r / 1.6) as usize).min(24);
        for k in 1..rings {
            let rr = face_r * k as f32 / rings as f32;
            let col = if k % 2 == 0 { Color32::from_white_alpha(if dark { 7 } else { 30 }) } else { Color32::from_black_alpha(if dark { 14 } else { 8 }) };
            p.circle_stroke(c, rr, Stroke::new(0.8, col));
        }
    }
    // Fase zwischen Stirn und Rändel
    p.circle_stroke(c, face_r, Stroke::new(1.0, Color32::from_white_alpha(if dark { 40 } else { rim_light })));
    p.circle_stroke(c, body, Stroke::new(1.0, Color32::from_black_alpha(if dark { 220 } else { 110 })));
    // Punktmarke
    let dot = c + vec2(a.sin(), -a.cos()) * face_r * 0.68;
    let dot_r = (body * 0.09).max(1.6);
    p.circle_filled(dot + vec2(0.0, 0.5), dot_r + 0.5, Color32::from_white_alpha(if dark { 20 } else { 140 }));
    p.circle_filled(dot, dot_r, if dark { Color32::from_rgb(0xf4, 0xf5, 0xf7) } else { Color32::from_rgb(0x1d, 0x1d, 0x1f) });
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
    fader_slot(p, Rect::from_center_size(rect.center(), vec2(rect.width() - 10.0, 6.0)), false);
    let cap_x = rect.left() + cap_w / 2.0 + (value.clamp(-1.0, 1.0) + 1.0) / 2.0 * travel;
    fader_cap(p, pos2(cap_x, rect.center().y), vec2(cap_w, rect.height() - 6.0), false);

    let mut out = None;
    if (resp.dragged() || resp.clicked()) && let Some(pos) = resp.interact_pointer_pos() {
        out = Some(((pos.x - rect.left() - cap_w / 2.0) / travel * 2.0 - 1.0).clamp(-1.0, 1.0));
    }
    let out = glide_reset(ui, &resp, value, default, 2.0, out);
    (resp, out)
}
