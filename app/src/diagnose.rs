// SPDX-License-Identifier: GPL-3.0-or-later
//! Logdatei und Absturzmeldung.
//!
//! Das Release-Programm hat unter Windows kein Konsolenfenster. Ohne Logdatei verschwinden Fehler
//! beim Start spurlos. Alle Meldungen gehen deshalb zusätzlich in `fadewerk-dj.log` im
//! Einstellungsordner. Ein Absturz (Panic) wird dort mit Ort und Aufrufliste festgehalten und
//! unter Windows zusätzlich als Fenster gemeldet.
//!
//! Abstürze im Grafiktreiber selbst kann Rust nicht abfangen. Dafür liegt während des Grafikstarts
//! eine Markierungsdatei im Einstellungsordner, die nach den ersten Bildern wieder verschwindet.
//! Findet der nächste Start sie noch vor, kam der letzte nicht bis zum Bild.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Ist die Logdatei beim Start größer, wird sie zu `fadewerk-dj.old.log` und neu begonnen.
/// Sonst wird angehängt, damit ein fehlgeschlagener Start beim nächsten nicht überschrieben wird.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn log_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("fadewerk-dj.log"))
}

/// Schreibt jede Meldung auf die Konsole und in die Logdatei (ungepuffert, damit vor einem
/// Absturz nichts verloren geht).
struct Tee(File);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        self.0.flush()
    }
}

fn open_log() -> Option<File> {
    let path = log_path()?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    OpenOptions::new().create(true).append(true).open(&path).ok()
}

/// Markierung für den Grafikstart nur, wenn auch ins Log geschrieben wird (nicht bei Entwicklerläufen).
static MARK_STARTS: AtomicBool = AtomicBool::new(false);
static GRAPHICS_OK: AtomicBool = AtomicBool::new(false);

fn start_marker() -> Option<PathBuf> {
    MARK_STARTS.load(Ordering::Relaxed).then(|| crate::settings::config_dir().map(|d| d.join("grafikstart")))?
}

/// Renderer des letzten Starts, wenn dieser nicht bis zum ersten Bild kam.
pub fn unfinished_graphics_start() -> Option<String> {
    std::fs::read_to_string(start_marker()?).ok().map(|s| s.trim().to_owned())
}

/// Vor dem Öffnen des Fensters: Markierung mit dem Namen des Renderers anlegen.
pub fn graphics_starting(renderer: &str) {
    if let Some(path) = start_marker() {
        let _ = std::fs::write(path, renderer);
    }
}

/// Nach den ersten Bildern: Grafik läuft, Markierung entfernen.
pub fn graphics_running() {
    if !GRAPHICS_OK.swap(true, Ordering::Relaxed) {
        log::info!("Grafik läuft");
        if let Some(path) = start_marker() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Richtet das Log ein (mit Datei nur, wenn `to_file`) und meldet Abstürze.
pub fn init(to_file: bool) {
    // Vor dem Anlegen des Einstellungsordners, sonst werden alte Einstellungen nicht mehr übernommen
    crate::settings::migrate_old_dir();
    let filter = env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn");
    let mut builder = env_logger::Builder::from_env(filter);
    MARK_STARTS.store(to_file, Ordering::Relaxed);
    if let Some(file) = to_file.then(open_log).flatten() {
        builder.target(env_logger::Target::Pipe(Box::new(Tee(file)))).write_style(env_logger::WriteStyle::Never);
    }
    builder.init();
    log::info!(
        "FadeWerk DJ {} startet ({} {}, Prozess {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::process::id()
    );
    install_panic_hook();
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("?");
        let msg = info.payload_as_str().unwrap_or("(ohne Text)");
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        log::error!("Absturz im Thread „{name}“: {msg} ({place})\n{}", std::backtrace::Backtrace::force_capture());
        log::logger().flush();
        // Nur Abstürze der Oberfläche beenden das Programm; dann Bescheid geben
        if cfg!(windows) && name == "main" {
            show_error(&crate::i18n::trf("FadeWerk DJ wurde wegen eines Fehlers beendet:\n\n{}\n({})", &[&msg, &place]));
        }
    }));
}

/// Zeigt eine Fehlermeldung als Fenster, mit dem Ort der Logdatei.
pub fn show_error(text: &str) {
    let log = log_path().map(|p| p.display().to_string()).unwrap_or_default();
    let text = format!("{text}\n\n{}", crate::i18n::trf("Einzelheiten stehen in der Logdatei:\n{}", &[&log]));
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("FadeWerk DJ")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
