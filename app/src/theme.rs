// SPDX-License-Identifier: GPL-3.0-or-later
//! Skins: austauschbare Farbsätze für die ganze Oberfläche.
//!
//! Gezeichnet wird immer über die Funktionen `ground()`, `ink()` usw., die den aktiven Skin lesen.
//! Anzeigen wie Wellenform, Zeit und Pegel liegen in „Wells“, die in jedem Skin dunkel bleiben,
//! damit sie wie Displays an echter Hardware wirken und lesbar bleiben.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

use crate::i18n::{Lang, lang};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SkinId {
    #[default]
    Nacht,
    Classic,
    Tag,
    Neon,
}

pub struct Skin {
    pub id: SkinId,
    pub name_de: &'static str,
    pub name_en: &'static str,
    pub about_de: &'static str,
    pub about_en: &'static str,
    pub dark: bool,
    pub ground: Color32,
    pub panel: Color32,
    pub panel2: Color32,
    pub well: Color32,
    pub well_ink: Color32,
    pub line: Color32,
    pub ink: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub deck_a: Color32,
    pub deck_b: Color32,
    pub cue: Color32,
    pub play: Color32,
    pub yellow: Color32,
}

impl Skin {
    pub fn name(&self) -> &'static str {
        if lang() == Lang::En { self.name_en } else { self.name_de }
    }
    pub fn about(&self) -> &'static str {
        if lang() == Lang::En { self.about_en } else { self.about_de }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub static SKINS: [Skin; 4] = [
    Skin {
        id: SkinId::Nacht,
        name_de: "Nacht",
        name_en: "Night",
        about_de: "Dunkles Blaugrau, Bernstein und Eisblau. Der Standard.",
        about_en: "Dark blue-grey with amber and ice blue. The default.",
        dark: true,
        ground: rgb(0x0a0e16),
        panel: rgb(0x111826),
        panel2: rgb(0x172033),
        well: rgb(0x080c13),
        well_ink: rgb(0xe6ebf5),
        line: rgb(0x243049),
        ink: rgb(0xe6ebf5),
        dim: rgb(0xa9b4c8),
        faint: rgb(0x7d89a3),
        deck_a: rgb(0xf2a33a),
        deck_b: rgb(0x58c4ee),
        cue: rgb(0xe5484d),
        play: rgb(0x3ecf74),
        yellow: rgb(0xf2c94c),
    },
    Skin {
        id: SkinId::Classic,
        name_de: "Classic",
        name_en: "Classic",
        about_de: "Gebürstetes Graphit mit LCD-Anzeigen, wie DJ-Software um 2005.",
        about_en: "Brushed graphite with LCD displays, like DJ software around 2005.",
        dark: true,
        ground: rgb(0x2a2d32),
        panel: rgb(0x3a3e45),
        panel2: rgb(0x4b5059),
        well: rgb(0x0a2133),
        well_ink: rgb(0x8be4ff),
        line: rgb(0x5c626c),
        ink: rgb(0xf1f3f6),
        dim: rgb(0xd0d4db),
        faint: rgb(0xa2a8b1),
        deck_a: rgb(0xff9a1f),
        deck_b: rgb(0x47c6ff),
        cue: rgb(0xe8352f),
        play: rgb(0x2fcc4f),
        yellow: rgb(0xf5d23c),
    },
    Skin {
        id: SkinId::Tag,
        name_de: "Tag",
        name_en: "Day",
        about_de: "Hell für Tageslicht und Bühnen im Freien. Anzeigen bleiben dunkel.",
        about_en: "Light for daylight and outdoor stages. Displays stay dark.",
        dark: false,
        ground: rgb(0xe4e8ef),
        panel: rgb(0xf7f8fb),
        panel2: rgb(0xe2e7ef),
        well: rgb(0x172030),
        well_ink: rgb(0xeef2f8),
        line: rgb(0xc6cdd9),
        ink: rgb(0x172033),
        // Kräftiger als in dunklen Skins: auf hellem Grund wirken Grautöne sonst verwaschen
        dim: rgb(0x3a4459),
        faint: rgb(0x5f6a80),
        deck_a: rgb(0xd97b10),
        deck_b: rgb(0x1689c2),
        cue: rgb(0xd33a3f),
        play: rgb(0x1f9d52),
        yellow: rgb(0xd9a400),
    },
    Skin {
        id: SkinId::Neon,
        name_de: "Neon",
        name_en: "Neon",
        about_de: "Club-Look mit Pink und Türkis auf Tiefviolett.",
        about_en: "Club look with pink and teal on deep violet.",
        dark: true,
        ground: rgb(0x0c0a16),
        panel: rgb(0x151229),
        panel2: rgb(0x211c3d),
        well: rgb(0x07060f),
        well_ink: rgb(0xf1ecff),
        line: rgb(0x2f2852),
        ink: rgb(0xf1ecff),
        dim: rgb(0xbbb2e2),
        faint: rgb(0x877cb8),
        deck_a: rgb(0xff4fa3),
        deck_b: rgb(0x2ee6c8),
        cue: rgb(0xff4060),
        play: rgb(0x47e08a),
        yellow: rgb(0xffd84a),
    },
];

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn skin() -> &'static Skin {
    &SKINS[(CURRENT.load(Ordering::Relaxed) as usize).min(SKINS.len() - 1)]
}

pub fn set_skin(id: SkinId) {
    let i = SKINS.iter().position(|s| s.id == id).unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

static BRUSHED: AtomicBool = AtomicBool::new(false);

/// Flächen als gebürsteter Stahl in den Farben des Skins (siehe `hw::brushed`).
pub fn set_brushed(on: bool) {
    BRUSHED.store(on, Ordering::Relaxed);
}

pub fn brushed() -> bool {
    BRUSHED.load(Ordering::Relaxed)
}

/// Hintergrund ausgewählter Listenzeilen: deutlich kräftiger als die Zebra-Streifen.
pub fn row_selected() -> Color32 {
    let s = skin();
    if brushed() { s.ink.gamma_multiply(if s.dark { 0.24 } else { 0.2 }) } else { mix(s.panel2, s.ink, if s.dark { 0.32 } else { 0.22 }) }
}

/// Hintergrund einer Listenzeile unter der Maus; auf Stahl durchscheinend.
pub fn row_hover() -> Color32 {
    if brushed() { skin().ink.gamma_multiply(0.08) } else { skin().panel2 }
}

pub fn ground() -> Color32 { skin().ground }
pub fn panel() -> Color32 { skin().panel }
pub fn panel2() -> Color32 { skin().panel2 }
pub fn well() -> Color32 { skin().well }
pub fn well_ink() -> Color32 { skin().well_ink }
pub fn line() -> Color32 { skin().line }
pub fn ink() -> Color32 { skin().ink }
/// Nebentext. Auf Stahl näher an `ink`, weil die Schleifspuren sonst die Schrift schlucken.
pub fn dim() -> Color32 {
    if brushed() { mix(skin().dim, skin().ink, 0.35) } else { skin().dim }
}
/// Schwacher Text (Nummern, Spaltenköpfe). Auf Stahl ebenfalls kräftiger.
pub fn faint() -> Color32 {
    if brushed() { mix(skin().faint, skin().ink, 0.35) } else { skin().faint }
}
pub fn cue() -> Color32 { skin().cue }
pub fn play() -> Color32 { skin().play }
pub fn yellow() -> Color32 { skin().yellow }

pub fn deck_color(deck: usize) -> Color32 {
    if deck == 0 { skin().deck_a } else { skin().deck_b }
}

pub fn deck_letter(deck: usize) -> &'static str {
    if deck == 0 { "A" } else { "B" }
}

/// Mischt zwei Farben, `t` = Anteil von `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Überträgt den aktiven Skin auf die egui-Grundwidgets.
pub fn apply(ctx: &egui::Context) {
    let s = skin();
    let mut v = if s.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = s.ground;
    v.window_fill = s.panel;
    v.extreme_bg_color = if s.dark { s.well } else { rgb(0xffffff) };
    v.faint_bg_color = s.panel2;
    v.override_text_color = Some(s.ink);
    v.selection.bg_fill = mix(s.panel2, s.ink, 0.25);
    v.selection.stroke = Stroke::new(1.0, s.ink);
    v.window_stroke = Stroke::new(1.0, s.line);
    let r = CornerRadius::same(4);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_fill = s.panel;
    v.widgets.noninteractive.weak_bg_fill = s.panel;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, s.line);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, s.dim);
    v.widgets.inactive.bg_fill = s.panel2;
    v.widgets.inactive.weak_bg_fill = s.panel2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, s.line);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, s.ink);
    v.widgets.hovered.bg_fill = mix(s.panel2, s.ink, 0.08);
    v.widgets.hovered.weak_bg_fill = mix(s.panel2, s.ink, 0.08);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, s.faint);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, s.ink);
    v.widgets.active.bg_fill = mix(s.panel2, s.ink, 0.16);
    v.widgets.active.weak_bg_fill = mix(s.panel2, s.ink, 0.16);
    v.widgets.active.fg_stroke = Stroke::new(1.0, s.ink);
    if brushed() {
        // Auf Stahl keine deckenden Flächen: Zebra-Zeilen und Auswahl lassen die Schleifspuren durchscheinen
        v.faint_bg_color = Color32::from_black_alpha(if s.dark { 60 } else { 26 });
        v.selection.bg_fill = s.ink.gamma_multiply(if s.dark { 0.16 } else { 0.12 });
    }
    ctx.set_visuals(v);

    ctx.global_style_mut(|st| {
        st.spacing.item_spacing = egui::vec2(6.0, 6.0);
        st.spacing.button_padding = egui::vec2(8.0, 4.0);
        st.spacing.slider_width = 140.0;
    });
}
