// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::{Path, PathBuf};

/// Dateiendungen, die als Audio gelten.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "wave", "aif", "aiff", "ogg", "oga", "m4a", "mp4", "aac", "alac", "caf", "mka",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Inhalt eines Ordners, eine Ebene tief, sortiert.
#[derive(Debug, Default, Clone)]
pub struct DirListing {
    pub dirs: Vec<PathBuf>,
    pub tracks: Vec<PathBuf>,
}

/// Liest einen Ordner sofort, ohne Tags zu öffnen. Versteckte Einträge werden übersprungen.
pub fn list_dir(path: &Path) -> std::io::Result<DirListing> {
    let mut out = DirListing::default();
    for entry in std::fs::read_dir(path)? {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        // Verknüpfungen auflösen, damit verlinkte Ordner und Dateien auftauchen
        let is_dir = if ft.is_symlink() { p.is_dir() } else { ft.is_dir() };
        if is_dir {
            out.dirs.push(p);
        } else if is_audio(&p) {
            out.tracks.push(p);
        }
    }
    let key = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    out.dirs.sort_by_key(key);
    out.tracks.sort_by_key(key);
    Ok(out)
}

/// Ordner, in denen keine Musik liegt und die eine Suche über ganze Laufwerke nicht betreten soll:
/// System, Programme, Papierkorb, Entwickler-Ordner. Gilt nur für Unterordner beim Durchsuchen;
/// wählt der Nutzer so einen Ordner selbst aus, wird er trotzdem gezeigt.
pub fn is_system_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    let lower = name.to_lowercase();
    // Überall: Papierkorb, Programmdaten, Entwickler-Ordner
    if matches!(lower.as_str(), "$recycle.bin" | "system volume information" | "appdata" | "node_modules" | "__pycache__") {
        return true;
    }
    let parent = path.parent();
    // Direkt unter einem Laufwerk (`/`, `C:\`)
    let at_drive_root = parent.is_some_and(|p| p.parent().is_none());
    if at_drive_root
        && matches!(
            lower.as_str(),
            "system" | "library" | "applications" | "private" | "cores" | "volumes" | "windows" | "program files"
                | "program files (x86)" | "programdata" | "recovery" | "perflogs" | "proc" | "sys" | "dev" | "usr"
                | "bin" | "sbin" | "etc" | "var" | "boot" | "lib" | "lib64" | "opt" | "run" | "snap" | "tmp" | "srv"
        )
    {
        return true;
    }
    // Direkt im Benutzerordner: macOS-Bibliothek und Programme
    let in_home = parent.is_some_and(|p| dirs::home_dir().is_some_and(|h| h == p));
    in_home && matches!(lower.as_str(), "library" | "applications")
}

/// Sucht Audiodateien in `root` und allen Unterordnern, Ordner für Ordner in sortierter Reihenfolge
/// (erst die Titel eines Ordners, dann seine Unterordner). `emit` bekommt die Titel je Ordner,
/// `keep_going` erlaubt den Abbruch zwischen zwei Ordnern. Verknüpfte Ordner werden nur einmal besucht.
pub fn walk_audio(root: &Path, mut keep_going: impl FnMut() -> bool, mut emit: impl FnMut(Vec<PathBuf>)) {
    const MAX_DEPTH: usize = 32;
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if !keep_going() {
            return;
        }
        let key = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !visited.insert(key) {
            continue;
        }
        let Ok(listing) = list_dir(&dir) else { continue };
        if !listing.tracks.is_empty() {
            emit(listing.tracks);
        }
        if depth < MAX_DEPTH {
            // Rückwärts auf den Stapel, damit der alphabetisch erste Unterordner zuerst dran ist
            stack.extend(listing.dirs.into_iter().filter(|d| !is_system_dir(d)).rev().map(|d| (d, depth + 1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_audio_by_extension() {
        assert!(is_audio(Path::new("/x/01 Track.MP3")));
        assert!(is_audio(Path::new("a.flac")));
        assert!(!is_audio(Path::new("cover.jpg")));
        assert!(!is_audio(Path::new("README")));
    }

    #[test]
    fn lists_folders_and_tracks_sorted() {
        let dir = std::env::temp_dir().join(format!("fw-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("B Album")).unwrap();
        std::fs::create_dir_all(dir.join("a Album")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["02 b.mp3", "01 a.flac", "cover.jpg"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let l = list_dir(&dir).unwrap();
        let names = |v: &[PathBuf]| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(&l.dirs), ["a Album", "B Album"]);
        assert_eq!(names(&l.tracks), ["01 a.flac", "02 b.mp3"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn skips_system_folders() {
        let root = if cfg!(windows) { Path::new("C:\\") } else { Path::new("/") };
        assert!(is_system_dir(&root.join(if cfg!(windows) { "Windows" } else { "System" })));
        assert!(is_system_dir(&root.join("Program Files")));
        assert!(is_system_dir(Path::new("/x/y/node_modules")));
        assert!(is_system_dir(&root.join("Users").join("dj").join("AppData")));
        // Gleiche Namen tiefer in der Musiksammlung sind erlaubt
        assert!(!is_system_dir(&root.join("Musik").join("System")));
        assert!(!is_system_dir(&root.join("Musik").join("Library")));
        if let Some(home) = dirs::home_dir() {
            assert!(is_system_dir(&home.join("Library")));
            assert!(!is_system_dir(&home.join("Music")));
        }
    }

    #[test]
    fn walks_subfolders_in_order() {
        let dir = std::env::temp_dir().join(format!("fw-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["CD 2", "CD 1/Bonus"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in ["intro.mp3", "CD 1/01.mp3", "CD 1/Bonus/x.flac", "CD 2/01.mp3", "CD 2/notes.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let mut found = Vec::new();
        walk_audio(&dir, || true, |batch| found.extend(batch));
        // Mit „/“ zusammensetzen, damit der Vergleich auch unter Windows („\“) passt
        let rel: Vec<String> = found
            .iter()
            .map(|p| {
                let parts: Vec<_> = p.strip_prefix(&dir).unwrap().components().map(|c| c.as_os_str().to_string_lossy()).collect();
                parts.join("/")
            })
            .collect();
        assert_eq!(rel, ["intro.mp3", "CD 1/01.mp3", "CD 1/Bonus/x.flac", "CD 2/01.mp3"]);

        let mut first_only = Vec::new();
        let mut calls = 0;
        walk_audio(&dir, || { calls += 1; calls <= 1 }, |batch| first_only.extend(batch));
        assert_eq!(first_only.len(), 1, "Abbruch nach dem ersten Ordner");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
