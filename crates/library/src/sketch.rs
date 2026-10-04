// SPDX-License-Identifier: GPL-3.0-or-later
//! Kleine Wellenform eines Titels für die Playlist-Zeilen (FadeWerk-Modus).
//!
//! Gespeichert werden nur Spitzenwerte als Bytes: eine Übersicht über den ganzen Titel und die ersten und
//! letzten [`EDGE_SECS`] Sekunden fein aufgelöst (dort setzt man die Fade-Bereiche). Zusammen wenige KB,
//! das Audio selbst wird nach dem Berechnen sofort verworfen.
//!
//! Der Cache auf der Platte ist ein Ordner mit einer Datei je Titel. Der Dateiname ist ein Hash aus Pfad,
//! Größe und Änderungszeit; ändert sich die Datei, passt der Name nicht mehr und sie wird neu berechnet.
//! Dateien, die [`EXPIRE_DAYS`] lang nicht gebraucht wurden, räumt [`clean_cache`] weg.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::waveform::Waveform;

/// Säulen der Übersicht über den ganzen Titel.
pub const OVERVIEW_COLS: usize = 480;
/// Länge der fein aufgelösten Stücke am Anfang und Ende (Sekunden).
pub const EDGE_SECS: f32 = 30.0;
/// Auflösung der Stücke am Anfang und Ende (Werte pro Sekunde).
pub const EDGE_RATE: f32 = 50.0;
/// So lange bleibt eine Cache-Datei, die nicht mehr gelesen wurde.
pub const EXPIRE_DAYS: u64 = 90;

const MAGIC: &[u8; 4] = b"FWW1";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaveSketch {
    pub duration_secs: f32,
    /// Spitzenwerte 0..255 über den ganzen Titel, [`OVERVIEW_COLS`] Stück (kürzer bei sehr kurzen Titeln).
    pub overview: Vec<u8>,
    /// Die ersten [`EDGE_SECS`] Sekunden mit [`EDGE_RATE`] Werten pro Sekunde.
    pub head: Vec<u8>,
    /// Die letzten [`EDGE_SECS`] Sekunden; der letzte Wert gehört zum Titelende.
    pub tail: Vec<u8>,
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl WaveSketch {
    pub fn from_waveform(wf: &Waveform) -> Self {
        let dur = wf.duration_secs();
        if dur <= 0.0 {
            return Self::default();
        }
        let cols = OVERVIEW_COLS.min(wf.peaks.len());
        let overview = (0..cols)
            .map(|i| {
                let a = i as f64 / cols as f64 * dur as f64;
                byte(wf.max_in(a, a + dur as f64 / cols as f64))
            })
            .collect();
        let edge = (EDGE_SECS.min(dur) * EDGE_RATE) as usize;
        let step = 1.0 / EDGE_RATE as f64;
        let head = (0..edge).map(|i| byte(wf.max_in(i as f64 * step, (i + 1) as f64 * step))).collect();
        let start = dur as f64 - edge as f64 * step;
        let tail = (0..edge).map(|i| byte(wf.max_in(start + i as f64 * step, start + (i + 1) as f64 * step))).collect();
        Self { duration_secs: dur, overview, head, tail }
    }

    /// Größter Wert der Übersicht im Anteil `from..to` des Titels (0..1), als 0..1.
    pub fn overview_max(&self, from: f32, to: f32) -> f32 {
        let n = self.overview.len();
        if n == 0 {
            return 0.0;
        }
        let a = ((from.max(0.0) * n as f32) as usize).min(n - 1);
        let b = ((to * n as f32).ceil() as usize).clamp(a + 1, n);
        self.overview[a..b].iter().copied().max().unwrap_or(0) as f32 / 255.0
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.overview.len() + self.head.len() + self.tail.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.duration_secs.to_le_bytes());
        for part in [&self.overview, &self.head, &self.tail] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let rest = bytes.strip_prefix(MAGIC)?;
        let duration_secs = f32::from_le_bytes(rest.get(0..4)?.try_into().ok()?);
        if !duration_secs.is_finite() || duration_secs < 0.0 {
            return None;
        }
        let mut pos = 4;
        let mut part = || -> Option<Vec<u8>> {
            let len = u32::from_le_bytes(rest.get(pos..pos + 4)?.try_into().ok()?) as usize;
            let data = rest.get(pos + 4..pos.checked_add(4 + len)?)?.to_vec();
            pos += 4 + len;
            Some(data)
        };
        let (overview, head, tail) = (part()?, part()?, part()?);
        let edge_max = (EDGE_SECS * EDGE_RATE) as usize;
        if overview.len() > OVERVIEW_COLS || head.len() > edge_max || tail.len() > edge_max {
            return None;
        }
        Some(Self { duration_secs, overview, head, tail })
    }
}

/// FNV-1a, 64 Bit. Bewusst nicht `DefaultHasher`: dessen Ergebnis darf sich zwischen Rust-Versionen ändern,
/// dann wäre der Cache nach jedem Update wertlos.
fn fnv1a(data: &[u8], mut h: u64) -> u64 {
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Name der Cache-Datei für `file`, `None` wenn die Datei nicht lesbar ist.
fn cache_name(file: &Path) -> Option<String> {
    let m = std::fs::metadata(file).ok()?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
    let mut h = fnv1a(file.as_os_str().as_encoded_bytes(), 0xcbf2_9ce4_8422_2325);
    h = fnv1a(&m.len().to_le_bytes(), h);
    h = fnv1a(&modified.to_le_bytes(), h);
    Some(format!("{h:016x}.fww"))
}

/// Liest die gemerkte Skizze für `file`. Frischt dabei die Änderungszeit der Cache-Datei auf,
/// damit oft gebrauchte Einträge nicht weggeräumt werden.
pub fn read_cached(dir: &Path, file: &Path) -> Option<WaveSketch> {
    let path = dir.join(cache_name(file)?);
    let sketch = WaveSketch::from_bytes(&std::fs::read(&path).ok()?)?;
    if let Ok(f) = std::fs::File::options().append(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(sketch)
}

/// Merkt die Skizze für `file`. Fehler werden nur geloggt (der Cache ist entbehrlich).
pub fn write_cached(dir: &Path, file: &Path, sketch: &WaveSketch) {
    let Some(name) = cache_name(file) else { return };
    let result = std::fs::create_dir_all(dir).and_then(|_| {
        // Erst unter anderem Namen schreiben: ein gleichzeitiger Leser sieht nie eine halbe Datei
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, sketch.to_bytes())?;
        std::fs::rename(&tmp, dir.join(&name))
    });
    if let Err(e) = result {
        log::debug!("Wellenform-Cache für {}: {e}", file.display());
    }
}

/// Löscht Cache-Dateien, die länger als [`EXPIRE_DAYS`] nicht gelesen wurden. Liefert die Anzahl.
pub fn clean_cache(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let limit = SystemTime::now() - Duration::from_secs(EXPIRE_DAYS * 24 * 60 * 60);
    let mut removed = 0;
    for e in entries.flatten() {
        let path: PathBuf = e.path();
        let ours = path.extension().is_some_and(|x| x == "fww" || x == "tmp");
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < limit);
        if ours && old && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::WAVE_RATE;

    fn wave(secs: f32) -> Waveform {
        let n = (secs * WAVE_RATE) as usize;
        Waveform { peaks: (0..n).map(|i| (i % 100) as f32 / 99.0).collect() }
    }

    #[test]
    fn sketch_has_overview_and_edges() {
        let s = WaveSketch::from_waveform(&wave(200.0));
        assert_eq!(s.overview.len(), OVERVIEW_COLS);
        assert_eq!(s.head.len(), (EDGE_SECS * EDGE_RATE) as usize);
        assert_eq!(s.tail.len(), s.head.len());
        assert!((s.duration_secs - 200.0).abs() < 0.01);
        // Kurzer Titel: Stücke höchstens so lang wie der Titel
        let s = WaveSketch::from_waveform(&wave(4.0));
        assert_eq!(s.head.len(), (4.0 * EDGE_RATE) as usize);
        assert_eq!(s.overview.len(), OVERVIEW_COLS.min(400));
        assert_eq!(WaveSketch::from_waveform(&Waveform::default()), WaveSketch::default());
    }

    #[test]
    fn bytes_round_trip_and_reject_garbage() {
        let s = WaveSketch::from_waveform(&wave(90.0));
        assert_eq!(WaveSketch::from_bytes(&s.to_bytes()), Some(s.clone()));
        let b = s.to_bytes();
        assert_eq!(WaveSketch::from_bytes(&b[..b.len() - 1]), None);
        assert_eq!(WaveSketch::from_bytes(b"FWW1"), None);
        assert_eq!(WaveSketch::from_bytes(&[0xff; 64]), None);
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&1.0f32.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(WaveSketch::from_bytes(&huge), None);
    }

    #[test]
    fn cache_finds_unchanged_file_only() {
        let dir = std::env::temp_dir().join(format!("fw-sketch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("titel.wav");
        std::fs::write(&file, b"abc").unwrap();
        let cache = dir.join("cache");
        let s = WaveSketch::from_waveform(&wave(60.0));
        assert_eq!(read_cached(&cache, &file), None);
        write_cached(&cache, &file, &s);
        assert_eq!(read_cached(&cache, &file), Some(s));
        // Geänderte Datei (andere Größe): Eintrag passt nicht mehr
        std::fs::write(&file, b"abcd").unwrap();
        assert_eq!(read_cached(&cache, &file), None);
        assert_eq!(clean_cache(&cache), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
