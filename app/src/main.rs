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
mod diagnose;
mod i18n;
mod meter;
mod mixer_ui;
mod playlist;
mod playlist_ui;
mod selection;
mod tag_editor;
mod session;
mod settings;
mod settings_ui;
mod theme;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use eframe::egui;

fn main() -> eframe::Result {
    // Entwicklerläufe (Demo, Screenshot …) schreiben keine Logdatei, wie sie auch nichts speichern
    let isolated = devtools::DevTools::from_env().isolated();
    diagnose::init(!isolated);
    let mut settings = settings::Settings::load();
    // Optional: ein Musikordner als Argument wird beim Start direkt geöffnet
    let start_dir = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| p.is_dir());

    // Grafik: wgpu (Standard) oder OpenGL. FW_RENDERER=glow|wgpu überschreibt die Einstellung.
    let renderer = match std::env::var("FW_RENDERER").as_deref() {
        Ok("glow" | "opengl") => eframe::Renderer::Glow,
        Ok("wgpu") => eframe::Renderer::Wgpu,
        _ if settings.opengl => eframe::Renderer::Glow,
        _ => eframe::Renderer::Wgpu,
    };
    let app_created = Rc::new(Cell::new(false));
    let mut result = run(renderer, settings.clone(), start_dir.clone(), &app_created);
    // Scheitert wgpu schon beim Einrichten (manche Treiber, virtuelle Maschinen, Remote Desktop),
    // mit OpenGL noch einmal versuchen und das für die nächsten Starts merken
    if let Err(e) = &result
        && renderer == eframe::Renderer::Wgpu
        && !app_created.get()
    {
        log::error!("Grafik über wgpu nicht verfügbar ({e}), versuche OpenGL");
        settings.opengl = true;
        if !isolated {
            settings.save();
        }
        result = run(eframe::Renderer::Glow, settings, start_dir, &app_created);
    }
    if let Err(e) = &result {
        log::error!("Programm beendet mit Fehler: {e}");
        if cfg!(windows) && !app_created.get() {
            diagnose::show_error(&i18n::trf("Das Fenster ließ sich nicht öffnen:\n{}", &[e]));
        }
    }
    result
}

fn run(renderer: eframe::Renderer, settings: settings::Settings, start_dir: Option<PathBuf>, app_created: &Rc<Cell<bool>>) -> eframe::Result {
    log::info!("Grafik: {renderer}");
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
        renderer,
        ..Default::default()
    };
    let created = Rc::clone(app_created);
    eframe::run_native(
        "FadeWerk DJ",
        options,
        Box::new(move |cc| {
            created.set(true);
            Ok(Box::new(app::FadeWerkDjApp::new(cc, settings, start_dir)))
        }),
    )
}
