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
    /// Die Datei für dieses System, wenn die App sie selbst installieren kann (`self_update`).
    pub asset: Option<String>,
}

/// Startet die Abfrage im Hintergrund; das Ergebnis kommt über den Empfänger (`Ok(None)` = nichts Neues,
/// `Err` = GitHub nicht erreichbar).
pub fn check_in_background(ctx: egui::Context) -> Receiver<Result<Option<NewVersion>, ()>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-update".into())
        .spawn(move || {
            let result = check();
            if let Ok(Some(v)) = &result {
                log::info!("Neue Version verfügbar: {} ({})", v.version, v.url);
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        })
        .ok();
    rx
}

fn check() -> Result<Option<NewVersion>, ()> {
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
    let out = cmd.output().ok().filter(|o| o.status.success()).ok_or(())?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| ())?;
    Ok(newer_release(&json, env!("CARGO_PKG_VERSION"), crate::self_update::asset_suffix()))
}

/// Vergleicht einen Release (Objekt der GitHub-API) mit dieser Version und sucht die Datei mit der Endung `suffix`.
fn newer_release(release: &serde_json::Value, version: &str, suffix: Option<&str>) -> Option<NewVersion> {
    let tag = release.get("tag_name")?.as_str()?;
    let page = release.get("html_url")?.as_str()?.to_string();
    let latest = tag.trim_start_matches('v');
    if !newer(latest, version) {
        return None;
    }
    let asset = suffix.and_then(|suffix| {
        release.get("assets")?.as_array()?.iter().filter_map(|a| a.get("browser_download_url")?.as_str()).find(|u| u.ends_with(suffix)).map(str::to_string)
    });
    Some(NewVersion { version: latest.to_string(), url: page, asset })
}

/// Ist `a` neuer als `b`? Vergleicht „1.2.3“ Teil für Teil als Zahlen.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("0.10.0", "0.9.0"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.2", "0.1.2"));
        assert!(!newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn picks_the_file_for_this_system() {
        let release = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://example/releases/v0.2.0",
            "assets": [
                {"browser_download_url": "https://example/FadeWerk-DJ-0.2.0-macOS.dmg"},
                {"browser_download_url": "https://example/FadeWerk-DJ-0.2.0-macOS.zip"},
                {"browser_download_url": "https://example/FadeWerk-DJ-0.2.0-Windows.zip"},
            ],
        });
        let found = newer_release(&release, "0.1.0", Some("-macOS.zip")).unwrap();
        assert_eq!(found.asset.as_deref(), Some("https://example/FadeWerk-DJ-0.2.0-macOS.zip"));
        assert_eq!(found.version, "0.2.0");
        let found = newer_release(&release, "0.1.0", Some("-Linux-x86_64.tar.gz")).unwrap();
        assert_eq!(found.asset, None);
        assert!(newer_release(&release, "0.2.0", Some("-macOS.zip")).is_none());
    }
}
