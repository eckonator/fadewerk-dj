// SPDX-License-Identifier: GPL-3.0-or-later
//! Bettet unter Windows Programm-Icon und Versionsangaben in die .exe ein (Explorer, Taskleiste,
//! Verknüpfungen). Das Fenster-Icon zur Laufzeit setzt main.rs, auf allen Systemen.

fn main() {
    println!("cargo:rerun-if-changed=../assets/icon.ico");
    // Bauzeitpunkt (Unix-Sekunden) als Untergrenze für die Uhr des Rechners (`update::clock_wrong`). Wird nur bei
    // geändertem build.rs neu ermittelt; ein älterer Wert schadet nicht, er ist dann nur eine schwächere Grenze
    let secs = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
    });
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rustc-env=FW_BUILD_SECS={secs}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../assets/icon.ico")
        .set("ProductName", "FadeWerk DJ")
        .set("FileDescription", "FadeWerk DJ")
        .set("OriginalFilename", "FadeWerk DJ.exe")
        // Nur ASCII: rc.exe liest die Datei je nach System nicht als UTF-8
        .set("LegalCopyright", "Copyright (C) 2026 Markus Eckert, GPL-3.0-or-later");
    if let Err(e) = res.compile() {
        // Ohne Icon weiterbauen statt den ganzen Build scheitern zu lassen (z. B. fehlendes rc.exe)
        println!("cargo:warning=Windows-Ressourcen nicht eingebettet: {e}");
    }
}
