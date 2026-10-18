// SPDX-License-Identifier: GPL-3.0-or-later
//! Titel online erkennen und Tags vorschlagen: AcoustID (Fingerabdruck), MusicBrainz (Textsuche, Genres)
//! und Cover Art Archive (Cover).
//!
//! Gesendet werden nur der akustische Fingerabdruck und die Länge bzw. Interpret und Titel; die
//! Musikdatei verlässt den Rechner nie. Abgefragt wird wie in `update.rs` über `curl`. Alles läuft in
//! eigenen Threads; das Tag-Fenster liest die Ergebnisse über einen Kanal.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

/// Anwendungsschlüssel bei AcoustID (für Open-Source-Programme kostenlos; öffentlich wie bei anderen freien Programmen).
const ACOUSTID_KEY: &str = "J6VouBWIib";
const ACOUSTID_URL: &str = "https://api.acoustid.org/v2/lookup";
const MB_URL: &str = "https://musicbrainz.org/ws/2";
const CAA_URL: &str = "https://coverartarchive.org";
/// MusicBrainz erlaubt eine Anfrage pro Sekunde und Programm; etwas Luft lassen.
const MB_GAP: Duration = Duration::from_millis(1100);
/// So viele Vorschläge zeigt das Fenster höchstens.
const MAX_CANDIDATES: usize = 6;
/// AcoustID-Treffer darunter sind meist falsch.
const MIN_SCORE: f32 = 0.5;

/// Meldungen für das Tag-Fenster (deutscher Text = Schlüssel für `tr`, Übersetzung in i18n.rs).
const OFFLINE: &str = "Keine Verbindung zum Erkennungsdienst. Ist der Rechner online?";
const BUSY: &str = "MusicBrainz ist gerade überlastet. Bitte gleich noch einmal versuchen.";

/// Ein Album (MusicBrainz „Release Group“), auf dem die Aufnahme erschienen ist.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumChoice {
    pub title: String,
    pub album_artist: String,
    /// „Album“, „Single“, „EP“, „Compilation“, „Live“ …
    pub kind: String,
    pub year: String,
    pub track: String,
    pub group_id: String,
    pub release_id: String,
}

/// Ein Vorschlag: eine Aufnahme mit allen Alben, auf denen sie vorkommt (bestes zuerst).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Trefferquote 0–1
    pub score: f32,
    /// Über den Klang erkannt (sonst über die Textsuche).
    pub by_sound: bool,
    pub title: String,
    pub artist: String,
    /// Jahr der ersten Veröffentlichung
    pub year: String,
    pub recording_id: String,
    pub artist_ids: Vec<String>,
    pub albums: Vec<AlbumChoice>,
}

/// Nachrichten aus dem Such-Thread.
pub enum Progress {
    /// Deutscher Text für `tr`, was gerade passiert.
    Status(&'static str),
    /// Fehler als deutscher Text für `tr`.
    Done(Result<Vec<Candidate>, &'static str>),
}

/// Genre und Cover zu einem gewählten Album, nachgeladen.
#[derive(Debug, Clone, Default)]
pub struct Details {
    pub genre: String,
    /// JPEG/PNG wie geladen (zum Schreiben in die Datei)
    pub cover: Option<Vec<u8>>,
}

/// Sucht Vorschläge: erst über den Fingerabdruck, ohne Treffer über Interpret und Titel.
pub fn search(path: PathBuf, artist: String, title: String, ctx: egui::Context) -> Receiver<Progress> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-identify".into())
        .spawn(move || {
            let result = run_search(&path, &artist, &title, &tx).map_err(|e| {
                log::info!("Online-Erkennung {}: {e}", path.display());
                if e.contains("überlastet") { BUSY } else { OFFLINE }
            });
            let _ = tx.send(Progress::Done(result));
            ctx.request_repaint();
        })
        .ok();
    rx
}

fn run_search(path: &Path, artist: &str, title: &str, tx: &Sender<Progress>) -> Result<Vec<Candidate>, String> {
    let _ = tx.send(Progress::Status("Berechnet Fingerabdruck …"));
    let mut found = Vec::new();
    let mut offline = false;
    match fw_library::identify::fingerprint(path) {
        Ok(fp) => {
            let _ = tx.send(Progress::Status("Fragt AcoustID …"));
            match acoustid_lookup(&fp) {
                Ok(json) => found = parse_acoustid(&json),
                Err(e) => {
                    log::info!("AcoustID: {e}");
                    offline = true;
                }
            }
        }
        Err(e) => log::info!("Fingerabdruck von {}: {e:#}", path.display()),
    }
    if found.is_empty() && !title.trim().is_empty() {
        let _ = tx.send(Progress::Status("Sucht bei MusicBrainz …"));
        match mb_get(&format!("{MB_URL}/recording?query={}&fmt=json&limit=10", url_encode(&text_query(artist, title)))) {
            Ok(body) => {
                offline = false;
                let json: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
                found = parse_mb_search(&json);
            }
            Err(e) if found.is_empty() => return Err(e),
            Err(_) => {}
        }
    }
    rank_by_hint(&mut found, artist, title);
    if found.is_empty() && offline {
        return Err(OFFLINE.into());
    }
    Ok(found)
}

/// Lädt Genre (MusicBrainz) und Cover (Cover Art Archive) für ein Album nach.
pub fn details(album: AlbumChoice, artist_ids: Vec<String>, ctx: egui::Context) -> Receiver<Details> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("fw-identify-details".into())
        .spawn(move || {
            let mut d = Details::default();
            let genre_of = |url: String| -> Option<String> {
                let body = mb_get(&url).ok()?;
                top_genre(&serde_json::from_slice(&body).ok()?)
            };
            d.genre = genre_of(format!("{MB_URL}/release-group/{}?inc=genres&fmt=json", album.group_id))
                .or_else(|| artist_ids.first().and_then(|a| genre_of(format!("{MB_URL}/artist/{a}?inc=genres&fmt=json"))))
                .unwrap_or_default();
            d.cover = http_get(&format!("{CAA_URL}/release-group/{}/front-500", album.group_id))
                .or_else(|_| http_get(&format!("{CAA_URL}/release/{}/front-500", album.release_id)))
                .ok()
                .filter(|b| fw_library::meta::decode_cover(b).is_ok());
            let _ = tx.send(d);
            ctx.request_repaint();
        })
        .ok();
    rx
}

// ---------------------------------------------------------------- Netz

fn acoustid_lookup(fp: &fw_library::identify::Fingerprint) -> Result<Value, String> {
    let mut cmd = curl();
    for (k, v) in [
        ("client", ACOUSTID_KEY.to_string()),
        ("clientversion", env!("CARGO_PKG_VERSION").to_string()),
        ("format", "json".into()),
        ("meta", "recordings releasegroups releases tracks compress".into()),
        ("duration", fp.duration_secs.to_string()),
        ("fingerprint", fp.encoded.clone()),
    ] {
        cmd.arg("--data-urlencode").arg(format!("{k}={v}"));
    }
    let (status, body) = run(cmd.arg(ACOUSTID_URL))?;
    let json: Value = serde_json::from_slice(&body).map_err(|_| format!("AcoustID antwortet mit {status}"))?;
    if json["status"] != "ok" {
        return Err(json["error"]["message"].as_str().unwrap_or("Fehler").to_string());
    }
    Ok(json)
}

/// GET bei MusicBrainz: höchstens eine Anfrage pro Sekunde (für alle Threads), bei „busy“ (503) bis zu
/// dreimal neu versuchen.
fn mb_get(url: &str) -> Result<Vec<u8>, String> {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    for attempt in 0..4 {
        {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = *last {
                let wait = MB_GAP.saturating_sub(t.elapsed()) + Duration::from_secs(attempt);
                std::thread::sleep(wait);
            }
            *last = Some(Instant::now());
        }
        match http_get(url) {
            Err(e) if e.starts_with("503") => continue,
            other => return other,
        }
    }
    Err(BUSY.into())
}

fn http_get(url: &str) -> Result<Vec<u8>, String> {
    let (status, body) = run(curl().arg(url))?;
    if status == 200 { Ok(body) } else { Err(format!("{status} {url}")) }
}

fn curl() -> Command {
    let mut cmd = Command::new("curl");
    let agent = format!("FadeWerkDJ/{} ( {} )", env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_REPOSITORY"));
    // Statuscode hinter den Inhalt hängen (immer drei Zeichen, „000“ ohne Verbindung)
    cmd.args(["-sL", "--max-time", "20", "-w", "%{http_code}", "-A"]).arg(agent);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn run(cmd: &mut Command) -> Result<(u16, Vec<u8>), String> {
    let out = cmd.output().map_err(|e| format!("curl: {e}"))?;
    let mut body = out.stdout;
    if body.len() < 3 {
        return Err("keine Antwort".into());
    }
    let code = body.split_off(body.len() - 3);
    let status = std::str::from_utf8(&code).ok().and_then(|c| c.parse().ok()).unwrap_or(0);
    if status == 0 {
        return Err("keine Verbindung".into());
    }
    Ok((status, body))
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Lucene-Abfrage für die MusicBrainz-Suche; Zusätze wie „(Radio Edit)“ und „feat. …“ fallen weg,
/// weil sie dort oft anders geschrieben sind.
fn text_query(artist: &str, title: &str) -> String {
    let clean = |s: &str| -> String {
        let s = s.split(['(', '[']).next().unwrap_or(s);
        let lower = s.to_lowercase();
        let cut = [" feat. ", " feat ", " ft. ", " featuring "].iter().filter_map(|f| lower.find(f)).min().unwrap_or(s.len());
        s[..cut].replace(['"', '\\'], " ").trim().to_string()
    };
    let (artist, title) = (clean(artist), clean(title));
    if artist.is_empty() { format!("recording:\"{title}\"") } else { format!("artist:\"{artist}\" AND recording:\"{title}\"") }
}

// ---------------------------------------------------------------- Auswerten

/// Antwort von AcoustID (mit `meta=recordings releasegroups releases tracks`) → Vorschläge.
pub fn parse_acoustid(json: &Value) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for result in json["results"].as_array().into_iter().flatten() {
        let score = result["score"].as_f64().unwrap_or(0.0) as f32;
        if score < MIN_SCORE {
            continue;
        }
        for rec in result["recordings"].as_array().into_iter().flatten() {
            let Some(title) = rec["title"].as_str() else { continue };
            let mut albums = Vec::new();
            for g in rec["releasegroups"].as_array().into_iter().flatten() {
                let kind = group_kind(g["type"].as_str(), g["secondarytypes"].as_array().into_iter().flatten().filter_map(Value::as_str));
                // Früheste Ausgabe des Albums: Jahr und Titelnummer
                let best = g["releases"].as_array().into_iter().flatten().min_by_key(|r| r["date"]["year"].as_u64().unwrap_or(9999));
                let Some(release) = best else { continue };
                let track = release["mediums"][0]["tracks"][0]["position"].as_u64().map(|p| p.to_string()).unwrap_or_default();
                albums.push(AlbumChoice {
                    title: g["title"].as_str().unwrap_or_default().to_string(),
                    album_artist: join_artists(&g["artists"], "name"),
                    kind,
                    year: release["date"]["year"].as_u64().map(|y| y.to_string()).unwrap_or_default(),
                    track,
                    group_id: g["id"].as_str().unwrap_or_default().to_string(),
                    release_id: release["id"].as_str().unwrap_or_default().to_string(),
                });
            }
            add(
                &mut out,
                Candidate {
                    score,
                    by_sound: true,
                    title: title.to_string(),
                    artist: join_artists(&rec["artists"], "name"),
                    year: String::new(),
                    recording_id: rec["id"].as_str().unwrap_or_default().to_string(),
                    artist_ids: ids(&rec["artists"]),
                    albums,
                },
            );
        }
    }
    finish(out)
}

/// Antwort der MusicBrainz-Suche nach Aufnahmen → Vorschläge.
pub fn parse_mb_search(json: &Value) -> Vec<Candidate> {
    let mut out = Vec::new();
    for rec in json["recordings"].as_array().into_iter().flatten() {
        let score = rec["score"].as_f64().unwrap_or(0.0) as f32 / 100.0;
        if score < MIN_SCORE {
            continue;
        }
        // Je Album nur die früheste Ausgabe (verglichen am ganzen Datum)
        let mut albums: Vec<(String, AlbumChoice)> = Vec::new();
        for r in rec["releases"].as_array().into_iter().flatten() {
            let g = &r["release-group"];
            let kind = group_kind(g["primary-type"].as_str(), g["secondary-types"].as_array().into_iter().flatten().filter_map(Value::as_str));
            let date = r["date"].as_str().unwrap_or_default().to_string();
            let choice = AlbumChoice {
                title: g["title"].as_str().or(r["title"].as_str()).unwrap_or_default().to_string(),
                album_artist: join_artists(&r["artist-credit"], "name"),
                kind,
                year: year_of(&date),
                track: r["media"][0]["track"][0]["number"].as_str().unwrap_or_default().to_string(),
                group_id: g["id"].as_str().unwrap_or_default().to_string(),
                release_id: r["id"].as_str().unwrap_or_default().to_string(),
            };
            match albums.iter_mut().find(|(_, a)| a.group_id == choice.group_id) {
                Some(slot) if !date.is_empty() && (slot.0.is_empty() || date < slot.0) => *slot = (date, choice),
                Some(_) => {}
                None => albums.push((date, choice)),
            }
        }
        let albums = albums.into_iter().map(|(_, a)| a).collect();
        add(
            &mut out,
            Candidate {
                score,
                by_sound: false,
                title: rec["title"].as_str().unwrap_or_default().to_string(),
                artist: join_artists(&rec["artist-credit"], "name"),
                year: year_of(rec["first-release-date"].as_str().unwrap_or_default()),
                recording_id: rec["id"].as_str().unwrap_or_default().to_string(),
                artist_ids: rec["artist-credit"].as_array().into_iter().flatten().filter_map(|c| c["artist"]["id"].as_str()).map(str::to_string).collect(),
                albums,
            },
        );
    }
    finish(out)
}

/// Fügt einen Vorschlag hinzu; dieselbe Aufnahme (Interpret + Titel) wird mit dem vorhandenen zusammengelegt.
fn add(out: &mut Vec<Candidate>, c: Candidate) {
    if c.title.is_empty() {
        return;
    }
    let key = |c: &Candidate| (c.artist.to_lowercase(), c.title.to_lowercase());
    match out.iter_mut().find(|o| key(o) == key(&c)) {
        Some(o) => {
            o.score = o.score.max(c.score);
            for a in c.albums {
                if !o.albums.iter().any(|b| b.group_id == a.group_id) {
                    o.albums.push(a);
                }
            }
            if o.year.is_empty() || (!c.year.is_empty() && c.year < o.year) {
                o.year = c.year;
            }
        }
        None => out.push(c),
    }
}

/// Alben sortieren (Studioalbum vor Single/EP vor Zusammenstellungen, dann älteste zuerst), Jahr der
/// ersten Veröffentlichung bestimmen, nach Trefferquote ordnen.
fn finish(mut out: Vec<Candidate>) -> Vec<Candidate> {
    for c in &mut out {
        c.albums.sort_by(|a, b| (kind_rank(&a.kind), a.year.is_empty(), &a.year).cmp(&(kind_rank(&b.kind), b.year.is_empty(), &b.year)));
        if let Some(first) = c.albums.iter().map(|a| &a.year).filter(|y| !y.is_empty()).min()
            && (c.year.is_empty() || *first < c.year)
        {
            c.year = first.clone();
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(MAX_CANDIDATES);
    out
}

/// Bei (fast) gleicher Trefferquote zuerst, was zu den vorhandenen Angaben passt: Ein Fingerabdruck trifft
/// oft mehrere Aufnahmen einer Single gleich gut (auch die B-Seite).
fn rank_by_hint(list: &mut [Candidate], artist: &str, title: &str) {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let (artist, title) = (norm(artist), norm(title));
    let fit = |c: &Candidate| -> u8 {
        let t = norm(&c.title);
        let title_fit = if title.is_empty() { 0 } else if t == title { 3 } else if t.contains(&title) || title.contains(&t) { 2 } else { 0 };
        let a = norm(&c.artist);
        title_fit + u8::from(!artist.is_empty() && (a.contains(&artist) || artist.contains(&a)))
    };
    list.sort_by_key(|c| (std::cmp::Reverse((c.score * 20.0).round() as u32), std::cmp::Reverse(fit(c))));
}

/// „Album“, „Single“, „EP“; mit Zusatzart wie „Compilation“ oder „Live“ gilt die Zusatzart.
fn group_kind<'a>(primary: Option<&str>, mut secondary: impl Iterator<Item = &'a str>) -> String {
    secondary.next().or(primary).unwrap_or("Other").to_string()
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "Album" => 0,
        "EP" => 1,
        "Single" => 2,
        "Other" | "Broadcast" => 3,
        "Soundtrack" => 4,
        "Compilation" => 5,
        _ => 6,
    }
}

/// Mitwirkende zu einem Namen verbinden; MusicBrainz liefert die Verbindungswörter mit („ feat. “),
/// sonst „, “.
fn join_artists(list: &Value, name_key: &str) -> String {
    let mut s = String::new();
    let items: Vec<&Value> = list.as_array().into_iter().flatten().collect();
    for (i, a) in items.iter().enumerate() {
        s.push_str(a[name_key].as_str().unwrap_or_default());
        match a["joinphrase"].as_str() {
            Some(j) => s.push_str(j),
            None if i + 1 < items.len() => s.push_str(", "),
            None => {}
        }
    }
    s.trim().to_string()
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array().into_iter().flatten().filter_map(|a| a["id"].as_str()).map(str::to_string).collect()
}

fn year_of(date: &str) -> String {
    date.get(..4).filter(|y| y.chars().all(|c| c.is_ascii_digit())).unwrap_or_default().to_string()
}

/// Meistgenanntes Genre („genres“ bei MusicBrainz), Wortanfänge groß („pop rock“ → „Pop Rock“).
fn top_genre(json: &Value) -> Option<String> {
    let best = json["genres"].as_array()?.iter().filter(|g| g["count"].as_u64().unwrap_or(0) > 0).max_by_key(|g| g["count"].as_u64())?;
    let name = best["name"].as_str()?;
    Some(
        name.split(' ')
            .map(|w| match w {
                "and" | "n" | "of" | "the" => w.to_string(),
                _ => {
                    let mut c = w.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn group(id: &str, title: &str, kind: &str, secondary: &[&str], year: u64, track: u64) -> Value {
        json!({
            "id": id, "title": title, "type": kind, "secondarytypes": secondary,
            "artists": [{"id": "a1", "name": "Pulp"}],
            "releases": [
                {"id": format!("{id}-r2"), "date": {"year": year + 3}, "mediums": [{"position": 1, "tracks": [{"position": track + 1}]}]},
                {"id": format!("{id}-r1"), "date": {"year": year}, "mediums": [{"position": 1, "tracks": [{"position": track}]}]},
            ],
        })
    }

    #[test]
    fn acoustid_prefers_studio_album_and_first_year() {
        let answer = json!({"status": "ok", "results": [
            {"score": 0.96, "recordings": [{"id": "rec1", "title": "Disco 2000", "artists": [{"id": "a1", "name": "Pulp"}],
                "releasegroups": [
                    group("g1", "Fetenhits", "Album", &["Compilation"], 2000, 11),
                    group("g2", "Disco 2000", "Single", &[], 1995, 1),
                    group("g3", "Different Class", "Album", &[], 1995, 7),
                ]}]},
            {"score": 0.94, "recordings": [{"id": "rec2", "title": "Disco 2000", "artists": [{"id": "a1", "name": "Pulp"}],
                "releasegroups": [group("g4", "Hits", "Album", &["Compilation"], 2002, 3)]}]},
            {"score": 0.3, "recordings": [{"id": "rec3", "title": "Falsch", "artists": []}]},
            {"score": 0.9},
        ]});
        let c = parse_acoustid(&answer);
        assert_eq!(c.len(), 1, "gleiche Aufnahme zusammengelegt, schwacher Treffer verworfen");
        let c = &c[0];
        assert_eq!((c.artist.as_str(), c.title.as_str(), c.year.as_str()), ("Pulp", "Disco 2000", "1995"));
        assert!(c.by_sound && (c.score - 0.96).abs() < 1e-6);
        let titles: Vec<_> = c.albums.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Different Class", "Disco 2000", "Fetenhits", "Hits"]);
        let best = &c.albums[0];
        assert_eq!((best.track.as_str(), best.year.as_str(), best.release_id.as_str(), best.kind.as_str()), ("7", "1995", "g3-r1", "Album"));
        assert_eq!(c.albums[2].kind, "Compilation");
    }

    #[test]
    fn acoustid_error_or_empty_gives_nothing() {
        assert!(parse_acoustid(&json!({"status": "ok", "results": []})).is_empty());
        assert!(parse_acoustid(&json!({"status": "error"})).is_empty());
        assert!(parse_acoustid(&json!("Unsinn")).is_empty());
    }

    #[test]
    fn musicbrainz_search() {
        let answer = json!({"recordings": [
            {"id": "r1", "score": 100, "title": "One More Time", "first-release-date": "2000-11-13",
             "artist-credit": [{"name": "Daft Punk", "artist": {"id": "dp"}}],
             "releases": [
                {"id": "x1", "title": "Boom 2001", "date": "2001-12",
                 "artist-credit": [{"name": "Various Artists"}],
                 "release-group": {"id": "gx", "title": "Boom 2001", "primary-type": "Album", "secondary-types": ["Compilation"]},
                 "media": [{"track": [{"number": "17"}]}]},
                {"id": "d2", "title": "Discovery", "date": "2001-03-12",
                 "artist-credit": [{"name": "Daft Punk"}],
                 "release-group": {"id": "gd", "title": "Discovery", "primary-type": "Album"},
                 "media": [{"track": [{"number": "1"}]}]},
                {"id": "d1", "title": "Discovery", "date": "2001-03-07",
                 "release-group": {"id": "gd", "title": "Discovery", "primary-type": "Album"},
                 "media": [{"track": [{"number": "A1"}]}]},
             ]},
            {"id": "r2", "score": 40, "title": "Anderes"},
            {"id": "r3", "score": 90, "title": "Mit Gast", "artist-credit": [
                {"name": "A", "joinphrase": " feat. ", "artist": {"id": "1"}}, {"name": "B", "artist": {"id": "2"}}]},
        ]});
        let c = parse_mb_search(&answer);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].title.as_str(), c[0].year.as_str(), c[0].artist_ids[0].as_str()), ("One More Time", "2000", "dp"));
        assert!(!c[0].by_sound);
        assert_eq!(c[0].albums.len(), 2);
        assert_eq!((c[0].albums[0].title.as_str(), c[0].albums[0].release_id.as_str(), c[0].albums[0].track.as_str()), ("Discovery", "d1", "A1"));
        assert_eq!(c[1].artist, "A feat. B");
    }

    #[test]
    fn hint_breaks_ties() {
        let c = |title: &str, score: f32| Candidate {
            score,
            by_sound: true,
            title: title.into(),
            artist: "Depeche Mode".into(),
            year: String::new(),
            recording_id: String::new(),
            artist_ids: vec![],
            albums: vec![],
        };
        let mut list = vec![c("Dangerous", 0.98), c("Personal Jesus (Telephone Stomp mix)", 0.98), c("Personal Jesus", 0.98), c("Sonst", 0.99)];
        rank_by_hint(&mut list, "Depeche Mode", "Personal Jesus");
        let titles: Vec<_> = list.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Personal Jesus", "Personal Jesus (Telephone Stomp mix)", "Dangerous", "Sonst"]);
        // Deutlich bessere Trefferquote geht vor
        let mut list = vec![c("Personal Jesus", 0.6), c("Dangerous", 0.98)];
        rank_by_hint(&mut list, "Depeche Mode", "Personal Jesus");
        assert_eq!(list[0].title, "Dangerous");
    }

    #[test]
    fn query_and_genre() {
        assert_eq!(text_query("Cro feat. Bausa", "Bye Bye (Radio Edit)"), "artist:\"Cro\" AND recording:\"Bye Bye\"");
        assert_eq!(text_query("", "Ti\"tel"), "recording:\"Ti tel\"");
        assert_eq!(url_encode("a b&ü"), "a%20b%26%C3%BC");
        let g = json!({"genres": [{"name": "pop", "count": 1}, {"name": "drum and bass", "count": 4}, {"name": "x", "count": 0}]});
        assert_eq!(top_genre(&g).as_deref(), Some("Drum and Bass"));
        assert_eq!(top_genre(&json!({"genres": []})), None);
    }
}
