// SPDX-License-Identifier: GPL-3.0-or-later
//! Historie: welche Titel an welchem Tag über Master zu hören waren, der Reihe nach.
//!
//! Keine Datenbank: je Tag eine Datei `historie/2026-10-14.json` neben den Einstellungen. Dasselbe Format
//! dient auch zum Exportieren und Importieren („FadeWerk-DJ-Historie“). Daneben lässt sich ein Tag als
//! M3U8-Playlist oder CSV-Tabelle exportieren und eine M3U-Playlist als Tag importieren.
//!
//! Ein Titel gilt als gespielt, sobald er [`PLAYED_AFTER_SECS`] lang über Master zu hören war
//! ([`PlayTracker`]). Titel bis [`DAY_STARTS_AT_HOUR`] Uhr früh zählen zum Vortag, damit eine Feier über
//! Mitternacht in einem Tag bleibt.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use jiff::civil;
use serde::{Deserialize, Serialize};

use crate::settings::{config_dir, write_atomic};

/// So lange muss ein Titel über Master zu hören sein, bis er in der Historie landet.
pub const PLAYED_AFTER_SECS: f64 = 30.0;
/// Vor dieser Stunde zählt ein Titel noch zum Vortag.
pub const DAY_STARTS_AT_HOUR: i8 = 6;
/// Kennung in exportierten Dateien.
const FORMAT: &str = "fadewerk-dj-history";
const VERSION: u32 = 1;

/// Ein gespielter Titel. Titel, Interpret usw. werden mitgespeichert, damit die Liste lesbar bleibt,
/// auch wenn die Datei später fehlt (oder auf einem anderen Rechner importiert wird).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: f64,
    /// Ortszeit, ab der der Titel zu hören war („2026-10-14T21:34:05“). Fehlt bei importierten Playlists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played_at: Option<String>,
    /// Player (0 = A, 1 = B). Fehlt bei importierten Playlists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deck: Option<u8>,
    /// Dateigröße, um die Datei nach einem Umzug oder auf einem anderen Rechner wiederzufinden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl Entry {
    /// Uhrzeit „21:34“ oder leer.
    pub fn clock(&self) -> String {
        self.played_at.as_deref().and_then(|t| t.parse::<civil::DateTime>().ok()).map(|t| t.strftime("%H:%M").to_string()).unwrap_or_default()
    }

    /// Anzeige „Interpret - Titel“ wie in M3U-Playlists üblich.
    fn display_name(&self) -> String {
        match (self.artist.is_empty(), self.title.is_empty()) {
            (false, false) => format!("{} - {}", self.artist, self.title),
            (true, false) => self.title.clone(),
            _ => self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
}

/// Ein Tag der Historie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Day {
    /// „2026-10-14“, zugleich Dateiname und Sortierschlüssel.
    pub date: String,
    /// Vom Nutzer vergebener Name, etwa „Omas Geburtstag“.
    pub name: String,
    pub entries: Vec<Entry>,
}

impl Day {
    /// Anzeige im Ordnerbaum: „2026-10-14 · Omas Geburtstag“.
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() { self.date.clone() } else { format!("{} · {}", self.date, self.name.trim()) }
    }

    /// Vorschlag für den Dateinamen beim Exportieren (ohne Endung).
    pub fn file_stem(&self) -> String {
        let name: String = self.name.trim().chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
        if name.is_empty() { format!("FadeWerk DJ {}", self.date) } else { format!("{} {}", self.date, name) }
    }
}

/// Gespeicherte bzw. exportierte Datei eines Tages.
#[derive(Serialize, Deserialize)]
struct DayFile {
    #[serde(default)]
    format: String,
    #[serde(default)]
    version: u32,
    date: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    entries: Vec<Entry>,
}

impl From<&Day> for DayFile {
    fn from(d: &Day) -> Self {
        Self { format: FORMAT.into(), version: VERSION, date: d.date.clone(), name: d.name.clone(), entries: d.entries.clone() }
    }
}

fn dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("historie"))
}

/// Jetzt in Ortszeit.
pub fn now_local() -> civil::DateTime {
    jiff::Zoned::now().datetime()
}

/// Tag, zu dem ein Zeitpunkt gehört: vor [`DAY_STARTS_AT_HOUR`] Uhr noch der Vortag.
pub fn history_date(at: civil::DateTime) -> civil::Date {
    if at.hour() < DAY_STARTS_AT_HOUR { at.date().yesterday().unwrap_or(at.date()) } else { at.date() }
}

/// Prüft eine Datumseingabe „2026-10-14“.
pub fn parse_date(s: &str) -> Option<String> {
    s.trim().parse::<civil::Date>().ok().map(|d| d.to_string())
}

#[derive(Default)]
pub struct History {
    days: BTreeMap<String, Day>,
    /// Seit dem letzten Speichern geänderte bzw. gelöschte Tage.
    dirty: HashSet<String>,
    deleted: HashSet<String>,
}

impl History {
    pub fn load() -> Self {
        let mut h = Self::default();
        let Some(dir) = dir() else { return h };
        let Ok(list) = std::fs::read_dir(&dir) else { return h };
        for file in list.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")) {
            match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|t| parse_json(&t)) {
                Ok(day) => {
                    h.days.insert(day.date.clone(), day);
                }
                Err(e) => log::warn!("Historie {} unlesbar: {e}", file.display()),
            }
        }
        h
    }

    pub fn save_dirty(&mut self) {
        let Some(dir) = dir() else { return };
        for date in std::mem::take(&mut self.deleted) {
            let file = dir.join(format!("{date}.json"));
            if let Err(e) = std::fs::remove_file(&file)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("Historie {} nicht gelöscht: {e}", file.display());
            }
        }
        for date in std::mem::take(&mut self.dirty) {
            let Some(day) = self.days.get(&date) else { continue };
            let file = dir.join(format!("{date}.json"));
            if let Err(e) = write_atomic(&file, &to_json(day)) {
                log::warn!("Historie {} nicht gespeichert: {e}", file.display());
                self.dirty.insert(date);
            }
        }
    }

    /// Alle Tage, der neueste zuerst.
    pub fn days(&self) -> impl Iterator<Item = &Day> {
        self.days.values().rev()
    }

    pub fn day(&self, date: &str) -> Option<&Day> {
        self.days.get(date)
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    fn touch(&mut self, date: &str) {
        self.deleted.remove(date);
        self.dirty.insert(date.to_owned());
    }

    /// Trägt einen gespielten Titel ein. Der Tag richtet sich nach `played_at` (siehe [`history_date`]).
    pub fn record(&mut self, entry: Entry) {
        let at = entry.played_at.as_deref().and_then(|t| t.parse::<civil::DateTime>().ok()).unwrap_or_else(now_local);
        let date = history_date(at).to_string();
        let day = self.days.entry(date.clone()).or_insert_with(|| Day { date: date.clone(), ..Default::default() });
        day.entries.push(entry);
        self.touch(&date);
    }

    pub fn rename(&mut self, date: &str, name: &str) {
        if let Some(d) = self.days.get_mut(date) {
            d.name = name.trim().to_owned();
            self.touch(date);
        }
    }

    /// Entfernt Einträge (Indizes in `entries`) aus einem Tag.
    pub fn remove_entries(&mut self, date: &str, indices: &[usize]) {
        if let Some(d) = self.days.get_mut(date) {
            let drop: HashSet<usize> = indices.iter().copied().collect();
            let mut i = 0;
            d.entries.retain(|_| {
                let keep = !drop.contains(&i);
                i += 1;
                keep
            });
            self.touch(date);
        }
    }

    pub fn delete_day(&mut self, date: &str) {
        if self.days.remove(date).is_some() {
            self.dirty.remove(date);
            self.deleted.insert(date.to_owned());
        }
    }

    /// Übernimmt einen importierten Tag. Gibt es den Tag schon, kommen nur Einträge dazu, die noch
    /// fehlen (gleiche Datei, gleiche Uhrzeit), nach Uhrzeit einsortiert; ein vorhandener Name bleibt.
    /// Liefert die Zahl der neuen Einträge.
    pub fn merge(&mut self, day: Day) -> usize {
        let date = day.date.clone();
        let added = match self.days.get_mut(&date) {
            None => {
                let n = day.entries.len();
                self.days.insert(date.clone(), day);
                n
            }
            Some(old) => {
                if old.name.trim().is_empty() {
                    old.name = day.name;
                }
                let mut n = 0;
                for e in day.entries {
                    let dup = old.entries.iter().any(|o| o.played_at == e.played_at && o.played_at.is_some() && same_file(&o.path, &e.path));
                    if !dup {
                        old.entries.push(e);
                        n += 1;
                    }
                }
                // Stabil sortieren: Einträge ohne Uhrzeit bleiben in ihrer Reihenfolge am Ende
                old.entries.sort_by(|a, b| match (&a.played_at, &b.played_at) {
                    (Some(x), Some(y)) => x.cmp(y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                });
                n
            }
        };
        self.touch(&date);
        added
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b || a.file_name() == b.file_name()
}

/// Erkennt, wann ein Titel als gespielt gilt: [`PLAYED_AFTER_SECS`] über Master hörbar. Gezählt wird der
/// Fortschritt über die bisher weiteste Stelle hinaus (nicht die Uhr), höchstens so schnell wie die Zeit
/// vergeht: Scratchen, Zurückspulen und Vorspulen zählen also nicht. Pausen unterbrechen nur, das Gehörte bleibt.
#[derive(Default)]
pub struct PlayTracker {
    id: u64,
    heard: f64,
    since: Option<civil::DateTime>,
    /// Weiteste gehörte Stelle; `None` = gerade nicht hörbar.
    furthest: Option<f64>,
    done: bool,
}

impl PlayTracker {
    /// Neu anfangen, etwa wenn der Titel zu Ende gelaufen ist und noch einmal gespielt wird.
    pub fn restart(&mut self) {
        *self = Self { id: self.id, ..Default::default() };
    }

    /// Einmal je Bild. `id` = geladener Titel (0 = keiner), `pos` = Position in Sekunden, `dt` = vergangene
    /// Zeit seit dem letzten Aufruf. Liefert genau einmal je Titel den Zeitpunkt, ab dem er zu hören war.
    pub fn update(&mut self, id: u64, audible: bool, pos: f64, dt: f64, now: impl FnOnce() -> civil::DateTime) -> Option<civil::DateTime> {
        if id != self.id {
            *self = Self { id, ..Default::default() };
        }
        if id == 0 || !audible {
            return None;
        }
        self.since.get_or_insert_with(now);
        let furthest = *self.furthest.get_or_insert(pos);
        if pos > furthest {
            // Etwas Luft für Pitch und ungleichmäßige Bilder
            self.heard += (pos - furthest).min(dt * 1.5 + 0.05);
            self.furthest = Some(pos);
        }
        if !self.done && self.heard >= PLAYED_AFTER_SECS {
            self.done = true;
            return self.since;
        }
        None
    }
}

// ---------- Export ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// Playlist (M3U8) für andere Player.
    M3u,
    /// Tabelle für Excel und Co.
    Csv,
    /// Eigenes Format mit allen Angaben, lässt sich wieder importieren.
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::M3u => "m3u8",
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

pub fn to_json(day: &Day) -> String {
    serde_json::to_string_pretty(&DayFile::from(day)).unwrap_or_default()
}

/// Erweiterte M3U-Playlist (UTF-8) mit absoluten Pfaden.
pub fn to_m3u(day: &Day) -> String {
    let mut s = String::from("#EXTM3U\n");
    if !day.name.trim().is_empty() {
        s.push_str(&format!("#PLAYLIST:{}\n", day.name.trim()));
    }
    for e in &day.entries {
        let secs = if e.duration_secs > 0.0 { e.duration_secs.round() as i64 } else { -1 };
        s.push_str(&format!("#EXTINF:{secs},{}\n{}\n", e.display_name(), e.path.display()));
    }
    s
}

/// Tabelle mit Uhrzeit, Interpret, Titel usw. `sep` = Trennzeichen (deutsches Excel erwartet `;`).
/// `headers` sind die übersetzten Spaltenköpfe in der Reihenfolge Nr, Uhrzeit, Interpret, Titel, Album,
/// Länge, Player, Datei.
pub fn to_csv(day: &Day, sep: char, headers: [&str; 8]) -> String {
    let field = |s: &str| {
        if s.contains(sep) || s.contains('"') || s.contains('\n') || s.contains('\r') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let line = |cols: &[String]| cols.iter().map(|c| field(c)).collect::<Vec<_>>().join(&sep.to_string()) + "\r\n";
    // Byte-Order-Mark, damit Excel Umlaute richtig liest
    let mut s = String::from("\u{feff}");
    s.push_str(&line(&headers.map(str::to_owned)));
    for (i, e) in day.entries.iter().enumerate() {
        let len = if e.duration_secs > 0.0 { crate::deck_ui::fmt_time(e.duration_secs) } else { String::new() };
        let deck = e.deck.map(|d| crate::theme::deck_letter(d as usize).to_owned()).unwrap_or_default();
        s.push_str(&line(&[
            (i + 1).to_string(),
            e.clock(),
            e.artist.clone(),
            e.title.clone(),
            e.album.clone(),
            len,
            deck,
            e.path.display().to_string(),
        ]));
    }
    s
}

// ---------- Import ----------

/// Gelesene Importdatei.
pub enum Imported {
    /// Eigene Historie: Datum und Name stehen in der Datei.
    Day(Day),
    /// Playlist (M3U): Datum und Name fragt das Importfenster ab. `name` ist ein Vorschlag.
    List { name: String, entries: Vec<Entry> },
}

fn parse_json(text: &str) -> Result<Day, String> {
    let f: DayFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !f.format.is_empty() && f.format != FORMAT {
        return Err(format!("unbekanntes Format „{}“", f.format));
    }
    let date = parse_date(&f.date).ok_or_else(|| format!("ungültiges Datum „{}“", f.date))?;
    // Unsinnige Werte aus beschädigten oder fremden Dateien entschärfen
    let mut entries = f.entries;
    for e in &mut entries {
        if !e.duration_secs.is_finite() || e.duration_secs < 0.0 {
            e.duration_secs = 0.0;
        }
        if e.deck.is_some_and(|d| d > 1) {
            e.deck = None;
        }
    }
    Ok(Day { date, name: f.name, entries })
}

/// Liest eine M3U/M3U8-Playlist. Relative Pfade gelten ab dem Ordner der Playlist.
fn parse_m3u(text: &str, base: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut info: Option<(f64, String)> = None;
    for line in text.lines().map(|l| l.trim().trim_start_matches('\u{feff}')) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (secs, name) = rest.split_once(',').unwrap_or((rest, ""));
            info = Some((secs.trim().parse::<f64>().unwrap_or(0.0).max(0.0), name.trim().to_owned()));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let raw = line.strip_prefix("file://").map(percent_decode).unwrap_or_else(|| line.to_owned());
        let p = PathBuf::from(&raw);
        let path = if p.is_absolute() || raw.contains(":\\") { p } else { base.join(p) };
        let (duration_secs, name) = info.take().unwrap_or_default();
        let fallback = fw_library::TrackMeta::from_filename(&path);
        let (artist, title) = match name.split_once(" - ") {
            Some((a, t)) => (a.trim().to_owned(), t.trim().to_owned()),
            None if !name.is_empty() => (String::new(), name),
            None => (fallback.artist, fallback.title),
        };
        out.push(Entry { path, title, artist, duration_secs, ..Default::default() });
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Liest eine Importdatei (eigene Historie oder M3U/M3U8).
pub fn read_import(file: &Path) -> Result<Imported, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "json" || text.trim_start().starts_with('{') {
        return parse_json(&text).map(Imported::Day);
    }
    let base = file.parent().unwrap_or(Path::new("."));
    let entries = parse_m3u(&text, base);
    if entries.is_empty() {
        return Err("keine Titel gefunden".into());
    }
    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Imported::List { name, entries })
}

/// Sucht fehlende Dateien in `roots` über Dateinamen (und Größe, falls bekannt), etwa nach dem Import von
/// einem anderen Rechner. Liefert, wie viele gefunden wurden.
pub fn relocate(entries: &mut [Entry], roots: &[PathBuf]) -> usize {
    let missing: HashSet<std::ffi::OsString> = entries.iter().filter(|e| !e.path.is_file()).filter_map(|e| e.path.file_name().map(|n| n.to_owned())).collect();
    if missing.is_empty() {
        return 0;
    }
    let mut found: HashMap<std::ffi::OsString, Vec<PathBuf>> = HashMap::new();
    for root in roots.iter().filter(|r| r.is_dir()) {
        fw_library::walk_audio(root, || true, |tracks| {
            for t in tracks {
                if let Some(n) = t.file_name().filter(|n| missing.contains(*n)) {
                    found.entry(n.to_owned()).or_default().push(t.clone());
                }
            }
        });
    }
    let mut n = 0;
    for e in entries.iter_mut().filter(|e| !e.path.is_file()) {
        let Some(cands) = e.path.file_name().and_then(|n| found.get(n)) else { continue };
        let size_ok = |p: &PathBuf| e.size.is_none_or(|s| std::fs::metadata(p).is_ok_and(|m| m.len() == s));
        if let Some(p) = cands.iter().find(|p| size_ok(p)) {
            e.path = p.clone();
            n += 1;
        }
    }
    n
}

/// Ergebnis eines Imports im Hintergrund: gelesene Datei und Zahl der wiedergefundenen Titel.
pub type ImportResult = Result<(Imported, usize), String>;

/// Liest und sucht im Thread `fw-history-import`, weil das Durchsuchen der Musikordner dauern kann.
pub fn import_in_background(file: PathBuf, roots: Vec<PathBuf>, repaint: impl Fn() + Send + 'static) -> Receiver<ImportResult> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("fw-history-import".into()).spawn(move || {
        let result = read_import(&file).map(|mut imp| {
            let entries = match &mut imp {
                Imported::Day(d) => &mut d.entries,
                Imported::List { entries, .. } => entries,
            };
            let n = relocate(entries, &roots);
            (imp, n)
        });
        let _ = tx.send(result);
        repaint();
    });
    if let Err(e) = spawned {
        log::warn!("Import startet nicht: {e}");
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_history_and_playlist_files_are_handled() {
        for text in ["", "{", "[]", "null", r#"{"date":"gestern"}"#, r#"{"date":"2026-13-45","entries":[]}"#, r#"{"date":"2026-10-14","entries":"kaputt"}"#] {
            assert!(parse_json(text).is_err(), "{text:?} sollte ein Fehler sein");
        }
        let day = parse_json(r#"{"date":"2026-10-14","entries":[{"path":"/a.mp3","duration_secs":-4,"deck":7,"played_at":"Unsinn"}]}"#).unwrap();
        assert_eq!((day.entries[0].duration_secs, day.entries[0].deck), (0.0, None));
        assert_eq!(day.entries[0].clock(), "");
        // M3U mit Müll, leeren Zeilen, Steuerzeichen und kaputten Prozent-Kodierungen
        let m3u = "#EXTM3U\n\n#EXTINF:abc,\n%ZZ%4\n\u{0}\u{7}\nfile:///%\n../../../x.mp3\n#EXTINF:-1,Titel\nC:\\Musik\\a.mp3\n";
        let _ = parse_m3u(m3u, Path::new("/Musik/Listen"));
    }

    fn dt(s: &str) -> civil::DateTime {
        s.parse().unwrap()
    }

    #[test]
    fn early_morning_belongs_to_the_day_before() {
        assert_eq!(history_date(dt("2026-10-14T23:59:00")).to_string(), "2026-10-14");
        assert_eq!(history_date(dt("2026-10-15T05:59:59")).to_string(), "2026-10-14");
        assert_eq!(history_date(dt("2026-10-15T06:00:00")).to_string(), "2026-10-15");
    }

    #[test]
    fn a_track_counts_after_thirty_audible_seconds() {
        let mut t = PlayTracker::default();
        let now = || dt("2026-10-14T20:00:00");
        // Vorhören bzw. stumm: zählt nicht
        assert!(t.update(1, false, 0.0, 0.1, now).is_none());
        let mut pos = 0.0;
        let mut hit = None;
        for _ in 0..400 {
            pos += 0.1;
            if let Some(at) = t.update(1, true, pos, 0.1, now) {
                hit = Some((at, pos));
            }
        }
        let (at, pos_at) = hit.expect("nach 30 s eingetragen");
        assert_eq!(at, now());
        assert!((pos_at - 30.1).abs() < 0.2, "{pos_at}");
        // Nur einmal
        assert!(t.update(1, true, pos + 0.1, 0.1, now).is_none());
    }

    #[test]
    fn seeking_and_scratching_do_not_count() {
        let mut t = PlayTracker::default();
        let now = || dt("2026-10-14T20:00:00");
        t.update(1, true, 0.0, 0.1, now);
        // Sprung um 60 s in einem Bild zählt nur so viel wie die Zeit
        assert!(t.update(1, true, 60.0, 0.1, now).is_none());
        // Hin und her scratchen
        for i in 0..1000 {
            let pos = 60.0 + if i % 2 == 0 { 0.1 } else { 0.0 };
            assert!(t.update(1, true, pos, 0.1, now).is_none());
        }
        // Ein anderer Titel fängt von vorn an
        assert!(t.update(2, true, 0.0, 0.1, now).is_none());
    }

    #[test]
    fn days_record_rename_and_merge() {
        let mut h = History::default();
        let e = |p: &str, at: &str| Entry { path: p.into(), title: p.into(), played_at: Some(at.into()), deck: Some(0), ..Default::default() };
        h.record(e("/a.mp3", "2026-10-14T21:00:00"));
        h.record(e("/b.mp3", "2026-10-15T01:30:00"));
        h.record(e("/c.mp3", "2026-10-15T20:00:00"));
        let dates: Vec<_> = h.days().map(|d| d.date.clone()).collect();
        assert_eq!(dates, ["2026-10-15", "2026-10-14"]);
        assert_eq!(h.day("2026-10-14").unwrap().entries.len(), 2);
        h.rename("2026-10-14", "  Omas Geburtstag ");
        assert_eq!(h.day("2026-10-14").unwrap().label(), "2026-10-14 · Omas Geburtstag");

        // Export und Re-Import: nichts doppelt
        let json = to_json(h.day("2026-10-14").unwrap());
        let day = parse_json(&json).unwrap();
        assert_eq!(h.merge(day.clone()), 0);
        // Auf einem leeren Rechner kommt alles an, mit Namen
        let mut other = History::default();
        assert_eq!(other.merge(day), 2);
        assert_eq!(other.day("2026-10-14").unwrap().name, "Omas Geburtstag");

        h.remove_entries("2026-10-14", &[0]);
        assert_eq!(h.day("2026-10-14").unwrap().entries[0].path, PathBuf::from("/b.mp3"));
        h.delete_day("2026-10-14");
        assert!(h.day("2026-10-14").is_none());
    }

    #[test]
    fn m3u_round_trip_and_csv_quoting() {
        let day = Day {
            date: "2026-10-14".into(),
            name: "Feier".into(),
            entries: vec![Entry {
                path: "/Musik/Künstler - Lied; Teil 1.mp3".into(),
                title: "Lied; Teil 1".into(),
                artist: "Künstler".into(),
                duration_secs: 201.0,
                played_at: Some("2026-10-14T21:05:00".into()),
                deck: Some(1),
                ..Default::default()
            }],
        };
        let m3u = to_m3u(&day);
        assert!(m3u.contains("#EXTINF:201,Künstler - Lied; Teil 1\n/Musik/"));
        let back = parse_m3u(&m3u, Path::new("/"));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].path, day.entries[0].path);
        assert_eq!((back[0].artist.as_str(), back[0].title.as_str()), ("Künstler", "Lied; Teil 1"));
        let csv = to_csv(&day, ';', ["Nr", "Zeit", "Interpret", "Titel", "Album", "Länge", "Player", "Datei"]);
        assert!(csv.contains("1;21:05;Künstler;\"Lied; Teil 1\";;3:21;B;"), "{csv}");
        // Relative Pfade und file://-Adressen
        let rel = parse_m3u("#EXTM3U\nSub/a%20b.mp3\nfile:///x/a%20b.mp3\n", Path::new("/base"));
        assert_eq!(rel[0].path, PathBuf::from("/base/Sub/a%20b.mp3"));
        assert_eq!(rel[1].path, PathBuf::from("/x/a b.mp3"));
    }
}
