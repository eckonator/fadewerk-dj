// SPDX-License-Identifier: GPL-3.0-or-later
//! LED-Pegelanzeigen im Stil klassischer Mischpulte.

use eframe::egui::{Color32, Painter, Rect, pos2};
use fw_engine::{BAND_HZ, BANDS};

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

    /// Nichts leuchtet mehr (auch keine Spitzen-LED): Die Anzeige muss nicht neu gezeichnet werden.
    pub fn is_dark(&self) -> bool {
        self.shown.iter().chain(&self.hold).all(|&v| v <= 0.0)
    }

    /// Zeichnet zwei LED-Ketten (links/rechts) nebeneinander in `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        self.paint_with_bands(painter, rect, None, rect.width());
    }

    /// Wie [`Self::paint`], mit Frequenzbändern zwischen L und R, sobald `rect` breiter ist als `base_w`
    /// (die Breite für L/R allein). Passen nicht alle Bänder, werden benachbarte zusammengefasst.
    pub fn paint_with_bands(&self, painter: &Painter, rect: Rect, bands: Option<&Bands>, base_w: f32) {
        painter.rect_filled(rect, 2.0, Color32::from_rgb(3, 6, 11));
        // Fuge zwischen links und rechts bleibt schmal, auch wenn die Anzeige breit wird
        let gap_x = (base_w * 0.14).clamp(1.0, 4.0);
        let col_w = (base_w.min(rect.width()) - gap_x) / 2.0;
        // Platz für Bänder zwischen L und R (je Band mindestens BAND_MIN_W plus Fuge)
        let inner = rect.width() - 2.0 * (col_w + gap_x);
        let band_gap = 2.0;
        let fit = ((inner + band_gap) / (BAND_MIN_W + band_gap)).floor() as usize;
        // Mindestens zwei Säulen (Bässe, Höhen), sonst nur L/R
        let n = if bands.is_some() && fit >= 2 { fit.min(BANDS) } else { 0 };
        if n == 0 {
            let col_w = (rect.width() - gap_x) / 2.0;
            for c in 0..2 {
                let x0 = rect.left() + c as f32 * (col_w + gap_x);
                led_column(painter, x0, col_w, rect, self.shown[c], self.hold[c]);
            }
            return;
        }
        led_column(painter, rect.left(), col_w, rect, self.shown[0], self.hold[0]);
        led_column(painter, rect.right() - col_w, col_w, rect, self.shown[1], self.hold[1]);
        let Some(b) = bands else { return };
        let left = rect.left() + col_w + gap_x;
        let w = (inner - band_gap * (n - 1) as f32) / n as f32;
        for i in 0..n {
            // Die zehn Bänder gleichmäßig auf die Säulen verteilen, jede bekommt mindestens eines
            let group = i * BANDS / n..((i + 1) * BANDS / n).max(i * BANDS / n + 1);
            let shown = b.shown[group.clone()].iter().copied().fold(0.0, f32::max);
            let hold = b.hold[group].iter().copied().fold(0.0, f32::max);
            led_column(painter, left + i as f32 * (w + band_gap), w, rect, shown, hold);
        }
    }
}

/// Mindestbreite einer Band-Säule.
const BAND_MIN_W: f32 = 5.0;

/// Eine LED-Kette in `x0..x0 + w` über die Höhe von `rect`: `shown` leuchtet, `hold` = Spitzen-LED.
fn led_column(painter: &Painter, x0: f32, w: f32, rect: Rect, shown: f32, hold: f32) {
    let gap_y = 1.0;
    let seg_h = (rect.height() - gap_y * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
    let lit = (shown * SEGMENTS as f32).round() as usize;
    let hold = (hold * SEGMENTS as f32).round() as usize;
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
        painter.rect_filled(Rect::from_min_max(pos2(x0, y1 - seg_h), pos2(x0 + w, y1)), 0.0, color);
    }
}

/// Anhebung der Bänder in dB je Oktave über 1 kHz (unter 1 kHz entsprechend Absenkung). Musik hat in den
/// Bässen viel mehr Energie; ohne Ausgleich blieben die Höhen dunkel.
const TILT_DB_PER_OCTAVE: f32 = 3.0;

/// Frequenzbänder des Masters mit derselben Trägheit und Spitzen-LED wie die Pegel.
#[derive(Default, Clone)]
pub struct Bands {
    shown: [f32; BANDS],
    hold: [f32; BANDS],
    hold_since: [f64; BANDS],
}

impl Bands {
    /// Wie [`Meter::is_dark`].
    pub fn is_dark(&self) -> bool {
        self.shown.iter().chain(&self.hold).all(|&v| v <= 0.0)
    }

    pub fn update(&mut self, peaks: [f32; BANDS], dt: f32, now: f64) {
        for (b, &peak) in peaks.iter().enumerate() {
            let tilt = 10f32.powf(TILT_DB_PER_OCTAVE * (BAND_HZ[b] / 1000.0).log2() / 20.0);
            let f = to_fraction(peak * tilt);
            self.shown[b] = if f > self.shown[b] { f } else { (self.shown[b] - dt * FALL_PER_SEC).max(f) };
            if f >= self.hold[b] {
                self.hold[b] = f;
                self.hold_since[b] = now;
            } else if now - self.hold_since[b] > HOLD_SECS {
                self.hold[b] = (self.hold[b] - dt * 0.6).max(self.shown[b]);
            }
        }
    }
}
