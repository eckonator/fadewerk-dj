// SPDX-License-Identifier: GPL-3.0-or-later
//! Fertige Bilder für abgerundete Flächen (Tasten, Gehäuse, Displays, Faderkappen).
//!
//! Schatten, Verlauf, Rahmen und Lichtkante einer Taste waren zusammen einige Hundert Vertices, die egui jedes Bild
//! neu zerlegte und hochlud. Quer zur Fläche ändert sich in der Mitte nichts (alles hängt nur von der Höhe ab), darum
//! reicht ein Bild aus zwei Endkappen und einer Pixelspalte dazwischen, die auf die Breite gestreckt wird: drei
//! Rechtecke, 12 Vertices. Das Bild entsteht einmal je Aussehen und Höhe und liegt in einem Zwischenspeicher.
//! Was nicht quer gleich bleibt (LED-Fenster in der Mitte einer Taste), bekommt ein eigenes, schmales Bild.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use eframe::egui::{self, Color32, Painter, Rect, Stroke, StrokeKind, pos2, vec2};

use crate::hw;
use crate::theme::{self, mix};

/// Eine Schicht des Bildes. Abstände in Punkten, bezogen auf das Rechteck der Fläche; `inset` ist der Abstand einer
/// Schicht zum Rand (negativ: größer als die Fläche).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Layer {
    /// Weicher Schatten wie egui `Shadow` (ohne `spread`): um `dy` nach unten versetzt, Übergang `blur` breit.
    Shadow { radius: f32, dy: f32, blur: f32, color: Color32 },
    /// Fläche mit senkrechtem Verlauf über ihre eigene Höhe (deckend wie `hw::gradient_rrect`); `top == bottom` ist
    /// einfarbig und behält die Deckkraft. `inset` = links/rechts, oben, unten.
    Fill { inset: [f32; 3], radius: f32, top: Color32, bottom: Color32 },
    /// Strich innen an der Kante (`outside`: außen herum), wie `rect_stroke` mit `StrokeKind::Inside/Outside`.
    Stroke { inset: f32, radius: f32, width: f32, color: Color32, outside: bool },
    /// Gefaste Lichtkante: 1 pt mittig auf der Kante, Weiß mit Deckkraft `top` oben, nach unten schnell auf `bottom`.
    Chamfer { inset: f32, radius: f32, top: u8, bottom: u8 },
    /// Waagerechte Linie `x_inset` vor beiden Seiten, Mitte bei `y` unter der Oberkante (`from_bottom`: unter der Unterkante).
    HLine { y: f32, from_bottom: bool, x_inset: f32, width: f32, color: Color32 },
}

impl Layer {
    fn feed(&self, h: &mut impl Hasher) {
        let mut f = |v: f32| v.to_bits().hash(h);
        match *self {
            Layer::Shadow { radius, dy, blur, color } => {
                f(0.0);
                f(radius);
                f(dy);
                f(blur);
                color.hash(h);
            }
            Layer::Fill { inset, radius, top, bottom } => {
                f(1.0);
                inset.iter().for_each(|&v| f(v));
                f(radius);
                top.hash(h);
                bottom.hash(h);
            }
            Layer::Stroke { inset, radius, width, color, outside } => {
                f(2.0);
                f(inset);
                f(radius);
                f(width);
                color.hash(h);
                outside.hash(h);
            }
            Layer::Chamfer { inset, radius, top, bottom } => {
                f(3.0);
                f(inset);
                f(radius);
                (top, bottom).hash(h);
            }
            Layer::HLine { y, from_bottom, x_inset, width, color } => {
                f(4.0);
                f(y);
                f(x_inset);
                f(width);
                from_bottom.hash(h);
                color.hash(h);
            }
        }
    }

    /// So weit reicht die Schicht über die Fläche hinaus: links/rechts, oben, unten (Punkte).
    fn overhang(&self, height: f32) -> [f32; 3] {
        match *self {
            Layer::Shadow { dy, blur, .. } => [blur / 2.0, blur / 2.0 - dy, blur / 2.0 + dy],
            Layer::Fill { inset, .. } => [-inset[0], -inset[1], -inset[2]],
            Layer::Stroke { inset, width, outside, .. } => {
                let o = if outside { width - inset } else { -inset };
                [o, o, o]
            }
            Layer::Chamfer { inset, .. } => [0.5 - inset; 3],
            Layer::HLine { y, from_bottom, width, .. } => {
                let mid = if from_bottom { height + y } else { y };
                [0.0, width / 2.0 - mid, mid + width / 2.0 - height]
            }
        }
    }

    /// Bis hierher (vom Rand nach innen, Punkte) unterscheiden sich die Spalten; dahinter ist die Schicht quer gleich.
    fn reach(&self) -> f32 {
        match *self {
            Layer::Shadow { radius, blur, .. } => radius + blur,
            Layer::Fill { inset, radius, .. } => inset[0] + radius,
            Layer::Stroke { inset, radius, .. } | Layer::Chamfer { inset, radius, .. } => inset + radius,
            Layer::HLine { x_inset, .. } => x_inset + 1.0,
        }
    }
}

/// Zeichnet die Fläche `rect` mit ihren Schichten als fertiges Bild.
pub fn paint(p: &Painter, rect: Rect, layers: &[Layer]) {
    if layers.is_empty() || !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // Einfache Formen (sparsam): dort ist alles eckig und ohne Kantenglättung, die Bilder sparten kaum Vertices und
    // brächten nur mehr Zeichenaufträge
    if theme::lite().shapes {
        paint_shapes(p, rect, layers);
        return;
    }
    let ppp = p.pixels_per_point();
    // Überstand außen (Punkte), ein Pixel Luft für die weichen Kanten
    let mut over = [0.0f32; 3];
    let mut reach = 0.0f32;
    for l in layers {
        let o = l.overhang(rect.height());
        for k in 0..3 {
            over[k] = over[k].max(o[k]);
        }
        reach = reach.max(l.reach());
    }
    let air = 1.0 / ppp;
    let [ox, ot, ob] = over.map(|o| o.max(0.0) + air);
    // Auf Pixel gelegt wie egui (`round_rects_to_pixels`): scharfe 1-pt-Linien
    let snap = |v: f32| (v * ppp).round() / ppp;
    let outer = Rect::from_min_max(pos2(snap(rect.left() - ox), snap(rect.top() - ot)), pos2(snap(rect.right() + ox), snap(rect.bottom() + ob)));
    let rows = (outer.height() * ppp).round().max(1.0) as usize;
    let full = (outer.width() * ppp).round().max(1.0) as usize;
    let cap = ((reach + ox) * ppp).ceil() as usize + 1;
    // Breit genug: Endkappen + eine Mittelspalte; sonst das ganze Bild in voller Breite
    let wide = full > 2 * cap + 1;
    let cols = if wide { 2 * cap + 1 } else { full };
    // Höher als jeder Bildschirm (Hintergrund einer langen Liste) oder größer als die Grafikkarte erlaubt (GeForce
    // 8600M: 8192 px, egui bricht dann ab): als Formen zeichnen
    let max_side = p.ctx().input(|i| i.max_texture_side).min(MAX_IMAGE_SIDE);
    if rows > max_side || cols > max_side {
        paint_shapes(p, rect, layers);
        return;
    }

    let mut h = std::collections::hash_map::DefaultHasher::new();
    layers.iter().for_each(|l| l.feed(&mut h));
    (ppp.to_bits(), rows, ox.to_bits(), ot.to_bits(), rect.height().to_bits()).hash(&mut h);
    if !wide {
        (cols, rect.width().to_bits()).hash(&mut h);
    }
    let key = h.finish();

    let ctx = p.ctx();
    let frame = ctx.cumulative_frame_nr();
    let id = egui::Id::new("flaechen-bilder");
    let found = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Cache>(id);
        cache.map.get_mut(&key).map(|e| {
            e.used = frame;
            (e.tex, e.uv)
        })
    });
    let (tex, uv_box) = match found {
        Some(f) => f,
        None => {
            let geo = Geometry { ppp, ox, ot, height: rect.height(), half_w: if wide { f32::INFINITY } else { rect.width() / 2.0 }, cap, wide };
            let image = rasterize(&geo, rows, cols, layers);
            store(ctx, id, key, image, frame)
        }
    };

    let mut mesh = egui::Mesh::with_texture(tex);
    let white = Color32::WHITE;
    // Ausschnitt `u0..u1` (Anteil der Bildbreite) im Atlas
    let uv = |u0: f32, u1: f32| Rect::from_x_y_ranges(uv_box.lerp_inside(vec2(u0, 0.0)).x..=uv_box.lerp_inside(vec2(u1, 0.0)).x, uv_box.y_range());
    if wide {
        let cw = cap as f32 / ppp;
        let n = cols as f32;
        let (l, r) = (outer.left() + cw, outer.right() - cw);
        let mid = (cap as f32 + 0.5) / n;
        mesh.add_rect_with_uv(Rect::from_x_y_ranges(outer.left()..=l, outer.y_range()), uv(0.0, cap as f32 / n), white);
        mesh.add_rect_with_uv(Rect::from_x_y_ranges(l..=r, outer.y_range()), uv(mid, mid), white);
        mesh.add_rect_with_uv(Rect::from_x_y_ranges(r..=outer.right(), outer.y_range()), uv((cap + 1) as f32 / n, 1.0), white);
    } else {
        mesh.add_rect_with_uv(outer, uv(0.0, 1.0), white);
    }
    p.add(mesh);
}

/// Größere Flächen (Pixel je Seite) liegen weit außerhalb des Bildschirms; ein Bild lohnt dort nicht.
const MAX_IMAGE_SIDE: usize = 4096;

/// Dieselben Schichten als einzelne egui-Formen, wie sie vor den Bildern gezeichnet wurden.
fn paint_shapes(p: &Painter, rect: Rect, layers: &[Layer]) {
    let inset = |l: f32, t: f32, b: f32| Rect::from_min_max(pos2(rect.left() + l, rect.top() + t), pos2(rect.right() - l, rect.bottom() - b));
    for layer in layers {
        match *layer {
            Layer::Shadow { radius, dy, blur, color } => {
                let s = egui::Shadow { offset: [0, dy as i8], blur: blur as u8, spread: 0, color };
                p.add(s.as_shape(rect, egui::CornerRadius::same(radius as u8)));
            }
            Layer::Fill { inset: [l, t, b], radius, top, bottom } if top == bottom => {
                p.rect_filled(inset(l, t, b), radius, top);
            }
            Layer::Fill { inset: [l, t, b], radius, top, bottom } => hw::gradient_rrect(p, inset(l, t, b), radius, top, bottom),
            Layer::Stroke { inset: i, radius, width, color, outside } => {
                let kind = if outside { StrokeKind::Outside } else { StrokeKind::Inside };
                p.rect_stroke(inset(i, i, i), radius, Stroke::new(width, color), kind);
            }
            Layer::Chamfer { inset: i, radius, top, bottom } => {
                let r = inset(i, i, i);
                let pts = hw::rrect_points(r, radius);
                let n = pts.len();
                let h = r.height().max(1.0);
                for k in 0..n {
                    let (a, b) = (pts[k], pts[(k + 1) % n]);
                    let t = (((a.y + b.y) / 2.0 - r.top()) / h).clamp(0.0, 1.0);
                    let alpha = bottom as f32 + (top as f32 - bottom as f32) * (1.0 - t).powi(3);
                    p.line_segment([a, b], Stroke::new(1.0, Color32::from_white_alpha(alpha as u8)));
                }
            }
            Layer::HLine { y, from_bottom, x_inset, width, color } => {
                let y = if from_bottom { rect.bottom() + y } else { rect.top() + y };
                p.line_segment([pos2(rect.left() + x_inset, y), pos2(rect.right() - x_inset, y)], Stroke::new(width, color));
            }
        }
    }
}

/// Kantenlänge des gemeinsamen Atlas in Pixeln. Alle Flächenbilder in einer Textur: egui fasst aufeinanderfolgende
/// Formen mit derselben Textur zu einem Zeichenauftrag zusammen, mit je einer Textur pro Bild waren es mehr als doppelt
/// so viele Aufträge wie vorher.
const ATLAS: usize = 2048;
/// Höhere Bilder (Gehäuse auf großen Bildschirmen) bekommen eine eigene Textur.
const ATLAS_MAX_H: usize = 1024;
/// Eigene Texturen ohne Gebrauch fliegen nach so vielen Bildern raus (beim Ziehen am Fenster entstehen viele Höhen).
const OWN_FRAMES: u64 = 120;

#[derive(Clone)]
struct Entry {
    tex: egui::TextureId,
    /// Lage im Atlas bzw. in der eigenen Textur (0..1).
    uv: Rect,
    /// Nur bei eigener Textur: hält sie am Leben.
    own: Option<egui::TextureHandle>,
    used: u64,
}

#[derive(Clone, Default)]
struct Cache {
    map: HashMap<u64, Entry>,
    atlas: Option<egui::TextureHandle>,
    /// Nächste freie Stelle in der aktuellen Zeile, Oberkante und Höhe der Zeile.
    cursor: (usize, usize),
    row_h: usize,
}

impl Cache {
    /// Platz im Atlas. Ist er voll, beginnt er von vorn; die alten Bilder entstehen bei Bedarf neu.
    fn allocate(&mut self, w: usize, h: usize) -> (usize, usize) {
        if self.cursor.0 + w > ATLAS {
            self.cursor = (0, self.cursor.1 + self.row_h + 1);
            self.row_h = 0;
        }
        if self.cursor.1 + h > ATLAS {
            self.map.retain(|_, e| e.own.is_some());
            self.cursor = (0, 0);
            self.row_h = 0;
        }
        let pos = self.cursor;
        self.cursor.0 += w + 1;
        self.row_h = self.row_h.max(h);
        pos
    }
}

/// Legt ein neues Bild ab (im Atlas oder, wenn zu groß, als eigene Textur) und liefert Textur und Lage.
fn store(ctx: &egui::Context, id: egui::Id, key: u64, image: egui::ColorImage, frame: u64) -> (egui::TextureId, Rect) {
    let [w, h] = image.size;
    // Texturen anlegen außerhalb von `data_mut`: `load_texture` braucht selbst den Kontext
    if h > ATLAS_MAX_H || w > ATLAS {
        let handle = ctx.load_texture("flaeche", image, egui::TextureOptions::LINEAR);
        let entry = Entry { tex: handle.id(), uv: Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), own: Some(handle), used: frame };
        let found = (entry.tex, entry.uv);
        ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<Cache>(id);
            cache.map.retain(|_, e| e.own.is_none() || frame.saturating_sub(e.used) < OWN_FRAMES);
            cache.map.insert(key, entry);
        });
        return found;
    }
    if ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(id).atlas.is_none()) {
        let atlas = ctx.load_texture("flaechen", egui::ColorImage::filled([ATLAS, ATLAS], Color32::TRANSPARENT), egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(id).atlas = Some(atlas));
    }
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<Cache>(id);
        let (x, y) = cache.allocate(w, h);
        let Some(atlas) = cache.atlas.as_mut() else { return (egui::TextureId::default(), Rect::NOTHING) };
        atlas.set_partial([x, y], image, egui::TextureOptions::LINEAR);
        let n = ATLAS as f32;
        let uv = Rect::from_min_size(pos2(x as f32 / n, y as f32 / n), vec2(w as f32 / n, h as f32 / n));
        let tex = atlas.id();
        cache.map.insert(key, Entry { tex, uv, own: None, used: frame });
        (tex, uv)
    })
}

/// Lage des Bildes zur Fläche: Pixel je Punkt, Überstand links und oben, Höhe und halbe Breite der Fläche (Punkte),
/// Breite einer Endkappe (Pixel).
struct Geometry {
    ppp: f32,
    ox: f32,
    ot: f32,
    height: f32,
    half_w: f32,
    cap: usize,
    wide: bool,
}

/// Abstand zur Kante eines abgerundeten Rechtecks (negativ innen). `dx`: Abstand zur nächsten Seitenkante der Fläche
/// nach innen, `y` ab Oberkante der Fläche, beides in Punkten; das Rechteck ist um `ix`, `top`, `bottom` eingerückt.
fn rrect_dist(dx: f32, y: f32, g: &Geometry, ix: f32, top: f32, bottom: f32, radius: f32) -> f32 {
    let hh = ((g.height - top - bottom) / 2.0).max(0.0);
    let hw = g.half_w - ix;
    let r = radius.min(hh).min(hw).max(0.0);
    let qx = r - (dx - ix);
    let qy = (y - (top + hh)).abs() - (hh - r);
    vec2(qx.max(0.0), qy.max(0.0)).length() + qx.max(qy).min(0.0) - r
}

fn rasterize(g: &Geometry, rows: usize, cols: usize, layers: &[Layer]) -> egui::ColorImage {
    let ppp = g.ppp;
    // Deckung einer Fläche bis zur Kante im Abstand `d` (Punkte), mit einem Pixel weicher Kante
    let fill = |d: f32| (0.5 - d * ppp).clamp(0.0, 1.0);
    let mut pixels = vec![Color32::TRANSPARENT; rows * cols];
    for row in 0..rows {
        let y = (row as f32 + 0.5) / ppp - g.ot;
        for col in 0..cols {
            let from_left = (col as f32 + 0.5) / ppp - g.ox;
            let from_right = ((cols - 1 - col) as f32 + 0.5) / ppp - g.ox;
            let dx = if g.wide && col == g.cap { f32::INFINITY } else { from_left.min(from_right) };
            let mut out = [0.0f32; 4];
            for l in layers {
                let (color, a) = match *l {
                    Layer::Shadow { radius, dy, blur, color } => {
                        // Wie der Tessellator: Radius um den halben Übergang größer, linearer Übergang über `blur`
                        let blur = blur.min(g.height.min(2.0 * g.half_w) - 0.1).max(0.0);
                        let d = rrect_dist(dx, y - dy, g, 0.0, 0.0, 0.0, radius + blur / 2.0);
                        let soft = blur.max(1.0 / ppp);
                        (color, (0.5 - d / soft).clamp(0.0, 1.0))
                    }
                    Layer::Fill { inset, radius, top, bottom } => {
                        let d = rrect_dist(dx, y, g, inset[0], inset[1], inset[2], radius);
                        let h = (g.height - inset[1] - inset[2]).max(1.0);
                        // Verlauf deckend wie `hw::gradient_rrect` (über `mix`), einfarbig mit Deckkraft wie `rect_filled`
                        let color = if top == bottom { top } else { mix(top, bottom, ((y - inset[1]) / h).clamp(0.0, 1.0)) };
                        (color, fill(d))
                    }
                    Layer::Stroke { inset, radius, width, color, outside } => {
                        let (r, i) = if outside { (radius + width, inset - width) } else { (radius, inset) };
                        let d = rrect_dist(dx, y, g, i, i, i, r);
                        (color, fill(d) - fill(d + width))
                    }
                    Layer::Chamfer { inset, radius, top, bottom } => {
                        let d = rrect_dist(dx, y, g, inset, inset, inset, radius);
                        let t = ((y - inset) / (g.height - 2.0 * inset).max(1.0)).clamp(0.0, 1.0);
                        let alpha = bottom as f32 + (top as f32 - bottom as f32) * (1.0 - t).powi(3);
                        (Color32::from_white_alpha(alpha as u8), fill(d - 0.5) - fill(d + 0.5))
                    }
                    Layer::HLine { y: ly, from_bottom, x_inset, width, color } => {
                        let mid = if from_bottom { g.height + ly } else { ly };
                        let w = width * ppp;
                        let across = (w.max(1.0) / 2.0 + 0.5 - (y - mid).abs() * ppp).clamp(0.0, 1.0) * w.min(1.0);
                        (color, across * ((dx - x_inset) * ppp + 0.5).clamp(0.0, 1.0))
                    }
                };
                if a <= 0.0 {
                    continue;
                }
                let src = color.to_array().map(|ch| ch as f32 / 255.0 * a);
                let keep = 1.0 - src[3];
                for (o, s) in out.iter_mut().zip(src) {
                    *o = s + *o * keep;
                }
            }
            let [r, gg, b, a] = out.map(|ch| (ch * 255.0).round().clamp(0.0, 255.0) as u8);
            pixels[row * cols + col] = Color32::from_rgba_premultiplied(r, gg, b, a);
        }
    }
    egui::ColorImage::new([cols, rows], pixels)
}

/// So viele Gruppen weit sucht eine Form rückwärts nach ihrer Gruppe (hält den Durchgang linear).
const BATCH_LOOKBACK: usize = 48;

/// Am Ende jedes Bildes: Formen mit derselben Textur zusammenrücken, damit egui sie in einem Zeichenauftrag zeichnet.
/// Jede Taste ist Bild (Atlas) und Beschriftung (Schriftatlas); ohne diesen Durchgang wechselte die Textur bei jeder
/// Taste, und das Zusammenstellen der Aufträge kostete mehr, als die Bilder sparten. Eine Form rückt nur dann vor andere,
/// wenn sie keine davon überdeckt (Begrenzungsrechteck plus Rand für die weiche Kante), das Bild bleibt also gleich.
pub fn batch_by_texture(ctx: &egui::Context) {
    struct Batch {
        key: Option<(Rect, egui::TextureId)>,
        items: Vec<usize>,
        bounds: Rect,
    }
    let layers: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    ctx.graphics_mut(|g| {
        for layer in layers {
            let Some(list) = g.get_mut(layer) else { continue };
            let n = list.next_idx().0;
            if n < 3 {
                continue;
            }
            let shapes: Vec<egui::epaint::ClippedShape> = list.all_entries().cloned().collect();
            let mut batches: Vec<Batch> = Vec::new();
            for (i, cs) in shapes.iter().enumerate() {
                let bounds = cs.shape.visual_bounding_rect().expand(1.0).intersect(cs.clip_rect);
                // Gemischte Gruppen und eigene Zeichenroutinen bleiben, wo sie sind
                let key = match &cs.shape {
                    egui::Shape::Vec(_) | egui::Shape::Callback(_) => None,
                    s => Some((cs.clip_rect, s.texture_id())),
                };
                let mut target = None;
                if key.is_some() {
                    for (k, b) in batches.iter().enumerate().rev().take(BATCH_LOOKBACK) {
                        if b.key == key {
                            target = Some(k);
                            break;
                        }
                        if b.key.is_none() || b.bounds.intersects(bounds) {
                            break;
                        }
                    }
                }
                match target {
                    Some(k) => {
                        let b = &mut batches[k];
                        b.items.push(i);
                        b.bounds = b.bounds.union(bounds);
                    }
                    None => batches.push(Batch { key, items: vec![i], bounds }),
                }
            }
            if batches.len() == n {
                continue;
            }
            for (slot, i) in batches.iter().flat_map(|b| b.items.iter()).enumerate() {
                let cs = &shapes[*i];
                list.set(egui::layers::ShapeIdx(slot), cs.clip_rect, cs.shape.clone());
            }
        }
    });
}
