// SPDX-License-Identifier: GPL-3.0-or-later
//! Prüfung auf neue Versionen bei GitHub (nur wenn der Nutzer zugestimmt hat).
//!
//! Gesendet wird nur die Anfrage nach dem neuesten Release des Projekts; keine Daten über Titel,
//! Ordner oder Einstellungen. Abgefragt wird über `curl` (auf macOS, Windows 10+ und Linux vorhanden),
//! damit die App keine eigene TLS-Bibliothek mitbringen muss. Fehler (offline, kein Release) bleiben still.

use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;

/// Neuester Release des Projekts.
const LATEST_URL: &str = "https://api.github.com/repos/eckonator/fadewerk-dj/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    pub version: String,
    /// Seite des Releases zum Herunterladen
    pub url: String,
}

/// Startet die Abfrage im Hintergrund; das Ergebnis kommt über den Empfänger (`None` = nichts Neues).
pub fn check_in_background(ctx: egui::Context) -> Receiver<Option<NewVersion>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-update".into())
        .spawn(move || {
            let result = check();
            if let Some(v) = &result {
                log::info!("Neue Version verfügbar: {} ({})", v.version, v.url);
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        })
        .ok();
    rx
}

fn check() -> Option<NewVersion> {
    // FW_UPDATE_URL (nur Debug): andere Quelle, z. B. eine lokale JSON-Datei zum Testen
    let url = std::env::var("FW_UPDATE_URL")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| LATEST_URL.into());
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "8", "-H", "Accept: application/vnd.github+json", "-A", "FadeWerk-DJ-Updatepruefung"])
        .arg(&url);
    #[cfg(windows)]
    {
        // kein Konsolenfenster aufblitzen lassen
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    let page = json.get("html_url")?.as_str()?.to_string();
    let latest = tag.trim_start_matches('v');
    newer(latest, env!("CARGO_PKG_VERSION")).then(|| NewVersion { version: latest.to_string(), url: page })
}

/// Ist `a` neuer als `b`? Vergleicht „1.2.3“ Teil für Teil als Zahlen.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.2", "0.1.2"));
        assert!(!newer("0.1.0", "0.2.0"));
    }
}
