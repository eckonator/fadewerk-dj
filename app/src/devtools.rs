// SPDX-License-Identifier: GPL-3.0-or-later
//! Hilfen für die Entwicklung, nur in Debug-Builds aktiv.
//!
//! - `FW_SCREENSHOT=/pfad/bild.bmp` speichert kurz nach dem Start ein Bild des Fensters und beendet die App.
//! - `FW_DEMO=1` verteilt die Titel des Startordners auf beide Playlists und startet Player A.
//! - `FW_SKIN=nacht|classic|tag|neon|silber|anthrazit` und `FW_LANG=de|en` überschreiben Skin und Sprache (ohne zu speichern).
//! - `FW_PREVIEW=1` hört in der Demo den zweiten Titel vor.
//! - `FW_TAGS=1` öffnet in der Demo das Fenster „Titelinfo & Tags“ für den ersten Titel; mit `FW_ONLINE=1`
//!   startet dort gleich „Online suchen“ (fragt AcoustID/MusicBrainz, ohne Rückfrage).
//! - `FW_BATCH=1` öffnet in der Demo „Tags online ergänzen“ für alle Titel des Startordners (mit `FW_ONLINE=1`
//!   startet die Erkennung sofort; geschrieben wird nur per Klick).
//! - `FW_SELECT=1` wählt in der Demo im Ordnerbrowser den 2. bis 4. Titel aus.
//! - `FW_HISTORY=1` legt in der Demo zwei Beispieltage der Historie an (einer mit fehlender Datei) und öffnet den neueren.
//! - `FW_BPM=1` ermittelt in der Demo die fehlenden BPM des Startordners (Ergebnis in `bpm.json`, mit `FW_PERSIST=1`).
//! - `FW_SEARCH="genre=Rock,year=1980-1989"` öffnet die erweiterte Suche mit diesen Feldern.
//! - `FW_TAB=kopfhoerer|fade|motor|zu` öffnet diesen Reiter unter dem Crossfader (`zu` = alle zugeklappt).
//! - `FW_PLATTERS=0` blendet die Plattenteller aus.
//! - `FW_BROWSER=rechts` stellt den Ordnerbrowser rechts neben die Player.
//! - `FW_COLUMNS=artist,album,genre,year,…` zeigt diese Spalten in der Titelliste des Browsers.
//! - `FW_AUTOSYNC=1` schaltet Auto-Sync ein (ohne zu speichern).
//! - `FW_STEEL=1` schaltet gebürsteten Stahl ein (ohne zu speichern).
//! - `FW_LITE=1` schaltet die sparsame Darstellung ein (ohne zu speichern).
//! - `FW_SETTINGS=allgemein|audio|effekte|sprache|aussehen|ueber` öffnet das Einstellungsfenster auf dieser Seite.
//! - `FW_POINTER=x,y` bewegt die Maus (in Punkten) nach Bild 100 dorthin, etwa um Tooltips zu sehen;
//!   `FW_POINTER=x,y,2` klickt dort rechts (Kontextmenü), `FW_POINTER=x,y,1` links;
//!   `FW_POINTER=x1,y1,x2,y2` zieht mit gedrückter Taste von x1,y1 nach x2,y2 und bleibt dort stehen.
//! - `FW_KEYS=ArrowDown,Enter` drückt ab Bild 120 nacheinander diese Tasten (Namen wie `egui::Key::from_name`),
//!   `text:abc` tippt Text.
//! - `FW_MINIMIZE=1` minimiert das Fenster kurz nach dem Start (Abspiel-Logik ohne sichtbares Fenster prüfen).
//! - `FW_SIZE=2400x1350` setzt die Fenstergröße (in `main.rs`).
//! - `FW_UPDATE_ASK=1` zeigt die Frage nach der Update-Prüfung; `FW_UPDATE_URL=file:///…json` prüft gegen eine
//!   eigene Release-Antwort (in `app.rs` bzw. `update.rs`).
//! - Bei diesen Schaltern wird nichts gespeichert; `FW_PERSIST=1` erzwingt Speichern (Test mit anderem `HOME`).

use std::io::Write;
use std::path::{Path, PathBuf};

use eframe::egui::{self, ColorImage, Event, ViewportCommand};

use crate::i18n::Lang;
use crate::theme::SkinId;

/// Bilder, bis das Bildschirmfoto ausgelöst wird (Zeit zum Dekodieren der Demo-Titel).
const SCREENSHOT_AFTER_FRAMES: u64 = 240;
/// Mindestwartezeit, damit kurze Hinweise (3 s) wieder verschwunden sind.
const SCREENSHOT_AFTER_SECS: f32 = 4.5;

/// Wartezeit bis zum Bildschirmfoto, mit `FW_SHOT_SECS` verlängerbar (etwa um eine Überblendung abzuwarten).
fn shot_secs() -> f32 {
    std::env::var("FW_SHOT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(SCREENSHOT_AFTER_SECS)
}

#[derive(Default)]
pub struct DevTools {
    screenshot: Option<PathBuf>,
    pub demo: bool,
    pub skin: Option<SkinId>,
    pub lang: Option<Lang>,
    pub steel: bool,
    pub lite: bool,
    pub settings_tab: Option<String>,
    frame: u64,
    started: Option<std::time::Instant>,
    requested: bool,
    /// Vorgetäuschte Maus aus `FW_POINTER`.
    pointer: Vec<f32>,
    /// Vorgetäuschte Tasten aus `FW_KEYS`: eine Taste oder ein getippter Text.
    keys: Vec<Result<egui::Key, String>>,
    /// `FW_MINIMIZE`: Fenster nach dem Start minimieren (ist es schon geschehen?).
    minimize: Option<bool>,
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
                "silber" => Some(SkinId::Silber),
                "anthrazit" => Some(SkinId::Anthrazit),
                _ => None,
            }),
            lang: std::env::var("FW_LANG").ok().map(|l| if l == "en" { Lang::En } else { Lang::De }),
            steel: std::env::var_os("FW_STEEL").is_some(),
            lite: std::env::var_os("FW_LITE").is_some(),
            settings_tab: std::env::var("FW_SETTINGS").ok(),
            pointer: std::env::var("FW_POINTER")
                .ok()
                .map(|s| s.split(',').filter_map(|v| v.trim().parse().ok()).collect())
                .unwrap_or_default(),
            keys: std::env::var("FW_KEYS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .filter_map(|k| match k.trim().strip_prefix("text:") {
                            Some(t) => Some(Err(t.to_owned())),
                            None => egui::Key::from_name(k.trim()).map(Ok),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            minimize: std::env::var_os("FW_MINIMIZE").map(|_| false),
            frame: 0,
            started: None,
            requested: false,
        }
    }

    /// Bei Entwicklerläufen (Demo, Screenshot, Skin/Sprache überschrieben) nichts speichern und
    /// keine Playlists wiederherstellen, damit die echten Daten des Nutzers unberührt bleiben.
    pub fn isolated(&self) -> bool {
        // FW_PERSIST=1 erzwingt Speichern trotzdem, etwa zum Testen mit einem anderen HOME
        std::env::var_os("FW_PERSIST").is_none()
            && (self.screenshot.is_some() || self.demo || self.skin.is_some() || self.lang.is_some() || self.steel || self.lite || self.settings_tab.is_some())
    }

    /// Speist die Mausbewegung aus `FW_POINTER` ein (aufgerufen vor jedem Bild).
    pub fn raw_input(&self, raw: &mut egui::RawInput) {
        // FW_KEYS: ab Bild 120 alle 5 Bilder eine Taste (nach einem Klick aus FW_POINTER)
        if let Some(n) = self.frame.checked_sub(120).filter(|n| n % 5 == 0).map(|n| (n / 5) as usize)
            && let Some(k) = self.keys.get(n)
        {
            match k {
                Ok(key) => {
                    for pressed in [true, false] {
                        raw.events.push(Event::Key { key: *key, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
                    }
                }
                Err(text) => raw.events.push(Event::Text(text.clone())),
            }
        }
        let p = &self.pointer;
        if p.len() < 2 {
            return;
        }
        let (start, f) = (100u64, self.frame);
        let pos = |x: f32, y: f32| egui::pos2(x, y);
        let button = |pressed| Event::PointerButton {
            pos: if pressed { pos(p[0], p[1]) } else { pos(p[2], p[3]) },
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        if p.len() == 3 {
            // x,y,2: Rechtsklick dort (Kontextmenü); x,y,1: Linksklick
            if f == start {
                raw.events.push(Event::PointerMoved(pos(p[0], p[1])));
            }
            if f == start + 2 || f == start + 3 {
                raw.events.push(Event::PointerButton {
                    pos: pos(p[0], p[1]),
                    button: if p[2] == 1.0 { egui::PointerButton::Primary } else { egui::PointerButton::Secondary },
                    pressed: f == start + 2,
                    modifiers: Default::default(),
                });
            }
            return;
        }
        if p.len() < 4 {
            // Nur eine Weile: egui zeigt Tooltips erst, wenn die Maus ruht
            if (start..start + 10).contains(&f) {
                raw.events.push(Event::PointerMoved(pos(p[0], p[1])));
            }
            return;
        }
        // Drücken, in 30 Bildern hinüberziehen, loslassen, stehen bleiben
        match f {
            f if f == start => raw.events.extend([Event::PointerMoved(pos(p[0], p[1])), button(true)]),
            f if f > start && f <= start + 30 => {
                let k = (f - start) as f32 / 30.0;
                raw.events.push(Event::PointerMoved(pos(p[0] + (p[2] - p[0]) * k, p[1] + (p[3] - p[1]) * k)));
            }
            f if f == start + 31 => raw.events.push(button(false)),
            _ => {}
        }
    }

    pub fn on_frame(&mut self, ctx: &egui::Context) {
        if self.minimize == Some(false) && ctx.cumulative_frame_nr() >= 60 {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            self.minimize = Some(true);
        }
        let Some(path) = self.screenshot.clone() else { return };
        self.frame += 1;
        // Auch nach Zeit: Hinweise (3 s) sollen verschwunden sein, egal wie schnell gezeichnet wird
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        if !self.requested && self.frame >= SCREENSHOT_AFTER_FRAMES && started.elapsed().as_secs_f32() >= shot_secs() {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(Default::default()));
            self.requested = true;
        }
        // Weiterzeichnen, auch wenn sich nichts bewegt, sonst kommt der Zeitpunkt nie
        ctx.request_repaint();
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
