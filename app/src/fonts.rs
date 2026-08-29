// SPDX-License-Identifier: GPL-3.0-or-later
//! Schriften: Chakra Petch für die Oberfläche, DSEG7 (7-Segment) für LCD-Anzeigen.
//! Beide unter SIL Open Font License, Lizenztexte in `assets/fonts/`.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};

const CHAKRA_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/ChakraPetch-Medium.ttf");
const CHAKRA_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/ChakraPetch-SemiBold.ttf");
const DSEG7: &[u8] = include_bytes!("../../assets/fonts/DSEG7Classic-Bold.ttf");

const BOLD: &str = "bold";
const LCD: &str = "lcd";

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("chakra".into(), Arc::new(FontData::from_static(CHAKRA_MEDIUM)));
    fonts.font_data.insert("chakra-bold".into(), Arc::new(FontData::from_static(CHAKRA_SEMIBOLD)));
    fonts.font_data.insert("dseg7".into(), Arc::new(FontData::from_static(DSEG7)));

    // Standardschriften bleiben als Rückfall für Symbole (⏮ ⏹ 🎧 …) erhalten
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| std::iter::once(first.to_owned()).chain(defaults.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional, with("chakra"));
    fonts.families.insert(FontFamily::Name(BOLD.into()), with("chakra-bold"));
    // Zeichen, die DSEG7 nicht hat (z. B. „—“), kommen aus Chakra Petch
    fonts.families.insert(FontFamily::Name(LCD.into()), with("dseg7").into_iter().chain(["chakra".to_owned()]).collect());
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("chakra".into());
    }
    ctx.set_fonts(fonts);
}

/// Kräftige Variante für Überschriften, Titel und Tasten.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// 7-Segment-Schrift für Zeiten, BPM und Werte.
pub fn lcd(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(LCD.into()))
}
