// SPDX-License-Identifier: GPL-3.0-or-later
//! FadeWerk DJ: DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade.

// Unter Windows im Release kein zusätzliches Konsolenfenster öffnen
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod bpm_cache;
mod caches;
mod crossdrag;
mod browser;
mod deck_ui;
mod fade_preview;
mod fadewerk;
mod fonts;
mod folder_cache;
mod fx;
mod fx_icons;
mod history;
mod history_ui;
mod hw;
mod devtools;
mod diagnose;
mod i18n;
mod keynav;
mod meter;
mod mixer_ui;
mod playlist;
mod playlist_ui;
mod quiet;
mod search;
mod selection;
mod self_update;
mod tag_cache;
mod tag_editor;
mod session;
mod settings;
mod settings_ui;
mod theme;
mod update;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use eframe::egui;

fn main() -> eframe::Result {
    // Entwicklerläufe (Demo, Screenshot …) schreiben keine Logdatei, wie sie auch nichts speichern
    let isolated = devtools::DevTools::from_env().isolated();
    diagnose::init(!isolated);
    self_update::clean_up();
    let mut settings = settings::Settings::load();
    // Optional: ein Musikordner als Argument wird beim Start direkt geöffnet
    let start_dir = std::env::args_os().nth(1).map(std::path::PathBuf::from).filter(|p| p.is_dir());

    // Grafik: wgpu (Standard) oder OpenGL. FW_RENDERER=glow|wgpu überschreibt die Einstellung.
    // Kam der letzte Start mit wgpu nicht bis zum ersten Bild, ist vermutlich der Grafiktreiber
    // abgestürzt (das kann Rust nicht abfangen): ab jetzt OpenGL
    if !settings.opengl
        && std::env::var_os("FW_RENDERER").is_none()
        && diagnose::unfinished_graphics_start().is_some_and(|r| r == "wgpu")
    {
        log::warn!("Der letzte Start mit wgpu kam nicht bis zum ersten Bild, nehme ab jetzt OpenGL");
        settings.opengl = true;
        settings.save();
    }
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
    diagnose::graphics_starting(&renderer.to_string());
    // Entwicklerhilfe (nur Debug): FW_SIZE=2400x1350 setzt die Fenstergröße
    let size = std::env::var("FW_SIZE")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?])))
        .unwrap_or([1440.0, 920.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FadeWerk DJ")
            // Wayland sucht Icon und Namen über die .desktop-Datei mit diesem Namen (assets/fadewerk-dj.desktop)
            .with_app_id("fadewerk-dj")
            .with_inner_size(size)
            .with_min_inner_size([1100.0, 620.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).unwrap_or_default()),
        renderer,
        wgpu_options: wgpu_options(),
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

/// wgpu-Einstellungen. Unter Windows nur DirectX 12: Vulkan-Treiber (etwa Intel UHD 620) stürzten
/// beim Start sporadisch ab, ohne dass sich das abfangen ließ. `WGPU_BACKEND` überschreibt das.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew, wgpu};
    let mut options = WgpuConfiguration::default().with_surface_config(eframe::SurfaceConfig::LOW_LATENCY);
    if cfg!(windows) {
        let mut setup = WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
        options.wgpu_setup = WgpuSetup::CreateNew(setup);
    }
    options.on_surface_status = std::sync::Arc::new(surface_status);
    options
}

/// So viele Bilder ohne Zeichenfläche („verdeckt“ oder Zeitüberschreitung), dann wird sie neu eingerichtet.
/// Bei laufender Musik sind das Sekundenbruchteile, im Ruhezustand (4 Bilder/s) etwa 5 s.
const SURFACE_STUCK_FRAMES: u32 = 20;

/// Was tun, wenn die Grafik kein Bild zum Zeichnen liefert? Wie eframe, aber ein dauerhaft „verdecktes“ Fenster
/// wird nach einer Weile neu eingerichtet. Unter Windows meldet DirectX nach ausgeschaltetem oder gesperrtem
/// Bildschirm mitunter weiter „verdeckt“, obwohl das Fenster wieder zu sehen ist: Die App lief (Musik spielte),
/// aber das Bild stand, bis das Fenster verschoben wurde (Nutzerbericht 2026-10-15).
fn surface_status(status: &eframe::egui_wgpu::wgpu::CurrentSurfaceTexture) -> eframe::egui_wgpu::SurfaceErrorAction {
    use eframe::egui_wgpu::{SurfaceErrorAction, wgpu::CurrentSurfaceTexture};
    use std::sync::atomic::{AtomicU32, Ordering};
    static STUCK: AtomicU32 = AtomicU32::new(0);
    match status {
        CurrentSurfaceTexture::Outdated => SurfaceErrorAction::Reconfigure,
        CurrentSurfaceTexture::Lost => {
            log::debug!("Zeichenfläche verloren, wird neu erzeugt");
            SurfaceErrorAction::RecreateSurface
        }
        _ => {
            let n = STUCK.fetch_add(1, Ordering::Relaxed) + 1;
            if n >= SURFACE_STUCK_FRAMES {
                STUCK.store(0, Ordering::Relaxed);
                log::info!("Zeichenfläche seit {n} Bildern nicht verfügbar ({status:?}), wird neu eingerichtet");
                // Richtet die Fläche neu ein und fordert gleich das nächste Bild an
                SurfaceErrorAction::Reconfigure
            } else {
                SurfaceErrorAction::SkipFrame
            }
        }
    }
}
