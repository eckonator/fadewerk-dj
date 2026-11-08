// SPDX-License-Identifier: GPL-3.0-or-later
//! Beliebtheit der Titel aus ListenBrainz (offene Hördaten), nur auf Knopfdruck.
//!
//! ListenBrainz zählt Wiedergaben fast nur auf der „kanonischen“ Aufnahme eines Liedes; die Kennung, die ein
//! Sampler in der Datei trägt, hat oft keine Zahlen. Ohne Anmeldung beantwortet ListenBrainz nur noch die
//! Stapelabfrage je Aufnahme-Kennung (Ranglisten und Titelsuche verlangen seit 2026 einen Token). Darum je
//! Titel: MusicBrainz sucht die Aufnahmen mit diesem Interpreten und Titel, ListenBrainz liefert für alle
//! Kandidaten die Wiedergaben, der höchste Wert gilt (das ist die kanonische Aufnahme). Gemerkt je
//! „Interpret + Titel“ in `beliebtheit.json`.
//!
//! Gezeigt wird die Beliebtheit im Vergleich zum größten Hit desselben Albums (bzw. Samplers), dazu der Platz
//! darin; so stechen die Hits eines Albums heraus. Hat ein Titel im Album keine Vergleichspartner (gemischte
//! Ordner wie „Top 100“, deren Dateien ihr Original-Album tragen), gilt der Ordner als Gruppe.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fw_library::TrackMeta;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{config_dir, write_atomic};

const FILE: &str = "beliebtheit.json";
const FORMAT: &str = "FadeWerk-DJ-Beliebtheit";
const VERSION: u32 = 2;
/// Danach wird ein Titel bei der nächsten Abfrage neu geholt.
const REFRESH_DAYS: u64 = 30;
const DAY_SECS: u64 = 24 * 60 * 60;
/// ListenBrainz verlangt höchstens eine Abfrage je Sekunde und Programm.
const LB_GAP: Duration = Duration::from_millis(1100);
/// Platz im Album, bis zu dem ein Titel als Hit gilt (nur in Alben mit mehr Titeln).
pub const HIT_RANK: u16 = 3;
/// Mindestgüte eines MusicBrainz-Treffers (0 bis 100) und höchstens so viele Kandidaten je Titel.
const MIN_SCORE: i64 = 80;
const MAX_CANDIDATES: usize = 50;

/// Gemerkte Hördaten eines Titels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Track {
    /// Abgefragt am, Sekunden seit 1970.
    #[serde(rename = "t")]
    at: u64,
    /// Höchste Zahl der Wiedergaben unter den Aufnahmen; `None` = nicht gefunden.
    #[serde(rename = "w", default, skip_serializing_if = "Option::is_none")]
    listens: Option<u64>,
    /// Album (bereinigt), zu dem der Titel beim Abfragen gehörte: Vergleichsgruppe.
    #[serde(rename = "a", default, skip_serializing_if = "String::is_empty")]
    album: String,
    /// Ordner der Datei beim Abfragen: Vergleichsgruppe, wenn das Album allein steht.
    #[serde(rename = "o", default, skip_serializing_if = "String::is_empty")]
    dir: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    #[serde(default)]
    tracks: HashMap<String, Track>,
}

/// Beliebtheit eines Titels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pop {
    /// Noch nicht abgefragt.
    Unknown,
    /// Abgefragt, aber ListenBrainz kennt keine Wiedergaben.
    NotFound,
    /// Wiedergaben; in der Vergleichsgruppe (Album, sonst Ordner) der größte Wert, Platz und Zahl der
    /// abgefragten Titel (`group_size` 0 = ohne Gruppe).
    Found { listens: u64, max: u64, rank: u16, group_size: u16, by_folder: bool },
}

impl Pop {
    /// Anteil am größten Hit des Albums (0 bis 1). Ohne Album auf einer logarithmischen Skala bis 10 Mio.
    pub fn share(self) -> Option<f32> {
        match self {
            Pop::Found { listens, group_size: 0, .. } => Some(((listens as f64 + 1.0).log10() / 7.0).clamp(0.0, 1.0) as f32),
            Pop::Found { listens, max, .. } if max > 0 => Some((listens as f64 / max as f64).clamp(0.0, 1.0) as f32),
            _ => None,
        }
    }

    /// Einer der größten Hits seines Albums bzw. Ordners?
    pub fn is_hit(self) -> bool {
        matches!(self, Pop::Found { rank, group_size, .. } if group_size > HIT_RANK && rank <= HIT_RANK)
    }
}

/// Stand einer laufenden Abfrage: (fertig, gesamt) Titel.
pub type Progress = (usize, usize);

struct Job {
    rx: Receiver<(String, Track)>,
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
}

#[derive(Default)]
pub struct Popularity {
    tracks: HashMap<String, Track>,
    /// Album bzw. Ordner → Wiedergaben der abgefragten Titel, absteigend (aus `tracks` abgeleitet).
    albums: HashMap<String, Vec<u64>>,
    dirs: HashMap<String, Vec<u64>>,
    dirty: bool,
    job: Option<Job>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Text für Vergleiche: klein, ohne Akzente und Satzzeichen.
fn simple(s: &str) -> String {
    let folded = crate::browser::fold(s);
    let cleaned: String = folded.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Schreibweise ohne Gäste („feat. …“).
fn without_guests(s: &str) -> &str {
    let lower = s.to_lowercase();
    let cut = [" feat. ", " feat ", " ft. ", " featuring ", " (feat", " [feat"].iter().filter_map(|f| lower.find(f)).min().unwrap_or(s.len());
    // `lower` kann bei manchen Zeichen andere Längen haben; nur an gültigen Grenzen schneiden
    if s.is_char_boundary(cut) { s[..cut].trim() } else { s.trim() }
}

/// Titel ohne Zusätze wie „(Radio Edit)“, „[Remastered]“, „- Remastered 2011“ und ohne Gäste.
pub fn title_key(title: &str) -> String {
    let t = without_guests(title);
    let t = t.split(['(', '[']).next().unwrap_or(t);
    let t = t.split(" - ").next().unwrap_or(t);
    simple(t)
}

/// Mögliche Schlüssel des Interpreten: der ganze Name ohne Gäste, dann der erste von mehreren
/// („A & B“, „A, B“, „A x B“, „A vs. B“).
fn artist_keys(artist: &str) -> Vec<String> {
    let full = without_guests(artist);
    let mut keys = vec![simple(full)];
    let lower = full.to_lowercase();
    if let Some(cut) = [" & ", ", ", " x ", " vs. ", " vs ", " und ", " and ", " / "].iter().filter_map(|f| lower.find(f)).min()
        && full.is_char_boundary(cut)
    {
        keys.push(simple(&full[..cut]));
    }
    keys.retain(|k| !k.is_empty());
    keys.dedup();
    keys
}

/// Schlüssel eines Titels: Interpret ohne Gäste und Titel ohne Zusätze.
fn track_key(meta: &TrackMeta) -> Option<String> {
    let artist = artist_keys(&meta.artist).into_iter().next()?;
    let title = title_key(&meta.title);
    (!title.is_empty()).then(|| format!("{artist}|{title}"))
}

impl Popularity {
    fn from_json(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!("unbekanntes Format „{}“", file.format));
        }
        // Version 1 war je Interpret gemerkt: verwerfen
        let tracks = if file.version < VERSION { HashMap::new() } else { file.tracks };
        let mut p = Self { tracks, ..Default::default() };
        p.index_albums();
        Ok(p)
    }

    pub fn load() -> Self {
        let Some(p) = config_dir().map(|d| d.join(FILE)) else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&p) else { return Self::default() };
        Self::from_json(&text).unwrap_or_else(|e| {
            log::warn!("Beliebtheit in {} unlesbar, fange neu an: {e}", p.display());
            Self::default()
        })
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(p) = config_dir().map(|d| d.join(FILE)) else { return };
        let file = File { format: FORMAT.into(), version: VERSION, tracks: self.tracks.clone() };
        match serde_json::to_string(&file).map_err(|e| e.to_string()).and_then(|json| write_atomic(&p, &json).map_err(|e| e.to_string())) {
            Ok(()) => self.dirty = false,
            Err(e) => log::warn!("Beliebtheit nicht gespeichert ({}): {e}", p.display()),
        }
    }

    fn index_albums(&mut self) {
        self.albums.clear();
        self.dirs.clear();
        for t in self.tracks.values() {
            let n = t.listens.unwrap_or(0);
            if !t.album.is_empty() {
                self.albums.entry(t.album.clone()).or_default().push(n);
            }
            if !t.dir.is_empty() {
                self.dirs.entry(t.dir.clone()).or_default().push(n);
            }
        }
        for v in self.albums.values_mut().chain(self.dirs.values_mut()) {
            v.sort_unstable_by(|a, b| b.cmp(a));
        }
    }

    /// Beliebtheit des Titels unter `path` nach seinen Tags.
    pub fn of(&self, path: &Path, meta: &TrackMeta) -> Pop {
        let Some(t) = track_key(meta).and_then(|k| self.tracks.get(&k)) else { return Pop::Unknown };
        let Some(listens) = t.listens else { return Pop::NotFound };
        let album = self.albums.get(&simple(&meta.album)).filter(|g| !meta.album.trim().is_empty() && g.len() > 1);
        let folder = || self.dirs.get(&dir_key(path)).filter(|g| g.len() > 1);
        let (group, by_folder) = match album {
            Some(g) => (Some(g), false),
            None => (folder(), true),
        };
        match group {
            Some(g) => {
                let rank = g.iter().position(|n| *n <= listens).unwrap_or(g.len()) as u16 + 1;
                Pop::Found { listens, max: g[0], rank, group_size: g.len() as u16, by_folder }
            }
            None => Pop::Found { listens, max: listens, rank: 1, group_size: 0, by_folder: false },
        }
    }

    pub fn progress(&self) -> Option<Progress> {
        self.job.as_ref().map(|j| (j.done, j.total))
    }

    pub fn stop(&mut self) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Fragt diese Titel ab (schon gemerkte, nicht zu alte überspringt sie; ein anderes Album wird nur
    /// umgetragen). Liefert die Zahl der Titel, die abgefragt werden. `repaint` weckt die Oberfläche.
    pub fn request<'a>(&mut self, tracks: impl IntoIterator<Item = (&'a Path, &'a TrackMeta)>, repaint: impl Fn() + Send + 'static) -> usize {
        let now = now_secs();
        let mut todo: Vec<(String, String, String, (String, String))> = Vec::new();
        for (path, m) in tracks {
            let Some(key) = track_key(m) else { continue };
            let album = simple(&m.album);
            let dir = dir_key(path);
            if let Some(t) = self.tracks.get_mut(&key)
                && now.saturating_sub(t.at) < REFRESH_DAYS * DAY_SECS
            {
                if (t.album.as_str(), t.dir.as_str()) != (album.as_str(), dir.as_str()) {
                    (t.album, t.dir) = (album, dir);
                    self.dirty = true;
                }
                continue;
            }
            if todo.iter().any(|(k, ..)| *k == key) {
                continue;
            }
            let artist = without_guests(&m.artist).to_owned();
            todo.push((key, artist, m.title.clone(), (album, dir)));
        }
        if self.dirty {
            self.index_albums();
        }
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if todo.is_empty() {
            return 0;
        }
        let total = todo.len();
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let spawned = std::thread::Builder::new().name("fw-popularity".into()).spawn(move || {
            for (key, artist, title, group) in todo {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let track = fetch_track(&artist, &title, group, &stop);
                if stop.load(Ordering::Relaxed) || tx.send((key, track)).is_err() {
                    return;
                }
                repaint();
            }
        });
        if let Err(e) = spawned {
            log::warn!("Abfrage der Beliebtheit startet nicht: {e}");
            return 0;
        }
        self.job = Some(Job { rx, cancel, done: 0, total });
        total
    }

    /// Ergebnisse der laufenden Abfrage übernehmen. Liefert (etwas geändert, gerade fertig geworden).
    pub fn poll(&mut self) -> (bool, bool) {
        let Some(job) = &mut self.job else { return (false, false) };
        let mut changed = false;
        let finished = loop {
            match job.rx.try_recv() {
                Ok((key, t)) => {
                    job.done += 1;
                    // Ein Netzfehler (`at == 0`) soll gemerkte Daten nicht überschreiben
                    if t.at == 0 {
                        continue;
                    }
                    self.tracks.insert(key, t);
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        if finished {
            self.job = None;
        }
        if changed {
            self.dirty = true;
            self.index_albums();
        }
        (changed, finished)
    }
}

/// Holt die Wiedergaben eines Titels. Bei Netzfehlern kommt ein Eintrag mit `at == 0` zurück (wird nicht
/// gemerkt).
fn fetch_track(artist: &str, title: &str, (album, dir): (String, String), stop: &AtomicBool) -> Track {
    let failed = Track::default();
    let clean = |s: &str| s.replace(['"', '\\'], " ");
    let base = without_guests(title);
    let base = base.split(['(', '[']).next().unwrap_or(base).trim();
    let query = crate::online_tags::url_encode(&format!("recording:\"{}\" AND artist:\"{}\"", clean(base), clean(artist)));
    let url = format!("https://musicbrainz.org/ws/2/recording?query={query}&limit={MAX_CANDIDATES}&fmt=json");
    let body = match crate::online_tags::mb_get(&url) {
        Ok(b) => b,
        Err(e) => {
            log::info!("Beliebtheit: „{artist} – {title}“ bei MusicBrainz nicht abgefragt: {e}");
            return failed;
        }
    };
    let ids = candidates(&body, title);
    let found = Track { at: now_secs(), listens: None, album, dir };
    if ids.is_empty() || stop.load(Ordering::Relaxed) {
        return found;
    }
    let request = serde_json::json!({ "recording_mbids": ids }).to_string();
    match lb_post("https://api.listenbrainz.org/1/popularity/recording", &request) {
        Ok(body) => Track { listens: most_listens(&body), ..found },
        Err(e) => {
            log::info!("Beliebtheit: „{artist} – {title}“ bei ListenBrainz nicht abgefragt: {e}");
            failed
        }
    }
}

/// Kennungen der Aufnahmen aus der MusicBrainz-Suche, deren Titel zum gesuchten passt.
fn candidates(body: &[u8], title: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else { return Vec::new() };
    let want = title_key(title);
    v["recordings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["score"].as_i64().unwrap_or(0) >= MIN_SCORE)
        .filter(|r| r["title"].as_str().is_some_and(|t| title_key(t) == want))
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .take(MAX_CANDIDATES)
        .collect()
}

/// Höchste Zahl der Wiedergaben aus der Antwort von ListenBrainz (`None` = keine).
fn most_listens(body: &[u8]) -> Option<u64> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v.as_array()?.iter().filter_map(|r| r["total_listen_count"].as_u64()).filter(|n| *n > 0).max()
}

/// POST bei ListenBrainz, höchstens einmal je Sekunde; bei 429 (zu viele) kurz warten und erneut.
fn lb_post(url: &str, json: &str) -> Result<Vec<u8>, String> {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    for attempt in 0..3u64 {
        {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = *last {
                std::thread::sleep(LB_GAP.saturating_sub(t.elapsed()) + Duration::from_secs(attempt * 5));
            }
            *last = Some(Instant::now());
        }
        let mut cmd = crate::online_tags::curl();
        cmd.args(["-H", "Content-Type: application/json", "--data-binary", json, url]);
        let (status, body) = crate::online_tags::run(&mut cmd)?;
        match status {
            200 => return Ok(body),
            429 => continue,
            s => return Err(format!("{s} {url}")),
        }
    }
    Err("ListenBrainz: zu viele Abfragen".into())
}

/// Für die Anzeige: Wiedergaben kurz, z. B. „2,3 Mio.“ oder „48 Tsd.“.
pub fn fmt_listens(n: u64) -> String {
    let de = crate::i18n::lang() == crate::i18n::Lang::De;
    let short = |v: f64, unit_de: &str, unit_en: &str| {
        let s = format!("{v:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s).to_owned();
        if de { format!("{} {unit_de}", s.replace('.', ",")) } else { format!("{s}{unit_en}") }
    };
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => short(n as f64 / 1e3, "Tsd.", "k"),
        _ => short(n as f64 / 1e6, "Mio.", "M"),
    }
}

/// Pfade ohne gelesene Tags fallen bei der Abfrage weg; diese Hilfe sammelt Pfad und Tags der Titel.
pub fn metas_of<'a>(paths: &'a [PathBuf], metas: &'a HashMap<PathBuf, TrackMeta>) -> impl Iterator<Item = (&'a Path, &'a TrackMeta)> {
    paths.iter().filter_map(|p| Some((p.as_path(), metas.get(p)?)))
}

/// Ordner einer Datei als Schlüssel der Vergleichsgruppe.
fn dir_key(path: &Path) -> String {
    path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_versions_and_guests() {
        assert_eq!(title_key("Don’t Stop Me Now - Remastered 2011"), "don t stop me now");
        assert_eq!(title_key("Don't Stop Me Now (Radio Edit)"), "don t stop me now");
        assert_eq!(title_key("Easy feat. Bausa [Extended]"), "easy");
        assert_eq!(title_key("Über den Wolken"), "uber den wolken");
    }

    #[test]
    fn several_artists_use_the_first() {
        assert_eq!(artist_keys("Simon & Garfunkel"), vec!["simon garfunkel", "simon"]);
        assert_eq!(artist_keys("Cro feat. Bausa"), vec!["cro"]);
        assert_eq!(artist_keys(""), Vec::<String>::new());
    }

    #[test]
    fn candidates_need_the_same_title_and_a_good_score() {
        let body = br#"{"recordings":[{"id":"a","title":"Need You Tonight","score":100},
            {"id":"b","title":"Need You Tonight (Remix)","score":95},{"id":"c","title":"Mediate","score":90},
            {"id":"d","title":"Need You Tonight","score":40}]}"#;
        assert_eq!(candidates(body, "Need you tonight"), vec!["a", "b"]);
        assert_eq!(most_listens(br#"[{"total_listen_count":5},{"total_listen_count":null},{"total_listen_count":498651}]"#), Some(498651));
        assert_eq!(most_listens(br#"[{"total_listen_count":null}]"#), None);
    }

    #[test]
    fn ranks_within_the_album() {
        let mut p = Popularity::default();
        let m = |artist: &str, title: &str| TrackMeta { artist: artist.into(), title: title.into(), album: "Fetenhits 1".into(), ..Default::default() };
        for (a, t, n) in [("INXS", "Need You Tonight", Some(500)), ("U2", "Sunday Bloody Sunday", Some(1000)), ("Toto", "Africa", Some(800)), ("X", "Y", Some(10)), ("Z", "W", None)] {
            p.tracks.insert(track_key(&m(a, t)).unwrap(), Track { at: 1, listens: n, album: "fetenhits 1".into(), dir: "/m/fete".into() });
        }
        p.index_albums();
        let at = Path::new("/m/fete/x.mp3");
        let inxs = p.of(at, &m("INXS", "Need you tonight"));
        assert_eq!(inxs, Pop::Found { listens: 500, max: 1000, rank: 3, group_size: 5, by_folder: false });
        assert!(inxs.is_hit());
        assert_eq!(inxs.share(), Some(0.5));
        assert!(!p.of(at, &m("X", "Y")).is_hit());
        assert_eq!(p.of(at, &m("Z", "W")), Pop::NotFound);
        assert_eq!(p.of(at, &m("ABBA", "SOS")), Pop::Unknown);
        // Album allein (gemischter Ordner): der Ordner ist die Gruppe
        let mixed = TrackMeta { album: "War".into(), ..m("U2", "Sunday Bloody Sunday") };
        assert_eq!(p.of(at, &mixed), Pop::Found { listens: 1000, max: 1000, rank: 1, group_size: 5, by_folder: true });
        // Weder Album noch Ordner mit Partnern: logarithmische Skala, kein Hit
        let alone = p.of(Path::new("/anders/x.mp3"), &mixed);
        assert!(matches!(alone, Pop::Found { group_size: 0, .. }));
        assert!(!alone.is_hit());
    }

    #[test]
    fn listens_are_short() {
        assert_eq!(fmt_listens(48), "48");
    }
}
