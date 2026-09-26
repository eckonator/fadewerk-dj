// SPDX-License-Identifier: GPL-3.0-or-later
//! Selbst gezeichnete Symbole für die Effekt-Pads: Linien und Flächen, gleiche Strichstärke,
//! in jeder Farbe und Größe scharf (keine Emojis aus fremden Schriften).

use eframe::egui::{Color32, Painter, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxIcon {
    Horn,
    Siren,
    Bolt,
    Burst,
    Riser,
    Drop,
    Rewind,
    Brake,
    Bell,
    Note,
    Speaker,
    Mic,
    Star,
    Heart,
    Bomb,
    Wave,
}

impl FxIcon {
    pub const ALL: [Self; 16] = [
        Self::Horn,
        Self::Siren,
        Self::Bolt,
        Self::Burst,
        Self::Riser,
        Self::Drop,
        Self::Rewind,
        Self::Brake,
        Self::Bell,
        Self::Note,
        Self::Speaker,
        Self::Mic,
        Self::Star,
        Self::Heart,
        Self::Bomb,
        Self::Wave,
    ];

    /// Symbole der mitgelieferten Effekte, Reihenfolge wie `fx::BUILTIN_NAMES`.
    pub const DEFAULTS: [Self; 8] = [Self::Horn, Self::Siren, Self::Bolt, Self::Burst, Self::Riser, Self::Drop, Self::Rewind, Self::Brake];

    /// Schlüssel für die Einstellungen.
    pub fn key(self) -> &'static str {
        match self {
            Self::Horn => "horn",
            Self::Siren => "sirene",
            Self::Bolt => "blitz",
            Self::Burst => "explosion",
            Self::Riser => "riser",
            Self::Drop => "drop",
            Self::Rewind => "rewind",
            Self::Brake => "bremse",
            Self::Bell => "glocke",
            Self::Note => "note",
            Self::Speaker => "lautsprecher",
            Self::Mic => "mikrofon",
            Self::Star => "stern",
            Self::Heart => "herz",
            Self::Bomb => "bombe",
            Self::Wave => "welle",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.key() == key)
    }

    /// Name für Tooltips (deutscher Schlüssel für `tr`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Horn => "Horn",
            Self::Siren => "Sirene",
            Self::Bolt => "Blitz",
            Self::Burst => "Explosion",
            Self::Riser => "Anstieg",
            Self::Drop => "Abfall",
            Self::Rewind => "Rückspulen",
            Self::Brake => "Stopp",
            Self::Bell => "Glocke",
            Self::Note => "Note",
            Self::Speaker => "Lautsprecher",
            Self::Mic => "Mikrofon",
            Self::Star => "Stern",
            Self::Heart => "Herz",
            Self::Bomb => "Bombe",
            Self::Wave => "Welle",
        }
    }

    /// Zeichnet das Symbol quadratisch und mittig in `rect`.
    pub fn paint(self, p: &Painter, rect: Rect, color: Color32) {
        let s = rect.width().min(rect.height()) / 2.0;
        let c = rect.center();
        // Einheitsquadrat −1 … 1 auf das Rechteck
        let at = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
        let w = (s * 0.16).max(1.2);
        let stroke = Stroke::new(w, color);
        let line = |pts: &[(f32, f32)]| {
            p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
        };
        let closed = |pts: &[(f32, f32)]| {
            p.add(Shape::closed_line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
        };
        let fill = |pts: &[(f32, f32)]| {
            p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| at(x, y)).collect(), color, Stroke::NONE));
        };
        // Kreisbogen um (cx, cy) mit Radius r von Winkel a0 bis a1 (Bogenmaß, 0 = rechts, im Uhrzeigersinn)
        let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| -> Vec<(f32, f32)> {
            (0..=20).map(|i| { let a = a0 + (a1 - a0) * i as f32 / 20.0; (cx + r * a.cos(), cy + r * a.sin()) }).collect()
        };
        let dot = |x: f32, y: f32, r: f32| {
            p.circle_filled(at(x, y), r * s, color);
        };
        let ring = |x: f32, y: f32, r: f32| {
            p.circle_stroke(at(x, y), r * s, stroke);
        };
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            Self::Horn => {
                // Druckdose links, Trichter nach rechts, zwei Schallbögen
                closed(&[(-0.92, -0.28), (-0.5, -0.28), (-0.5, 0.28), (-0.92, 0.28)]);
                closed(&[(-0.5, -0.12), (0.45, -0.62), (0.45, 0.62), (-0.5, 0.12)]);
                line(&arc(0.3, 0.0, 0.5, -0.6, 0.6));
                line(&arc(0.3, 0.0, 0.72, -0.5, 0.5));
            }
            Self::Siren => {
                // Kuppel auf Sockel, Strahlen
                line(&arc(0.0, 0.35, 0.48, PI, 2.0 * PI));
                closed(&[(-0.7, 0.35), (0.7, 0.35), (0.7, 0.6), (-0.7, 0.6)]);
                for a in [-FRAC_PI_2, -FRAC_PI_2 - 0.75, -FRAC_PI_2 + 0.75, PI, 0.0] {
                    let (r0, r1) = (0.66, 0.92);
                    line(&[(r0 * a.cos(), 0.35 + r0 * a.sin()), (r1 * a.cos(), 0.35 + r1 * a.sin())]);
                }
            }
            Self::Bolt => {
                // Zwei konvexe Hälften ergeben den Blitz
                fill(&[(0.25, -0.95), (-0.5, 0.12), (-0.02, 0.12), (0.08, -0.12)]);
                fill(&[(-0.08, 0.12), (0.02, -0.12), (0.5, -0.12), (-0.25, 0.95)]);
            }
            Self::Burst => {
                let pts: Vec<(f32, f32)> = (0..16)
                    .map(|i| {
                        let a = i as f32 / 16.0 * 2.0 * PI - FRAC_PI_2;
                        let r = if i % 2 == 0 { 0.95 } else { 0.45 };
                        (r * a.cos(), r * a.sin())
                    })
                    .collect();
                closed(&pts);
                dot(0.0, 0.0, 0.16);
            }
            Self::Riser => {
                line(&[(-0.9, 0.6), (-0.35, 0.15), (0.0, 0.35), (0.62, -0.42)]);
                fill(&[(0.85, -0.7), (0.72, -0.18), (0.36, -0.55)]);
            }
            Self::Drop => {
                line(&[(-0.9, -0.6), (-0.35, -0.15), (0.0, -0.35), (0.62, 0.42)]);
                fill(&[(0.85, 0.7), (0.36, 0.55), (0.72, 0.18)]);
            }
            Self::Rewind => {
                fill(&[(0.0, -0.6), (0.0, 0.6), (-0.85, 0.0)]);
                fill(&[(0.85, -0.6), (0.85, 0.6), (0.0, 0.0)]);
            }
            Self::Brake => {
                // Platte mit Stopp-Quadrat
                ring(0.0, 0.0, 0.85);
                fill(&[(-0.32, -0.32), (0.32, -0.32), (0.32, 0.32), (-0.32, 0.32)]);
            }
            Self::Bell => {
                let mut pts = vec![(-0.62, 0.45)];
                pts.extend(arc(0.0, -0.15, 0.45, PI, 2.0 * PI));
                pts.push((0.62, 0.45));
                line(&pts);
                line(&[(-0.8, 0.45), (0.8, 0.45)]);
                dot(0.0, 0.66, 0.13);
                dot(0.0, -0.68, 0.09);
            }
            Self::Note => {
                dot(-0.3, 0.55, 0.27);
                line(&[(-0.06, 0.55), (-0.06, -0.8)]);
                line(&[(-0.06, -0.8), (0.5, -0.5), (0.5, -0.1)]);
            }
            Self::Speaker => {
                fill(&[(-0.9, -0.25), (-0.5, -0.25), (-0.05, -0.65), (-0.05, 0.65), (-0.5, 0.25), (-0.9, 0.25)]);
                line(&arc(-0.05, 0.0, 0.42, -0.75, 0.75));
                line(&arc(-0.05, 0.0, 0.75, -0.7, 0.7));
            }
            Self::Mic => {
                // Kapsel, Halterung, Fuß
                let r = 0.28;
                let mut caps = arc(0.0, -0.55, r, PI, 2.0 * PI);
                caps.extend(arc(0.0, 0.0, r, 0.0, PI));
                closed(&caps);
                line(&arc(0.0, -0.05, 0.52, 0.0, PI));
                line(&[(0.0, 0.47), (0.0, 0.78)]);
                line(&[(-0.38, 0.8), (0.38, 0.8)]);
            }
            Self::Star => {
                let pts: Vec<(f32, f32)> = (0..10)
                    .map(|i| {
                        let a = i as f32 / 10.0 * 2.0 * PI - FRAC_PI_2;
                        let r = if i % 2 == 0 { 0.95 } else { 0.4 };
                        (r * a.cos(), r * a.sin() + 0.05)
                    })
                    .collect();
                closed(&pts);
            }
            Self::Heart => {
                // Spitze unten, zwei Bögen oben, die sich in der Kerbe treffen
                let mut pts = vec![(0.0, 0.85)];
                pts.extend(arc(-0.42, -0.28, 0.45, 0.8 * PI, 2.0 * PI));
                pts.extend(arc(0.42, -0.28, 0.45, PI, 2.2 * PI));
                closed(&pts);
            }
            Self::Bomb => {
                dot(-0.15, 0.25, 0.58);
                line(&[(0.22, -0.2), (0.42, -0.48), (0.62, -0.52)]);
                for (dx, dy) in [(0.0, -1.0), (0.85, -0.5), (0.85, 0.5), (-0.6, -0.8)] {
                    let (x, y) = (0.72, -0.6);
                    line(&[(x + dx * 0.12, y + dy * 0.12), (x + dx * 0.26, y + dy * 0.26)]);
                }
            }
            Self::Wave => {
                let pts: Vec<(f32, f32)> = (0..=40)
                    .map(|i| {
                        let x = -0.9 + 1.8 * i as f32 / 40.0;
                        (x, -0.5 * (x * 1.5 * PI).sin())
                    })
                    .collect();
                line(&pts);
            }
        }
    }
}
