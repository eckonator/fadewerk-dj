// SPDX-License-Identifier: GPL-3.0-or-later
//! Hilfen für die Entwicklung, nur in Debug-Builds aktiv.
//!
//! - `FW_SCREENSHOT=/pfad/bild.bmp` speichert kurz nach dem Start ein Bild des Fensters und beendet die App.
//! - `FW_DEMO=1` verteilt die Titel des Startordners auf beide Playlists und startet Player A.
//! - `FW_SKIN=nacht|classic|tag|neon` und `FW_LANG=de|en` überschreiben Skin und Sprache (ohne zu speichern).
//! - `FW_PREVIEW=1` hört in der Demo den zweiten Titel vor.
//! - `FW_STEEL=1` schaltet gebürsteten Stahl ein (ohne zu speichern).
//! - `FW_SETTINGS=audio|sprache|aussehen|ueber` öffnet das Einstellungsfenster auf dieser Seite.
//! - `FW_SIZE=2400x1350` setzt die Fenstergröße (in `main.rs`).
//! - Bei diesen Schaltern wird nichts gespeichert; `FW_PERSIST=1` erzwingt Speichern (Test mit anderem `HOME`).

use std::io::Write;
use std::path::{Path, PathBuf};

use eframe::egui::{self, ColorImage, Event, ViewportCommand};

use crate::i18n::Lang;
use crate::theme::SkinId;

/// Bilder, bis das Bildschirmfoto ausgelöst wird (Zeit zum Dekodieren der Demo-Titel).
const SCREENSHOT_AFTER_FRAMES: u64 = 240;

#[derive(Default)]
pub struct DevTools {
    screenshot: Option<PathBuf>,
    pub demo: bool,
    pub skin: Option<SkinId>,
    pub lang: Option<Lang>,
    pub steel: bool,
    pub settings_tab: Option<String>,
    frame: u64,
    requested: bool,
}

impl DevTools {
    pub fn from_env() -> Self {
        if !cfg!(debug_assertions) {
            return Self::default();
        }
        Self {
            screenshot: std::env::var_os("FW_SCREENSHOT").map(PathBuf::from),
            demo: std::env::var_os("FW_DEMO").is_some(),
            skin: std::env::var("FW_SKIN").ok().and_then(|s| match s.as_str() {
                "nacht" => Some(SkinId::Nacht),
                "classic" => Some(SkinId::Classic),
                "tag" => Some(SkinId::Tag),
                "neon" => Some(SkinId::Neon),
                _ => None,
            }),
            lang: std::env::var("FW_LANG").ok().map(|l| if l == "en" { Lang::En } else { Lang::De }),
            steel: std::env::var_os("FW_STEEL").is_some(),
            settings_tab: std::env::var("FW_SETTINGS").ok(),
            frame: 0,
            requested: false,
        }
    }

    /// Bei Entwicklerläufen (Demo, Screenshot, Skin/Sprache überschrieben) nichts speichern und
    /// keine Playlists wiederherstellen, damit die echten Daten des Nutzers unberührt bleiben.
    pub fn isolated(&self) -> bool {
        // FW_PERSIST=1 erzwingt Speichern trotzdem, etwa zum Testen mit einem anderen HOME
        std::env::var_os("FW_PERSIST").is_none()
            && (self.screenshot.is_some() || self.demo || self.skin.is_some() || self.lang.is_some() || self.steel || self.settings_tab.is_some())
    }

    pub fn on_frame(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        self.frame += 1;
        if !self.requested && self.frame >= SCREENSHOT_AFTER_FRAMES {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            match save_bmp(&path, &image) {
                Ok(()) => log::info!("Bildschirmfoto gespeichert: {}", path.display()),
                Err(e) => log::error!("Bildschirmfoto nicht gespeichert: {e}"),
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

/// Schreibt ein unkomprimiertes 24-Bit-BMP.
fn save_bmp(path: &Path, img: &ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"BM")?;
    f.write_all(&(size as u32).to_le_bytes())?;
    f.write_all(&[0; 4])?;
    f.write_all(&54u32.to_le_bytes())?;
    f.write_all(&40u32.to_le_bytes())?;
    f.write_all(&(w as i32).to_le_bytes())?;
    f.write_all(&(h as i32).to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&24u16.to_le_bytes())?;
    f.write_all(&[0; 24])?;
    let pad = vec![0u8; row - w * 3];
    for y in (0..h).rev() {
        for x in 0..w {
            let c = img.pixels[y * w + x];
            f.write_all(&[c.b(), c.g(), c.r()])?;
        }
        f.write_all(&pad)?;
    }
    f.flush()
}
