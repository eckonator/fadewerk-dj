// SPDX-License-Identifier: GPL-3.0-or-later
//! LED-Pegelanzeigen im Stil klassischer Mischpulte.

use eframe::egui::{Color32, Painter, Rect, pos2};

use crate::theme;

const SEGMENTS: usize = 28;
/// Abfallgeschwindigkeit der Anzeige (Anzeigebreite pro Sekunde).
const FALL_PER_SEC: f32 = 1.1;
/// So lange bleibt die Spitzen-LED stehen.
const HOLD_SECS: f64 = 0.9;

#[derive(Default, Clone)]
pub struct Meter {
    shown: [f32; 2],
    hold: [f32; 2],
    hold_since: [f64; 2],
}

/// Pegel auf Anzeigeanteil: −36 dB unten bis +3 dB oben.
fn to_fraction(x: f32) -> f32 {
    if x <= 1e-5 { 0.0 } else { ((20.0 * x.log10() + 36.0) / 39.0).clamp(0.0, 1.0) }
}

impl Meter {
    pub fn update(&mut self, peaks: [f32; 2], dt: f32, now: f64) {
        for (c, &peak) in peaks.iter().enumerate() {
            let f = to_fraction(peak);
            self.shown[c] = if f > self.shown[c] { f } else { (self.shown[c] - dt * FALL_PER_SEC).max(f) };
            if f >= self.hold[c] {
                self.hold[c] = f;
                self.hold_since[c] = now;
            } else if now - self.hold_since[c] > HOLD_SECS {
                self.hold[c] = (self.hold[c] - dt * 0.6).max(self.shown[c]);
            }
        }
    }

    /// Zeichnet zwei LED-Ketten (links/rechts) nebeneinander in `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        painter.rect_filled(rect, 2.0, Color32::from_rgb(3, 6, 11));
        // Fuge zwischen links und rechts bleibt schmal, auch wenn die Anzeige breit wird
        let gap_x = (rect.width() * 0.14).clamp(1.0, 4.0);
        let gap_y = 1.0;
        let col_w = (rect.width() - gap_x) / 2.0;
        let seg_h = (rect.height() - gap_y * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
        for c in 0..2 {
            let lit = (self.shown[c] * SEGMENTS as f32).round() as usize;
            let hold = (self.hold[c] * SEGMENTS as f32).round() as usize;
            let x0 = rect.left() + c as f32 * (col_w + gap_x);
            for i in 0..SEGMENTS {
                let base = if i >= SEGMENTS - 3 {
                    theme::cue()
                } else if i >= SEGMENTS - 8 {
                    theme::yellow()
                } else {
                    theme::play()
                };
                let on = i < lit || (hold > 1 && i + 1 == hold);
                let color = if on { base } else { base.gamma_multiply(0.13) };
                let y1 = rect.bottom() - i as f32 * (seg_h + gap_y);
                painter.rect_filled(Rect::from_min_max(pos2(x0, y1 - seg_h), pos2(x0 + col_w, y1)), 0.0, color);
            }
        }
    }
}
