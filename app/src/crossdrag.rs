// SPDX-License-Identifier: GPL-3.0-or-later
//! Titel aus dem ausgeklinkten Ordnerfenster ins Hauptfenster ziehen.
//!
//! egui kennt Drag & Drop nur innerhalb eines Fensters. Das Ordnerfenster rechnet den Zeiger darum in
//! Desktop-Koordinaten um ([`CrossDrag::track_source`]), das Hauptfenster zurück in seine eigenen
//! ([`CrossDrag::publish`]). Playlists und Player fragen über [`hover`] und [`take_drop`] nach, ob ein
//! Titel aus dem anderen Fenster über ihnen schwebt oder losgelassen wurde. Möglich, weil das System die
//! Maus beim Ziehen an das Fenster bindet, in dem es begann: Es meldet Bewegung und Loslassen auch
//! außerhalb davon.

use eframe::egui::{self, Context, Pos2, Rect, ViewportInfo};

use crate::i18n::trf;
use crate::playlist::DragItem;
use crate::theme;

/// Zustand im Hauptfenster während eines Bilds: Zeiger in dessen Koordinaten, losgelassen?, Titel.
#[derive(Clone)]
struct Foreign {
    pos: Pos2,
    released: bool,
    item: DragItem,
}

fn data_id() -> egui::Id {
    egui::Id::new("fw-cross-drag")
}

/// Was aus dem Ordnerfenster gezogen wird, mit dem Zeiger in Desktop-Koordinaten.
#[derive(Default)]
pub struct CrossDrag {
    item: Option<DragItem>,
    desktop: Option<Pos2>,
    released: bool,
}

impl CrossDrag {
    /// Im Ordnerfenster am Ende seines Bilds: laufendes Ziehen merken. Wird außerhalb des Fensters
    /// losgelassen, bleibt der Titel für das Hauptfenster liegen (egui verwirft ihn gleich danach).
    pub fn track_source(&mut self, ctx: &Context) {
        let Some(payload) = egui::DragAndDrop::payload::<DragItem>(ctx) else {
            *self = Self::default();
            return;
        };
        let (pos, released, info) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released(), i.viewport().clone()));
        let inside = pos.is_some_and(|p| ctx.content_rect().contains(p));
        self.item = Some((*payload).clone());
        self.desktop = pos.and_then(|p| to_desktop(&info, ctx.zoom_factor(), p));
        // Im Ordnerfenster selbst gibt es nichts abzulegen
        self.released = released && !inside;
        if released && inside {
            *self = Self::default();
        }
    }

    /// Im Hauptfenster vor Playlists und Playern: Zeiger umrechnen und für [`hover`]/[`take_drop`] ablegen,
    /// dazu ein Schildchen mit dem gezogenen Titel am Zeiger.
    pub fn publish(&self, ctx: &Context) {
        ctx.data_mut(|d| d.remove::<Foreign>(data_id()));
        let (Some(item), Some(desktop)) = (self.item.clone(), self.desktop) else { return };
        let info = ctx.input(|i| i.viewport().clone());
        let Some(pos) = from_desktop(&info, ctx.zoom_factor(), desktop) else { return };
        if !ctx.content_rect().contains(pos) {
            return;
        }
        let label = match &item {
            DragItem::Files(paths) if paths.len() == 1 => paths[0].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            DragItem::Files(paths) => trf("{} Titel", &[&paths.len()]),
            DragItem::Row { .. } => String::new(),
        };
        ctx.data_mut(|d| d.insert_temp(data_id(), Foreign { pos, released: self.released, item }));
        if !self.released && !label.is_empty() {
            egui::Area::new(egui::Id::new("fw-cross-drag-label"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(14.0, 14.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).fill(theme::panel()).show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(label).color(theme::ink())).wrap_mode(egui::TextWrapMode::Extend));
                    });
                });
        }
        // Das Hauptfenster bekommt keine eigenen Eingaben, solange gezogen wird
        ctx.request_repaint();
    }

    /// Nach Playlists und Playern: Ein losgelassener Titel ist abgelegt oder verfallen.
    pub fn finish(&mut self, ctx: &Context) {
        ctx.data_mut(|d| d.remove::<Foreign>(data_id()));
        if self.released {
            *self = Self::default();
        }
    }
}

/// Zeiger eines Titels aus dem Ordnerfenster über dem Hauptfenster (dessen Koordinaten).
pub fn hover(ctx: &Context) -> Option<Pos2> {
    ctx.data(|d| d.get_temp::<Foreign>(data_id())).map(|f| f.pos)
}

/// Wurde ein Titel aus dem Ordnerfenster über `rect` losgelassen? Dann gehört er dem Aufrufer.
pub fn take_drop(ctx: &Context, rect: Rect) -> Option<DragItem> {
    let f = ctx.data(|d| d.get_temp::<Foreign>(data_id()))?;
    if !f.released || !rect.contains(f.pos) {
        return None;
    }
    ctx.data_mut(|d| d.remove::<Foreign>(data_id()));
    Some(f.item)
}

/// Faktor von egui-Punkten eines Fensters zu Desktop-Koordinaten. macOS rechnet auf allen Bildschirmen in
/// gemeinsamen logischen Punkten, Windows und X11 in Pixeln; dazwischen liegt nur der Zoom der App bzw.
/// zusätzlich die Skalierung des Bildschirms, auf dem das Fenster liegt.
fn scale(info: &ViewportInfo, zoom: f32) -> Option<f32> {
    if cfg!(target_os = "macos") { Some(zoom) } else { info.native_pixels_per_point.map(|n| n * zoom) }
}

fn to_desktop(info: &ViewportInfo, zoom: f32, p: Pos2) -> Option<Pos2> {
    let inner = info.inner_rect?;
    Some(((inner.min.to_vec2() + p.to_vec2()) * scale(info, zoom)?).to_pos2())
}

fn from_desktop(info: &ViewportInfo, zoom: f32, g: Pos2) -> Option<Pos2> {
    let inner = info.inner_rect?;
    Some((g.to_vec2() / scale(info, zoom)? - inner.min.to_vec2()).to_pos2())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_round_trip_between_windows() {
        // Ordnerfenster rechts auf einem zweiten Bildschirm, Hauptfenster links
        let window = |x: f32, native: f32| ViewportInfo {
            inner_rect: Some(Rect::from_min_size(egui::pos2(x, 50.0), egui::vec2(800.0, 600.0))),
            native_pixels_per_point: Some(native),
            ..Default::default()
        };
        let (browser, main) = (window(1900.0, 2.0), window(100.0, 2.0));
        let g = to_desktop(&browser, 1.0, egui::pos2(-1500.0, 200.0)).unwrap();
        let p = from_desktop(&main, 1.0, g).unwrap();
        assert!((p - egui::pos2(300.0, 200.0)).length() < 0.01, "{p:?}");
        assert_eq!(from_desktop(&ViewportInfo::default(), 1.0, g), None, "ohne Fensterlage (Wayland) kein Ablegen");
    }
}
