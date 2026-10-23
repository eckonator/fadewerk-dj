// SPDX-License-Identifier: GPL-3.0-or-later
//! Tags eines ganzen Ordners online ergänzen: erst erkennen und einstufen (sicher, unsicher, nicht gefunden),
//! dann auf Klick schreiben. Ergänzt werden nur leere oder offensichtlich falsche Felder; vorher merkt sich ein
//! Journal die alten Werte, damit der ganze Durchlauf rückgängig gemacht werden kann.
//!
//! Erkennung über den Fingerabdruck (`online_tags::identify_by_sound`, drei Threads, AcoustID gebremst),
//! Genre und Cover in einem eigenen Thread mit Merkliste je Album bzw. Interpret.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{self, Align2, Frame, Margin, RichText};
use fw_library::{CoverChange, TagFields, has_cover, read_tags, write_tags_ext};
use serde::{Deserialize, Serialize};

use crate::i18n::{tr, trf};
use crate::online_tags::{self, Candidate, FIELDS, Field};
use crate::{fonts, theme};

/// Gleichzeitige Fingerabdrücke (Dekodieren kostet Rechenzeit, AcoustID bremst ohnehin auf 3/s).
const THREADS: usize = 3;
/// Ab dieser Trefferquote kann ein Treffer „sicher“ sein.
const SURE_SCORE: f32 = 0.9;
/// Ein anderer Titel so nah an der besten Quote macht den Treffer unsicher.
const RIVAL_GAP: f32 = 0.05;
/// Erlaubte Abweichung der Länge in Sekunden.
const LENGTH_TOLERANCE: f64 = 5.0;

// ---------------------------------------------------------------- Einstufen

/// Eine geplante Änderung eines Feldes.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub field: Field,
    pub old: String,
    pub new: String,
}

/// Ergebnis der Einstufung eines erkannten Titels.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub sure: bool,
    /// Warum unsicher (deutscher Text für `tr`), leer wenn sicher.
    pub doubts: Vec<&'static str>,
    /// Gewählter Vorschlag (nach `rank_by_hint`) und Album, falls eins zum Titel passt bzw. gewählt wurde.
    pub candidate: Candidate,
    pub album: Option<usize>,
    pub changes: Vec<Change>,
    pub need_genre: bool,
    pub need_cover: bool,
}

/// Kleinbuchstaben, nur Buchstaben und Ziffern.
fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Kern eines Titels ohne Zusätze in Klammern und ohne „feat. …“.
fn core(s: &str) -> String {
    let s = s.split(['(', '[']).next().unwrap_or(s);
    let lower = s.to_lowercase();
    let cut = [" feat. ", " feat ", " ft. ", " featuring "].iter().filter_map(|f| lower.find(f)).min().unwrap_or(lower.len());
    norm(&lower[..cut])
}

/// Ist der Wert leer oder offensichtlich kein echter Wert (Platzhalter, Dateiname als Titel …)?
pub fn is_bad(field: Field, value: &str, stem: &str) -> bool {
    let v = value.trim();
    let l = v.to_lowercase();
    if v.is_empty() || ["unknown", "unbekannt", "<unknown>", "n/a", "-", "?", "none"].contains(&l.as_str()) {
        return true;
    }
    match field {
        Field::Title => {
            let numbered = ["track", "titel", "audiotrack", "piste", "spur"].iter().any(|p| {
                l.strip_prefix(p).is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
            });
            numbered || l == stem.trim().to_lowercase() || ["untitled", "ohne titel"].contains(&l.as_str())
        }
        Field::Artist => ["unknown artist", "unbekannter interpret", "unbekannter künstler", "various artists", "various", "va"].contains(&l.as_str()),
        Field::Album => ["unknown album", "unbekanntes album", "untitled"].contains(&l.as_str()),
        Field::Year => !(v.len() >= 4 && v[..4].chars().all(|c| c.is_ascii_digit()) && matches!(&v[..1], "1" | "2")),
        Field::Track => v.split('/').next().and_then(|n| n.trim().parse::<u32>().ok()).is_none_or(|n| n == 0),
        Field::Genre => ["other", "(255)", "genre", "sonstige"].contains(&l.as_str()),
        Field::AlbumArtist | Field::Cover => false,
    }
}

/// Stuft die Vorschläge zu einer Datei ein und plant die Änderungen. `None` = nichts gefunden.
pub fn plan(fields: &TagFields, stem: &str, file_has_cover: bool, duration: f64, found: &[Candidate]) -> Option<Plan> {
    let title_ok = !is_bad(Field::Title, &fields.title, stem);
    let artist_ok = !is_bad(Field::Artist, &fields.artist, stem);
    let mut list = found.to_vec();
    if title_ok {
        online_tags::rank_by_hint(&mut list, &fields.artist, &fields.title);
    } else {
        // Ohne brauchbaren Titel hilft vielleicht der Dateiname („Interpret - Titel“, „09_Interpret_Titel“)
        online_tags::rank_by_hint(&mut list, "", stem);
    }
    let top = list.first()?.clone();
    let stem_n = norm(stem);
    let artist_fits = |a: &str| {
        let (x, y) = (norm(a), norm(&top.artist));
        !x.is_empty() && !y.is_empty() && (x.contains(&y) || y.contains(&x))
    };
    // Titel gleich, oder einer steckt im anderen („Anspieltipp - Wir trinken gern“)
    let title_fits = |a: &str| {
        let (x, y) = (core(a), core(&top.title));
        x == y || (x.len().min(y.len()) >= 4 && (x.contains(&y) || y.contains(&x)))
    };
    let hint_matches = if title_ok {
        title_fits(&fields.title) && (!artist_ok || artist_fits(&fields.artist))
    } else {
        let t = core(&top.title);
        !t.is_empty() && stem_n.contains(&t) && (norm(&top.artist).is_empty() || stem_n.contains(&norm(&top.artist)))
    };
    let rival = list.iter().skip(1).any(|c| c.score >= top.score - RIVAL_GAP && core(&c.title) != core(&top.title));
    let length_ok = duration <= 0.0 || top.durations.is_empty() || top.durations.iter().any(|d| (d - duration).abs() <= LENGTH_TOLERANCE);
    // Mit Tags (oder passendem Dateinamen) müssen sie zum Treffer passen; ganz ohne Hinweis entscheidet
    // der Klang, solange kein anderer Titel genauso gut passt.
    let has_hint = title_ok || !stem_n.is_empty() && hint_matches;
    let mut doubts = Vec::new();
    if top.score < SURE_SCORE {
        doubts.push("Trefferquote unter 90 %");
    }
    if !length_ok {
        doubts.push("Länge weicht ab");
    }
    if has_hint && !hint_matches {
        doubts.push(if title_ok { "Tags nennen etwas anderes" } else { "Dateiname nennt etwas anderes" });
    }
    if !has_hint && rival {
        doubts.push("anderer Titel genauso wahrscheinlich");
    }
    let sure = doubts.is_empty();

    // Album: ein vorhandenes bleibt; passt es zu einem Album des Treffers, kommen Titelnummer und
    // Album-Interpret von dort. Fehlt es, gilt das beste Album.
    let album = if is_bad(Field::Album, &fields.album, stem) {
        (!top.albums.is_empty()).then_some(0)
    } else {
        let have = norm(&fields.album);
        top.albums.iter().position(|a| {
            let n = norm(&a.title);
            n == have || (n.len() >= 4 && have.len() >= 4 && (n.contains(&have) || have.contains(&n)))
        })
    };
    let chosen = album.and_then(|i| top.albums.get(i));
    let mut changes = Vec::new();
    for field in FIELDS {
        let new = match field {
            Field::Title => top.title.clone(),
            Field::Artist => top.artist.clone(),
            Field::Year => top.year.clone(),
            Field::Album => chosen.map(|a| a.title.clone()).unwrap_or_default(),
            Field::AlbumArtist => chosen.map(|a| a.album_artist.clone()).unwrap_or_default(),
            Field::Track => chosen.map(|a| a.track.clone()).unwrap_or_default(),
            Field::Genre | Field::Cover => continue,
        };
        let old = field.slot(&mut fields.clone()).cloned().unwrap_or_default();
        if !new.is_empty() && new != old && is_bad(field, &old, stem) {
            changes.push(Change { field, old, new });
        }
    }
    Some(Plan {
        sure,
        doubts,
        need_genre: is_bad(Field::Genre, &fields.genre, stem),
        // Passt das vorhandene Album zu keinem (Sampler, den die Dienste nicht kennen), kommt das Cover vom
        // besten Album des Treffers; der Album-Tag bleibt
        need_cover: !file_has_cover && !top.albums.is_empty(),
        candidate: top,
        album,
        changes,
    })
}

// ---------------------------------------------------------------- Durchlauf

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Waiting,
    Sure,
    Unsure,
    NotFound,
    /// Trägt schon eine MusicBrainz-Kennung (früher erkannt).
    Known,
    /// Nicht lesbar oder keine Antwort (deutscher Text bzw. Fehlermeldung).
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    pub state: State,
    pub artist: String,
    pub title: String,
    pub score: f32,
    pub doubts: Vec<&'static str>,
    pub recording_id: String,
    pub release_id: String,
    pub changes: Vec<Change>,
    pub cover: Option<Arc<Vec<u8>>>,
    /// Genre/Cover werden noch geladen.
    pub details_pending: bool,
    /// Schon erkannt, nur das Cover wird nachgeholt.
    pub cover_only: bool,
    /// Beim Schreiben dabei (Häkchen).
    pub write: bool,
    pub written: Option<Result<(), String>>,
}

impl Item {
    fn waiting(path: PathBuf) -> Self {
        Self {
            path,
            state: State::Waiting,
            artist: String::new(),
            title: String::new(),
            score: 0.0,
            doubts: Vec::new(),
            recording_id: String::new(),
            release_id: String::new(),
            changes: Vec::new(),
            cover: None,
            details_pending: false,
            cover_only: false,
            write: false,
            written: None,
        }
    }
}

enum Msg {
    Item(usize, Box<Item>),
    Details { index: usize, genre: Option<String>, cover: Option<Arc<Vec<u8>>> },
    Finished,
}

struct DetailJob {
    index: usize,
    group_id: String,
    release_id: String,
    artist_id: Option<String>,
    genre: bool,
    cover: bool,
}

/// Startet die Erkennung aller Dateien im Hintergrund.
fn start(paths: Vec<PathBuf>, stop: Arc<AtomicBool>, ctx: egui::Context) -> Receiver<Msg> {
    let (tx, rx) = mpsc::channel();
    let (detail_tx, detail_rx) = mpsc::channel::<DetailJob>();
    let paths = Arc::new(paths);
    let next = Arc::new(AtomicUsize::new(0));
    for n in 0..THREADS {
        let (paths, next, stop, tx, detail_tx, ctx) = (paths.clone(), next.clone(), stop.clone(), tx.clone(), detail_tx.clone(), ctx.clone());
        std::thread::Builder::new()
            .name(format!("fw-tag-batch-{n}"))
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let (item, job) = identify(i, path);
                    if let Some(job) = job {
                        let _ = detail_tx.send(job);
                    }
                    if tx.send(Msg::Item(i, Box::new(item))).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok();
    }
    drop(detail_tx);
    std::thread::Builder::new()
        .name("fw-tag-batch-details".into())
        .spawn(move || {
            let mut covers: HashMap<String, Option<Arc<Vec<u8>>>> = HashMap::new();
            // Endet, wenn alle Erkennungs-Threads fertig sind
            while let Ok(job) = detail_rx.recv() {
                if stop.load(Ordering::Relaxed) {
                    continue;
                }
                let genre = if job.genre { online_tags::fetch_genre(&job.group_id, job.artist_id.as_deref()) } else { None };
                let cover = if job.cover {
                    covers.entry(format!("{}/{}", job.group_id, job.release_id)).or_insert_with(|| online_tags::fetch_cover(&job.group_id, &job.release_id).map(Arc::new)).clone()
                } else {
                    None
                };
                let _ = tx.send(Msg::Details { index: job.index, genre, cover });
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::Finished);
            ctx.request_repaint();
        })
        .ok();
    rx
}

/// Erkennt eine Datei und stuft sie ein; liefert ggf. den Auftrag für Genre und Cover.
fn identify(index: usize, path: &Path) -> (Item, Option<DetailJob>) {
    let mut item = Item::waiting(path.to_owned());
    let (fields, info) = match read_tags(path) {
        Ok(t) => t,
        Err(e) => {
            item.state = State::Failed(format!("{e:#}"));
            return (item, None);
        }
    };
    if !fields.mb_recording_id.is_empty() {
        // Schon erkannt. Fehlt (noch) ein brauchbares Cover, wird es über die gespeicherte Albumkennung
        // nachgeholt; ohne die geht es über die Erkennung unten.
        if has_cover(path) {
            item.state = State::Known;
            return (item, None);
        }
    }
    if !fields.mb_recording_id.is_empty() && !fields.mb_release_id.is_empty() {
        item.state = State::Sure;
        item.write = true;
        item.artist = fields.artist.clone();
        item.title = fields.title.clone();
        item.score = 1.0;
        item.recording_id = fields.mb_recording_id.clone();
        item.release_id = fields.mb_release_id.clone();
        item.details_pending = true;
        item.cover_only = true;
        let job = DetailJob { index, group_id: String::new(), release_id: fields.mb_release_id.clone(), artist_id: None, genre: false, cover: true };
        return (item, Some(job));
    }
    let found = match online_tags::identify_by_sound(path) {
        Ok(f) => f,
        Err(e) => {
            log::info!("Ordner-Erkennung {}: {e}", path.display());
            item.state = State::Failed(e);
            return (item, None);
        }
    };
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(p) = plan(&fields, &stem, has_cover(path), info.duration_secs, &found) else {
        item.state = State::NotFound;
        return (item, None);
    };
    let c = &p.candidate;
    let album = p.album.and_then(|i| c.albums.get(i)).or(c.albums.first());
    item.state = if p.sure { State::Sure } else { State::Unsure };
    item.write = p.sure;
    item.artist = c.artist.clone();
    item.title = c.title.clone();
    item.score = c.score;
    item.doubts = p.doubts.clone();
    if !p.sure {
        log::info!("Unsicher {}: {:?} (Tags „{} – {}“, Treffer „{} – {}“, {:.1} s / {:?} s)", path.display(), p.doubts, fields.artist, fields.title, c.artist, c.title, info.duration_secs, c.durations);
    }
    item.recording_id = c.recording_id.clone();
    // Albumkennung nur, wenn das Album im Tag dazu passt (das Cover kann von einem anderen kommen)
    item.release_id = p.album.and_then(|i| c.albums.get(i)).map(|a| a.release_id.clone()).unwrap_or_default();
    item.changes = p.changes;
    let job = (p.need_genre || p.need_cover).then(|| DetailJob {
        index,
        group_id: album.map(|a| a.group_id.clone()).unwrap_or_default(),
        release_id: album.map(|a| a.release_id.clone()).unwrap_or_default(),
        artist_id: c.artist_ids.first().cloned(),
        genre: p.need_genre,
        cover: p.need_cover,
    });
    item.details_pending = job.is_some();
    if p.need_genre {
        // Platz für das Genre merken; der Wert kommt mit den Details
        item.changes.push(Change { field: Field::Genre, old: fields.genre.clone(), new: String::new() });
    }
    (item, job)
}

// ---------------------------------------------------------------- Journal und Schreiben

const JOURNAL_FORMAT: &str = "FadeWerk-DJ-Tag-Journal";

#[derive(Serialize, Deserialize, Default)]
struct Journal {
    format: String,
    version: u32,
    created: String,
    entries: Vec<JournalEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct JournalEntry {
    path: PathBuf,
    /// Feldschlüssel (`Field::key`), alter und neuer Wert
    changes: Vec<(String, String, String)>,
    cover_added: bool,
}

fn journal_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("tag-journal"))
}

/// Neuestes Journal, das noch nicht rückgängig gemacht wurde: Pfad, Datum, Zahl der Titel.
fn latest_journal() -> Option<(PathBuf, String, usize)> {
    latest_journal_in(&journal_dir()?)
}

fn latest_journal_in(dir: &Path) -> Option<(PathBuf, String, usize)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json") && !p.to_string_lossy().ends_with(".rueckgaengig.json"))
        .collect();
    files.sort();
    let path = files.pop()?;
    let j: Journal = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    (j.format == JOURNAL_FORMAT && !j.entries.is_empty()).then_some((path, j.created, j.entries.len()))
}

enum WriteMsg {
    Done(usize, Result<(), String>),
    Finished(Vec<PathBuf>),
}

struct WriteJob {
    index: usize,
    path: PathBuf,
    changes: Vec<Change>,
    cover: Option<Arc<Vec<u8>>>,
    recording_id: String,
    release_id: String,
}

/// Wendet eine Änderung an, wenn das Feld noch so ist wie bei der Erkennung (der Nutzer könnte es
/// inzwischen selbst geändert haben).
fn apply(fields: &mut TagFields, field: Field, old: &str, new: &str) -> bool {
    match field.slot(fields) {
        Some(slot) if slot.trim() == old.trim() && slot != new => {
            *slot = new.to_owned();
            true
        }
        _ => false,
    }
}

fn write_all(jobs: Vec<WriteJob>, dir: Option<PathBuf>, ctx: egui::Context) -> Receiver<WriteMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-tag-batch-write".into())
        .spawn(move || {
            let now = jiff::Zoned::now();
            let mut journal = Journal { format: JOURNAL_FORMAT.into(), version: 1, created: now.strftime("%Y-%m-%d %H:%M").to_string(), entries: Vec::new() };
            let journal_path = dir.as_ref().map(|d| d.join(format!("{}.json", now.strftime("%Y-%m-%d_%H%M%S"))));
            if let Some(d) = &dir {
                let _ = std::fs::create_dir_all(d);
            }
            let save = |j: &Journal| {
                if let Some(p) = &journal_path
                    && let Err(e) = crate::settings::write_atomic(p, &serde_json::to_string_pretty(j).unwrap_or_default())
                {
                    log::warn!("Tag-Journal {}: {e}", p.display());
                }
            };
            let mut written = Vec::new();
            for job in jobs {
                let result = (|| -> Result<(), String> {
                    let (mut fields, _) = read_tags(&job.path).map_err(|e| format!("{e:#}"))?;
                    let mut entry = JournalEntry { path: job.path.clone(), changes: Vec::new(), cover_added: false };
                    for c in &job.changes {
                        if !c.new.is_empty() && apply(&mut fields, c.field, &c.old, &c.new) {
                            entry.changes.push((c.field.key().into(), c.old.clone(), c.new.clone()));
                        }
                    }
                    for (slot, new, key) in [(&mut fields.mb_recording_id, &job.recording_id, "mb_recording_id"), (&mut fields.mb_release_id, &job.release_id, "mb_release_id")] {
                        if slot.is_empty() && !new.is_empty() {
                            entry.changes.push((key.into(), String::new(), new.clone()));
                            *slot = new.clone();
                        }
                    }
                    let cover = job.cover.as_ref().filter(|_| !has_cover(&job.path));
                    entry.cover_added = cover.is_some();
                    if entry.changes.is_empty() && !entry.cover_added {
                        return Ok(());
                    }
                    // Erst ins Journal, dann in die Datei: so lässt sich auch ein abgebrochener Lauf zurücknehmen
                    journal.entries.push(entry);
                    save(&journal);
                    write_tags_ext(&job.path, &fields, cover.map_or(CoverChange::Keep, |c| CoverChange::Set(c))).map_err(|e| format!("{e:#}"))?;
                    written.push(job.path.clone());
                    Ok(())
                })();
                let _ = tx.send(WriteMsg::Done(job.index, result));
                ctx.request_repaint();
            }
            let _ = tx.send(WriteMsg::Finished(written));
            ctx.request_repaint();
        })
        .ok();
    rx
}

/// Nimmt einen Durchlauf zurück: Felder, die noch den geschriebenen Wert haben, bekommen den alten zurück,
/// ergänzte Cover werden entfernt. Das Journal wird danach umbenannt (nicht noch einmal anbieten).
fn undo(journal_path: PathBuf, ctx: egui::Context) -> Receiver<WriteMsg> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-tag-batch-undo".into())
        .spawn(move || {
            let journal: Journal = std::fs::read_to_string(&journal_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let mut restored = Vec::new();
            for (i, e) in journal.entries.iter().enumerate() {
                let result = (|| -> Result<(), String> {
                    let (mut fields, _) = read_tags(&e.path).map_err(|e| format!("{e:#}"))?;
                    for (key, old, new) in &e.changes {
                        match key.as_str() {
                            "mb_recording_id" if fields.mb_recording_id == *new => fields.mb_recording_id = old.clone(),
                            "mb_release_id" if fields.mb_release_id == *new => fields.mb_release_id = old.clone(),
                            k => {
                                if let Some(f) = Field::from_key(k) {
                                    apply(&mut fields, f, new, old);
                                }
                            }
                        }
                    }
                    let cover = if e.cover_added { CoverChange::Remove } else { CoverChange::Keep };
                    write_tags_ext(&e.path, &fields, cover).map_err(|e| format!("{e:#}"))?;
                    restored.push(e.path.clone());
                    Ok(())
                })();
                if let Err(err) = &result {
                    log::warn!("Rückgängig {}: {err}", e.path.display());
                }
                let _ = tx.send(WriteMsg::Done(i, result));
            }
            let done = journal_path.with_extension("rueckgaengig.json");
            let _ = std::fs::rename(&journal_path, done);
            let _ = tx.send(WriteMsg::Finished(restored));
            ctx.request_repaint();
        })
        .ok();
    rx
}

// ---------------------------------------------------------------- Fenster

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Sure,
    Unsure,
    NotFound,
    Skipped,
}

impl View {
    fn holds(self, s: &State) -> bool {
        match self {
            View::Sure => *s == State::Sure,
            View::Unsure => *s == State::Unsure,
            View::NotFound => *s == State::NotFound,
            View::Skipped => matches!(s, State::Known | State::Failed(_)),
        }
    }
}

pub enum BatchEvent {
    Close,
    AllowOnline,
    /// Unsicheren Titel im Tag-Fenster prüfen.
    Review(PathBuf),
    /// Diese Dateien wurden geändert (Tags neu einlesen).
    Changed(Vec<PathBuf>),
    /// Die Erkennung im Hintergrund ist fertig: (sicher, unsicher).
    BackgroundDone(usize, usize),
}

/// Stand für die Browser-Kopfzeile, solange das Fenster im Hintergrund ist.
#[derive(Clone, Copy)]
pub struct BatchStatus {
    pub done: usize,
    pub total: usize,
    pub rest_secs: Option<f64>,
    pub finished: bool,
}

/// „noch ca. … s/min“ für die geschätzte Restzeit.
pub fn rest_text(rest: f64) -> String {
    if rest < 90.0 { trf("noch ca. {} s", &[&(rest.round().max(1.0) as u64)]) } else { trf("noch ca. {} Min.", &[&((rest / 60.0).round() as u64)]) }
}

pub struct TagBatch {
    paths: Vec<PathBuf>,
    items: Vec<Item>,
    asking: bool,
    rx: Option<Receiver<Msg>>,
    stop: Arc<AtomicBool>,
    started: Option<Instant>,
    finished: bool,
    view: View,
    writing: Option<(Receiver<WriteMsg>, bool)>,
    report: Option<String>,
    journal: Option<(PathBuf, String, usize)>,
    /// Fenster ausgeblendet, die Erkennung läuft weiter (Stand in der Browser-Kopfzeile).
    background: bool,
    /// Hinweis „fertig“ für den Hintergrund-Durchlauf schon gegeben.
    announced: bool,
    /// Fenster beim nächsten Zeichnen mittig setzen (danach frei verschiebbar).
    place: bool,
}

impl TagBatch {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            items: paths.iter().cloned().map(Item::waiting).collect(),
            paths,
            asking: true,
            rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            started: None,
            finished: false,
            view: View::Sure,
            writing: None,
            report: None,
            journal: latest_journal(),
            background: false,
            announced: false,
            place: true,
        }
    }

    pub fn status(&self) -> Option<BatchStatus> {
        self.background.then(|| BatchStatus {
            done: self.done(),
            total: self.items.len(),
            rest_secs: self.rest_secs(),
            finished: self.rx.is_none(),
        })
    }

    /// Fenster ausblenden, die Erkennung läuft weiter.
    pub fn send_to_background(&mut self) {
        self.background = true;
        self.announced = false;
    }

    /// Fenster wieder zeigen.
    pub fn bring_back(&mut self) {
        if self.background {
            self.background = false;
            self.place = true;
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn done(&self) -> usize {
        self.items.iter().filter(|i| i.state != State::Waiting).count()
    }

    /// Geschätzte Restzeit der Erkennung (ab 5 Titeln, ohne die Genre/Cover-Phase).
    fn rest_secs(&self) -> Option<f64> {
        let (done, total) = (self.done(), self.items.len());
        let t = self.started?;
        (self.rx.is_some() && done >= 5 && done < total).then(|| t.elapsed().as_secs_f64() / done as f64 * (total - done) as f64)
    }

    fn begin(&mut self, ctx: &egui::Context) {
        self.asking = false;
        self.started = Some(Instant::now());
        self.rx = Some(start(self.paths.clone(), self.stop.clone(), ctx.clone()));
    }

    /// Das Tag-Fenster hat diese Datei gespeichert: war sie zur Prüfung offen, gilt sie als erledigt.
    pub fn saved(&mut self, path: &Path) {
        if let Some(item) = self.items.iter_mut().find(|i| i.path == path && i.state == State::Unsure) {
            item.write = false;
            item.written = Some(Ok(()));
        }
    }

    fn poll(&mut self) -> Option<BatchEvent> {
        if let Some(rx) = &self.rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Item(i, item) => self.items[i] = *item,
                    Msg::Details { index, genre, cover } => {
                        let item = &mut self.items[index];
                        item.details_pending = false;
                        if item.cover_only && cover.is_none() {
                            item.state = State::Known;
                            item.write = false;
                        }
                        item.cover = cover;
                        match genre {
                            Some(g) => {
                                if let Some(c) = item.changes.iter_mut().find(|c| c.field == Field::Genre) {
                                    c.new = g;
                                }
                            }
                            None => item.changes.retain(|c| c.field != Field::Genre),
                        }
                    }
                    Msg::Finished => {
                        self.finished = true;
                        self.rx = None;
                        // Was beim Stoppen noch wartete, gilt als nicht geprüft
                        for item in &mut self.items {
                            item.details_pending = false;
                            item.changes.retain(|c| c.field != Field::Genre || !c.new.is_empty());
                        }
                        break;
                    }
                }
            }
        }
        let (rx, is_undo) = self.writing.as_ref()?;
        let is_undo = *is_undo;
        let mut changed = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                WriteMsg::Done(i, r) => {
                    if !is_undo && let Some(item) = self.items.get_mut(i) {
                        item.written = Some(r);
                    }
                }
                WriteMsg::Finished(paths) => {
                    self.report = Some(if is_undo {
                        trf("{} Titel zurückgesetzt.", &[&paths.len()])
                    } else {
                        let failed = self.items.iter().filter(|i| matches!(i.written, Some(Err(_)))).count();
                        if failed > 0 { trf("{} Titel geschrieben, {} Fehler.", &[&paths.len(), &failed]) } else { trf("{} Titel geschrieben.", &[&paths.len()]) }
                    });
                    self.journal = latest_journal();
                    changed = Some(BatchEvent::Changed(paths));
                }
            }
        }
        if changed.is_some() {
            self.writing = None;
        }
        changed
    }

    /// `online_allowed`: Zustimmung liegt vor. `busy`: Dateien, die gerade in einem Player spielen.
    pub fn show(&mut self, ctx: &egui::Context, online_allowed: bool, busy: &[PathBuf]) -> Option<BatchEvent> {
        if self.asking && online_allowed {
            self.begin(ctx);
        }
        let mut event = self.poll();
        if self.rx.is_some() || self.writing.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if self.background {
            if self.rx.is_none() && !self.announced {
                self.announced = true;
                let count = |s: State| self.items.iter().filter(|i| i.state == s).count();
                event = event.or(Some(BatchEvent::BackgroundDone(count(State::Sure), count(State::Unsure))));
            }
            return event;
        }
        let mut open = true;
        // Mittig geöffnet, danach verschiebbar (z. B. weg von den Playern)
        let mut window = egui::Window::new(tr("Tags online ergänzen"))
            .id(egui::Id::new("tag-batch"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_CENTER);
        if std::mem::take(&mut self.place) {
            window = window.current_pos(ctx.content_rect().center());
        }
        window
            .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(720.0);
                if let Some(e) = self.contents(ui, ctx, online_allowed, busy) {
                    event = Some(e);
                }
            });
        if !open {
            event = Some(BatchEvent::Close);
        }
        if matches!(event, Some(BatchEvent::Close)) {
            self.stop.store(true, Ordering::Relaxed);
        }
        event
    }

    fn contents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, online_allowed: bool, busy: &[PathBuf]) -> Option<BatchEvent> {
        let mut event = None;
        ui.label(RichText::new(trf("{} Titel", &[&self.paths.len()])).font(fonts::bold(15.0)));
        if self.asking {
            ui.add_space(4.0);
            ui.label(RichText::new(tr("Für jeden Titel wird ein akustischer Fingerabdruck berechnet und mit der Länge an AcoustID geschickt; Genres kommen von MusicBrainz, Cover aus dem Cover Art Archive. Die Musikdateien bleiben auf deinem Rechner. Geschrieben wird erst, wenn du es bestätigst.")).size(12.5).color(theme::dim()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = if online_allowed { tr("Erkennung starten") } else { tr("Erlauben und starten") };
                if ui.button(RichText::new(label).strong()).clicked() {
                    if !online_allowed {
                        event = Some(BatchEvent::AllowOnline);
                    }
                    self.begin(ctx);
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    event = Some(BatchEvent::Close);
                }
            });
            self.undo_row(ui, ctx);
            return event;
        }

        // Fortschritt
        let done = self.done();
        let total = self.items.len().max(1);
        ui.horizontal(|ui| {
            ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_width(360.0).desired_height(8.0));
            let mut text = format!("{done} / {}", self.items.len());
            if let Some(rest) = self.rest_secs() {
                text += &format!("  · {}", rest_text(rest));
            }
            if self.rx.is_some() && done == self.items.len() {
                text += &format!("  · {}", tr("Lädt Genres und Cover …"));
            }
            ui.label(RichText::new(text).color(theme::dim()));
            if self.rx.is_some() && ui.button(tr("Stopp")).clicked() {
                self.stop();
            }
            if self.rx.is_some()
                && self.writing.is_none()
                && ui.button(tr("Im Hintergrund")).on_hover_text(tr("Fenster schließen, die Erkennung läuft weiter; Stand und Rückkehr über die Kopfzeile des Ordnerbrowsers")).clicked()
            {
                self.send_to_background();
            }
        });
        ui.add_space(6.0);

        // Reiter
        let count = |v: View| self.items.iter().filter(|i| v.holds(&i.state)).count();
        ui.horizontal(|ui| {
            for (v, label, color) in [
                (View::Sure, "Sicher ({})", theme::play()),
                (View::Unsure, "Unsicher ({})", theme::select_accent()),
                (View::NotFound, "Nicht gefunden ({})", theme::dim()),
                (View::Skipped, "Übersprungen ({})", theme::dim()),
            ] {
                let text = RichText::new(trf(label, &[&count(v)])).color(color);
                if ui.add(egui::Button::selectable(self.view == v, text)).clicked() {
                    self.view = v;
                }
            }
        });
        ui.label(RichText::new(match self.view {
            View::Sure => tr("Treffer passt zu Länge und vorhandenen Angaben. Ergänzt werden nur leere oder offensichtlich falsche Felder."),
            View::Unsure => tr("Nicht eindeutig (z. B. Remix, B-Seite oder Angaben passen nicht). Mit „Prüfen …“ im Tag-Fenster ansehen."),
            View::NotFound => tr("Die Dienste kennen diese Aufnahmen nicht. Hier ändert sich nichts."),
            View::Skipped => tr("Schon früher erkannt (MusicBrainz-Kennung in der Datei) oder nicht lesbar."),
        }).size(12.0).color(theme::dim()));
        ui.add_space(4.0);

        // Liste
        let rows: Vec<usize> = (0..self.items.len()).filter(|&i| self.view.holds(&self.items[i].state)).collect();
        let row_h = 40.0;
        let locked = self.writing.is_some();
        egui::Frame::new().fill(theme::panel().gamma_multiply(0.7)).corner_radius(4.0).inner_margin(Margin::same(6)).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt(("tag-batch-liste", self.view as u8)).max_height(340.0).min_scrolled_height(340.0).auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
                for &i in &rows[range] {
                    if let Some(e) = self.row(ui, i, row_h, locked, busy) {
                        event = Some(e);
                    }
                }
            });
        });

        // Schreiben
        ui.add_space(6.0);
        let pending = self.items.iter().filter(|i| i.write && i.written.is_none()).count();
        let details_open = self.items.iter().any(|i| i.details_pending);
        ui.horizontal(|ui| {
            let ready = (self.finished || self.rx.is_none()) && !details_open && !locked && pending > 0;
            let tip = if self.rx.is_some() || details_open { tr("Geht, sobald die Erkennung fertig ist oder gestoppt wurde") } else { tr("Schreibt die angehakten Titel; die alten Werte merkt sich ein Journal zum Rückgängigmachen") };
            if ui.add_enabled(ready, egui::Button::new(RichText::new(trf("Angehakte schreiben ({})", &[&pending])).strong())).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                let jobs = self
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.write && i.written.is_none() && !busy.contains(&i.path))
                    .map(|(index, i)| WriteJob {
                        index,
                        path: i.path.clone(),
                        changes: i.changes.clone(),
                        cover: i.cover.clone(),
                        recording_id: i.recording_id.clone(),
                        release_id: i.release_id.clone(),
                    })
                    .collect();
                self.report = None;
                self.writing = Some((write_all(jobs, journal_dir(), ctx.clone()), false));
            }
            if locked {
                ui.add(egui::Spinner::new().size(14.0));
            }
            if let Some(r) = &self.report {
                ui.label(RichText::new(r).color(theme::dim()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Schließen")).clicked() {
                    event = Some(BatchEvent::Close);
                }
            });
        });
        self.undo_row(ui, ctx);
        event
    }

    /// Hinweis auf den letzten Durchlauf mit „Rückgängig machen“.
    fn undo_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some((path, date, n)) = self.journal.clone() else { return };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(trf("Letzter Durchlauf: {} · {} Titel", &[&date, &n])).size(12.0).color(theme::dim()));
            let free = self.writing.is_none();
            if ui.add_enabled(free, egui::Button::new(tr("Rückgängig machen"))).on_hover_text(tr("Setzt die damals geschriebenen Felder zurück, sofern sie seitdem niemand geändert hat, und entfernt ergänzte Cover")).clicked() {
                self.report = None;
                self.journal = None;
                self.writing = Some((undo(path, ctx.clone()), true));
            }
        });
    }

    fn row(&mut self, ui: &mut egui::Ui, i: usize, h: f32, locked: bool, busy: &[PathBuf]) -> Option<BatchEvent> {
        let mut event = None;
        let item = &mut self.items[i];
        let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let resp = ui
            .horizontal(|ui| {
                ui.set_height(h);
                let writable = matches!(item.state, State::Sure | State::Unsure) && item.written.is_none();
                ui.add_enabled(writable && !locked, egui::Checkbox::without_text(&mut item.write));
                ui.vertical(|ui| {
                    ui.set_width(430.0);
                    ui.add(egui::Label::new(RichText::new(&name).size(12.0).color(theme::dim())).truncate());
                    let line = match &item.state {
                        State::Sure if item.cover_only => format!("{} – {}  ·  {}", item.artist, item.title, tr("schon erkannt, Cover fehlt")),
                        State::Sure => format!("{} – {}   {} %", item.artist, item.title, (item.score * 100.0).round()),
                        State::Unsure => {
                            let why: Vec<&str> = item.doubts.iter().map(|d| tr(d)).collect();
                            format!("{} – {}   {} %  ·  {}", item.artist, item.title, (item.score * 100.0).round(), why.join(", "))
                        }
                        State::Known => tr("schon erkannt").to_owned(),
                        State::Failed(e) => e.clone(),
                        State::NotFound => tr("nicht gefunden").to_owned(),
                        State::Waiting => String::new(),
                    };
                    ui.add(egui::Label::new(RichText::new(line).size(13.0)).truncate());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if item.state == State::Unsure && item.written.is_none() && ui.button(tr("Prüfen …")).clicked() {
                        event = Some(BatchEvent::Review(item.path.clone()));
                    }
                    let summary = match &item.written {
                        Some(Ok(())) => RichText::new(tr("erledigt")).color(theme::play()),
                        Some(Err(_)) => RichText::new(tr("Fehler")).color(theme::cue()),
                        None if busy.contains(&item.path) => RichText::new(tr("spielt gerade")).color(theme::cue()),
                        None if item.details_pending => RichText::new(tr("lädt …")).color(theme::dim()),
                        None if matches!(item.state, State::Sure | State::Unsure) => {
                            let mut parts: Vec<&str> = item.changes.iter().filter(|c| !c.new.is_empty()).map(|c| c.field.label()).collect();
                            if item.cover.is_some() {
                                parts.push(Field::Cover.label());
                            }
                            RichText::new(if parts.is_empty() { tr("nur Kennung").to_owned() } else { parts.join(", ") }).size(12.0).color(theme::dim())
                        }
                        None => RichText::new(""),
                    };
                    ui.add(egui::Label::new(summary).truncate());
                });
            })
            .response;
        // Einzelheiten beim Überfahren
        let details: Vec<String> = item
            .changes
            .iter()
            .filter(|c| !c.new.is_empty())
            .map(|c| format!("{}: {} → {}", c.field.label(), if c.old.trim().is_empty() { "–" } else { c.old.as_str() }, c.new))
            .chain(item.cover.as_ref().map(|_| format!("{}: – → {}", Field::Cover.label(), tr("neu"))))
            .chain(match &item.written {
                Some(Err(e)) => Some(e.clone()),
                _ => None,
            })
            .collect();
        if !details.is_empty() {
            resp.on_hover_text(details.join("\n"));
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online_tags::AlbumChoice;

    fn album(title: &str, artist: &str, kind: &str, year: &str, track: &str) -> AlbumChoice {
        AlbumChoice {
            title: title.into(),
            album_artist: artist.into(),
            kind: kind.into(),
            year: year.into(),
            track: track.into(),
            group_id: format!("g-{title}"),
            release_id: format!("r-{title}"),
        }
    }

    fn cand(title: &str, score: f32) -> Candidate {
        Candidate {
            score,
            by_sound: true,
            title: title.into(),
            artist: "Depeche Mode".into(),
            year: "1989".into(),
            recording_id: format!("rec-{title}"),
            artist_ids: vec!["dm".into()],
            albums: vec![album("Violator", "Depeche Mode", "Album", "1990", "6"), album("Fetenhits: The Real Classics 4", "Various Artists", "Compilation", "2000", "9")],
            durations: vec![223.0],
        }
    }

    fn tags(title: &str, artist: &str, album: &str) -> TagFields {
        TagFields { title: title.into(), artist: artist.into(), album: album.into(), genre: "Rock/Pop".into(), ..Default::default() }
    }

    #[test]
    fn bad_values() {
        assert!(is_bad(Field::Title, "", "x"));
        assert!(is_bad(Field::Title, "Track 07", "x"));
        assert!(is_bad(Field::Title, "09_Depeche Mode_Personal Jesus", "09_Depeche Mode_Personal Jesus"));
        assert!(!is_bad(Field::Title, "Personal Jesus", "09_Depeche Mode_Personal Jesus"));
        assert!(is_bad(Field::Artist, "Unknown Artist", ""));
        assert!(is_bad(Field::Artist, "Various Artists", ""));
        assert!(!is_bad(Field::Artist, "Pulp", ""));
        assert!(is_bad(Field::Year, "0", ""));
        assert!(!is_bad(Field::Year, "1995-11-27", ""));
        assert!(is_bad(Field::Track, "0", ""));
        assert!(!is_bad(Field::Track, "3/12", ""));
        assert!(is_bad(Field::Genre, "Other", ""));
        assert!(!is_bad(Field::Genre, "Schlager", ""));
    }

    #[test]
    fn sure_when_tags_match_and_fills_only_bad_fields() {
        // Vier Aufnahmen gleich gut (wie bei „Personal Jesus“); die Tags entscheiden
        let found = [cand("Dangerous", 0.98), cand("Personal Jesus (Telephone Stomp mix)", 0.98), cand("Personal Jesus", 0.98)];
        let p = plan(&tags("Personal Jesus", "Depeche Mode", "Fetenhits - the real classics 4 - CD 2"), "09_x", true, 224.0, &found).unwrap();
        assert!(p.sure);
        assert_eq!(p.candidate.title, "Personal Jesus");
        // Vorhandenes Album passt zur Zusammenstellung: Album bleibt, Album-Interpret und Track kommen von dort
        assert_eq!(p.album, Some(1));
        let changed: Vec<_> = p.changes.iter().map(|c| (c.field, c.new.as_str())).collect();
        assert_eq!(changed, [(Field::AlbumArtist, "Various Artists"), (Field::Year, "1989"), (Field::Track, "9")]);
        assert!(!p.need_genre && !p.need_cover);
        // Sampler, den die Dienste nicht kennen: Album bleibt, Cover vom besten Album
        let p = plan(&tags("Personal Jesus", "Depeche Mode", "Fetenhits - the real classics 2 - CD 1"), "x", false, 224.0, &[cand("Personal Jesus", 0.98)]).unwrap();
        assert_eq!(p.album, None);
        assert!(p.need_cover);
        assert!(p.changes.iter().all(|c| c.field != Field::Album));
    }

    #[test]
    fn unsure_cases() {
        let found = [cand("Personal Jesus", 0.98)];
        // Tags nennen einen anderen Titel
        assert!(!plan(&tags("Enjoy the Silence", "Depeche Mode", ""), "x", false, 223.0, &found).unwrap().sure);
        // Länge passt nicht (Extended Version)
        assert_eq!(plan(&tags("Personal Jesus", "Depeche Mode", ""), "x", false, 290.0, &found).unwrap().doubts, ["Länge weicht ab"]);
        // … außer eine der zusammengelegten Aufnahmen ist so lang
        let mut two = cand("Personal Jesus", 0.98);
        two.durations.push(288.0);
        assert!(plan(&tags("Personal Jesus", "Depeche Mode", ""), "x", false, 290.0, &[two]).unwrap().sure);
        // Zusatz im Tag-Titel stört nicht
        assert!(plan(&tags("Anspieltipp - Personal Jesus", "Depeche Mode", ""), "x", false, 223.0, &found).unwrap().sure);
        // Quote zu schwach
        assert!(!plan(&tags("Personal Jesus", "Depeche Mode", ""), "x", false, 223.0, &[cand("Personal Jesus", 0.8)]).unwrap().sure);
        // Ohne jeden Hinweis und mit gleich gutem anderen Titel
        let rivals = [cand("Personal Jesus", 0.98), cand("Dangerous", 0.97)];
        assert!(!plan(&tags("", "", ""), "Track01", false, 223.0, &rivals).unwrap().sure);
        assert!(plan(&tags("", "", ""), "x", false, 223.0, &[]).is_none());
    }

    #[test]
    fn empty_file_gets_everything_from_sound() {
        let p = plan(&TagFields::default(), "Track01", false, 223.0, &[cand("Personal Jesus", 0.97)]).unwrap();
        assert!(p.sure);
        assert_eq!(p.album, Some(0));
        let changed: Vec<_> = p.changes.iter().map(|c| (c.field, c.new.as_str())).collect();
        assert_eq!(
            changed,
            [(Field::Title, "Personal Jesus"), (Field::Artist, "Depeche Mode"), (Field::Album, "Violator"), (Field::AlbumArtist, "Depeche Mode"), (Field::Year, "1989"), (Field::Track, "6")]
        );
        assert!(p.need_genre && p.need_cover);
        // Dateiname „Interpret - Titel“ mit passendem Treffer gilt auch als Hinweis
        let p = plan(&TagFields::default(), "Depeche Mode - Personal Jesus", false, 223.0, &[cand("Personal Jesus", 0.97), cand("Dangerous", 0.97)]).unwrap();
        assert!(p.sure && p.candidate.title == "Personal Jesus");
    }

    /// Schreiben mit Journal und Rückgängig an einer kleinen WAV-Datei.
    #[test]
    fn write_and_undo() {
        let dir = std::env::temp_dir().join(format!("fw-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("titel.wav");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&(36u32 + 1600).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 8000u32.to_le_bytes().to_vec(), 16000u32.to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
            wav.extend(v);
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&1600u32.to_le_bytes());
        wav.resize(wav.len() + 1600, 0);
        std::fs::write(&path, wav).unwrap();
        fw_library::write_tags(&path, &tags("Personal Jesus", "", "Eigenes Album")).unwrap();
        let mut png = Vec::new();
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let job = WriteJob {
            index: 0,
            path: path.clone(),
            changes: vec![
                Change { field: Field::Artist, old: String::new(), new: "Depeche Mode".into() },
                Change { field: Field::Year, old: String::new(), new: "1989".into() },
                // Nicht mehr so wie bei der Erkennung: bleibt
                Change { field: Field::Album, old: String::new(), new: "Violator".into() },
            ],
            cover: Some(Arc::new(png)),
            recording_id: "rec".into(),
            release_id: "rel".into(),
        };
        let journal = dir.join("journal");
        let finished = |rx: Receiver<WriteMsg>| loop {
            if let WriteMsg::Finished(p) = rx.recv().unwrap() {
                break p;
            }
        };
        assert_eq!(finished(write_all(vec![job], Some(journal.clone()), egui::Context::default())), std::slice::from_ref(&path));
        let (f, _) = read_tags(&path).unwrap();
        assert_eq!((f.artist.as_str(), f.year.as_str(), f.album.as_str(), f.mb_recording_id.as_str()), ("Depeche Mode", "1989", "Eigenes Album", "rec"));
        assert!(has_cover(&path));
        // Inzwischen von Hand geändert: das Jahr bleibt beim Rückgängigmachen
        fw_library::write_tags(&path, &TagFields { year: "1990".into(), ..f }).unwrap();
        let (jpath, _, n) = latest_journal_in(&journal).unwrap();
        assert_eq!(n, 1);
        assert_eq!(finished(undo(jpath, egui::Context::default())), std::slice::from_ref(&path));
        let (f, _) = read_tags(&path).unwrap();
        assert_eq!((f.artist.as_str(), f.year.as_str(), f.title.as_str(), f.mb_recording_id.as_str()), ("", "1990", "Personal Jesus", ""));
        assert!(!has_cover(&path));
        assert!(latest_journal_in(&journal).is_none(), "zurückgenommenes Journal wird nicht mehr angeboten");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_respects_later_edits() {
        let mut f = tags("Personal Jesus", "", "");
        assert!(apply(&mut f, Field::Artist, "", "Depeche Mode"));
        assert_eq!(f.artist, "Depeche Mode");
        // Inzwischen von Hand geändert: nicht anfassen
        assert!(!apply(&mut f, Field::Title, "", "Anders"));
        assert_eq!(f.title, "Personal Jesus");
    }
}
