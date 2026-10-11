// SPDX-License-Identifier: GPL-3.0-or-later
//! FadeWerk DJ: DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade.

// Unter Windows im Release kein zusätzliches Konsolenfenster öffnen
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod browser;
mod deck_ui;
mod fonts;
mod hw;
mod devtools;
mod i18n;
mod meter;
mod mixer_ui;
mod playlist;
mod playlist_ui;
mod selection;
mod session;
mod settings;
mod settings_ui;
mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Entwicklerhilfe (nur Debug): FW_SIZE=2400x1350 setzt die Fenstergröße
    let size = std::env::var("FW_SIZE")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?])))
        .unwrap_or([1440.0, 920.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FadeWerk DJ")
            .with_inner_size(size)
            .with_min_inner_size([1100.0, 720.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    // Optional: ein Musikordner als Argument wird beim Start direkt geöffnet
    let start_dir = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| p.is_dir());
    eframe::run_native("FadeWerk DJ", options, Box::new(|cc| Ok(Box::new(app::FadeWerkDjApp::new(cc, start_dir)))))
}
