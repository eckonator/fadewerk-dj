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

/// Warum die Abfrage nicht geklappt hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckError {
    /// GitHub nicht erreichbar oder Antwort unbrauchbar.
    Offline,
    /// Das Programm `curl` fehlt (etwa auf schlanken Linux-Installationen).
    NoCurl,
    /// Die Uhr des Rechners geht falsch (vor dem Bau dieser Version, oder curl hält das Zertifikat für noch nicht
    /// gültig). Dann scheitert jede sichere Verbindung (MacBook mit Datum 2001, Nutzerbericht 2026-11-03).
    ClockWrong,
}

/// Toleranz, bevor die Uhr als falsch gilt (Zeitzonen, Bau kurz vor Mitternacht).
const CLOCK_SLACK_SECS: u64 = 24 * 60 * 60;

/// Steht die Uhr vor dem Bauzeitpunkt dieser Version (mit einem Tag Spielraum)?
pub fn clock_wrong() -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    before_build(now, env!("FW_BUILD_SECS").parse().unwrap_or(0))
}

fn before_build(now: u64, build: u64) -> bool {
    now + CLOCK_SLACK_SECS < build
}

/// Startet die Abfrage im Hintergrund; das Ergebnis kommt über den Empfänger (`Ok(None)` = nichts Neues).
pub fn check_in_background(ctx: egui::Context) -> Receiver<Result<Option<NewVersion>, CheckError>> {
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

fn check() -> Result<Option<NewVersion>, CheckError> {
    // FW_UPDATE_URL (nur Debug): andere Quelle, z. B. eine lokale JSON-Datei zum Testen
    let url = std::env::var("FW_UPDATE_URL")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| LATEST_URL.into());
    if clock_wrong() {
        log::warn!("Update-Prüfung: Die Uhr des Rechners steht vor dem Bau dieser Version");
        return Err(CheckError::ClockWrong);
    }
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "8", "-H", "Accept: application/vnd.github+json", "-A", "FadeWerk-DJ-Updatepruefung"])
        .arg(&url);
    #[cfg(windows)]
    {
        // kein Konsolenfenster aufblitzen lassen
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = match cmd.output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::warn!("Update-Prüfung: curl nicht gefunden");
            return Err(CheckError::NoCurl);
        }
        Err(e) => {
            log::warn!("Update-Prüfung: curl ließ sich nicht starten ({e})");
            return Err(CheckError::Offline);
        }
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr);
            log::warn!("Update-Prüfung gescheitert ({}): {}", o.status, err.trim());
            // Zertifikat „noch nicht gültig“: Die Uhr geht nach, auch wenn sie nach dem Bau dieser Version steht
            return Err(if err.contains("not yet valid") { CheckError::ClockWrong } else { CheckError::Offline });
        }
        Ok(o) => o,
    };
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| CheckError::Offline)?;
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
    #[test]
    fn clock_before_build_is_wrong() {
        let build = 1_793_000_000; // November 2026
        assert!(super::before_build(978_307_200, build)); // 1. Januar 2001
        assert!(!super::before_build(build, build));
        // Ein paar Stunden Zeitzone schaden nicht
        assert!(!super::before_build(build - 6 * 3600, build));
        assert!(!super::before_build(build + 86_400 * 400, build));
    }

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
