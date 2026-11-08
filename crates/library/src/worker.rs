// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use anyhow::Context;
use crossbeam_channel::{Receiver, Sender, unbounded};
use fw_engine::TrackAudio;

use crate::beatgrid::{BeatGrid, estimate_grid};
use crate::bpm::estimate_bpm;
use crate::key::{Camelot, estimate_key};
use crate::decode::{Early, decode_file, decode_file_bytes_early, read_audio_without_id3};
use crate::meta::{Cover, TrackMeta, read_meta, read_meta_bytes};
use crate::scan::{list_dir, walk_audio_progress};
use crate::sketch::{WaveSketch, read_cached, write_cached};
use crate::waveform::Waveform;

/// Ein fertig geladener Titel für einen Player oder das Vorhören.
pub struct Loaded {
    pub deck: usize,
    /// Kennung der Anfrage. Die Oberfläche verwirft Ergebnisse, die inzwischen überholt sind.
    pub token: u64,
    pub path: PathBuf,
    pub meta: TrackMeta,
    pub audio: Arc<TrackAudio>,
    pub waveform: Waveform,
    /// Kleine Wellenform für die Playlist-Zeile (auch gleich im Cache gemerkt).
    pub sketch: Arc<WaveSketch>,
    pub bpm: Option<f32>,
    /// `bpm` stammt aus der Tempo-Erkennung, nicht aus dem Tag (dann lohnt es sich, ihn zu merken).
    pub bpm_analyzed: bool,
    /// Tonart aus dem Tag oder der Erkennung (`None` = unsicher).
    pub key: Option<Camelot>,
    /// `key` stammt aus der Erkennung (dann wird es gemerkt, auch ein „unsicher“).
    pub key_analyzed: bool,
}

/// Höchstzahl an Titeln pro Ordnersuche, damit ein Klick auf „Benutzerordner“ nicht ewig sucht.
pub const SCAN_LIMIT: usize = 50_000;

pub enum Response {
    /// Gelesene Tags samt Größe und Änderungszeit der Datei (`None`, wenn die nicht lesbar sind).
    Meta { path: PathBuf, meta: TrackMeta, stamp: Option<Stamp> },
    /// Ergebnis von [`Library::check_meta`]: die Datei ist unverändert, die gemerkten Tags gelten.
    /// (Hat sie sich geändert, kommt stattdessen [`Response::Meta`] mit den neuen Tags.)
    MetaValid { path: PathBuf, stamp: Stamp },
    /// Gefundene Titel einer Ordnersuche. `done` kommt einmal am Ende, `limited` wenn die Höchstzahl erreicht ist.
    /// `progress` = geschätzter erledigter Anteil (0 bis 1, siehe [`crate::walk_audio_progress`]).
    Scan { token: u64, tracks: Vec<PathBuf>, progress: f32, done: bool, limited: bool },
    /// Anfang eines Titels, der noch dekodiert wird (nach [`PARTIAL_SECS`]): Der Player kann schon spielen, der
    /// vollständige Titel folgt als [`Response::Loaded`] mit derselben Kennung.
    Partial { deck: usize, token: u64, path: PathBuf, meta: TrackMeta, audio: Arc<TrackAudio> },
    Loaded(Box<Loaded>),
    /// Cover eines eben geladenen Titels, nachgereicht: Dekodieren und Verkleinern großer Bilder dauerte auf
    /// schwachen Rechnern viele Sekunden, so lange musste der Player sonst warten.
    Cover { deck: usize, token: u64, cover: Cover },
    Failed { path: PathBuf, deck: Option<usize>, error: String },
    /// Ergebnis von [`Library::analyze`]: BPM und Tonart, je aus dem Tag oder (`*_analyzed`) aus der Erkennung.
    /// `None`, wenn die Datei nicht lesbar ist, kein Beat erkennbar war bzw. die Tonart unsicher ist.
    /// `readable` = die Datei ließ sich dekodieren (sonst nichts merken).
    /// `grid`: Beatraster, wenn es berechnet wurde (`Some(None)` = unsicher), sonst `None`.
    Analysis { path: PathBuf, bpm: Option<f32>, bpm_analyzed: bool, key: Option<Camelot>, key_analyzed: bool, readable: bool, grid: Option<Option<BeatGrid>> },
    /// Ergebnis von [`Library::request_sketches`], `None` wenn die Datei nicht lesbar ist.
    Sketch { path: PathBuf, sketch: Option<Arc<WaveSketch>> },
}

/// Warteschlange für Wellenform-Skizzen. Die neuesten Anfragen kommen zuerst dran: das sind die Zeilen,
/// die gerade sichtbar sind, während weggescrollte warten.
#[derive(Default)]
struct SketchQueue {
    jobs: Mutex<VecDeque<PathBuf>>,
    ready: Condvar,
}

/// Größe und Änderungszeit (Millisekunden seit 1970) einer Datei: ändert sich eins davon, gelten gemerkte
/// Angaben (Tags, BPM) nicht mehr.
pub type Stamp = (u64, u64);

/// Größe und Änderungszeit der Datei, `None` wenn sie nicht lesbar ist.
pub fn file_stamp(path: &std::path::Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    Some((m.len(), modified))
}

/// Threads zum Lesen der Tags. Auf Netzlaufwerken wartet jeder fast nur auf Antworten des Servers;
/// mehrere gleichzeitig verkürzen das Lesen einer großen Sammlung deutlich.
const TAG_THREADS: usize = 4;

/// Threads für die BPM-Analyse. Zwei reichen: Dekodieren ist schwer, und während des Auflegens soll
/// genug Rechenzeit für Oberfläche und Laden bleiben.
const BPM_THREADS: usize = 2;

/// Größtes eingebettetes Cover-Bild, das noch dekodiert wird.
const COVER_MAX_BYTES: usize = 8_000_000;

/// Auf schwachen Rechnern (höchstens zwei logische Kerne, etwa ein Atom-Netbook) nur ein Thread je Aufgabe:
/// Sonst kratzte die laufende Musik, während ein großer Ordner gelesen oder harmonisch sortiert wurde.
fn threads(normal: usize) -> usize {
    match std::thread::available_parallelism() {
        Ok(n) if n.get() <= 2 => 1,
        _ => normal,
    }
}

struct BpmJob {
    generation: u64,
    path: PathBuf,
    /// Auch analysieren, wenn der Tag schon eine BPM-Angabe hat.
    force: bool,
}

struct LoadJob {
    deck: usize,
    token: u64,
    path: PathBuf,
    sketch_cache: Option<PathBuf>,
    /// Cover gebraucht? Nur auf sichtbaren Plattentellern; sonst spart das Dekodieren großer Bilder viel Zeit.
    cover: bool,
    /// Schon bekannte Tags (aus dem Tag-Cache der App). Mit ihnen und ohne Cover muss der ID3-Block einer MP3
    /// nicht gelesen werden.
    known: Option<TrackMeta>,
    /// Schon bekanntes Tempo und schon bekannte Tonart (Tag oder Cache der App; Tonart `Some(None)` = unsicher).
    /// Was fehlt, wird erst nach dem Laden ermittelt und als [`Response::Analysis`] nachgereicht.
    known_bpm: Option<f32>,
    known_key: Option<Option<Camelot>>,
    /// Beatraster schon bekannt (auch „unsicher“)? Sonst wird es nach dem Laden berechnet.
    known_grid: bool,
}

/// Nach so vielen dekodierten Sekunden bekommt der Player den Anfang des Titels ([`Response::Partial`]).
/// Der Rest ist auf schwachen Rechnern nach etwa 10 s fertig, lange bevor der Anfang abgespielt ist.
const PARTIAL_SECS: f64 = 30.0;

/// Was beim Laden schon bekannt ist (siehe [`Library::load`]).
#[derive(Default)]
pub struct Known {
    pub meta: Option<TrackMeta>,
    pub bpm: Option<f32>,
    pub key: Option<Option<Camelot>>,
    /// Beatraster schon bekannt (auch „unsicher“).
    pub grid: bool,
}

/// Hintergrund-Arbeiter: ein Thread liest Tags, einer dekodiert und analysiert.
/// Getrennt, damit ein Ordner voller Tag-Anfragen das Laden in einen Player nicht aufhält.
pub struct Library {
    meta_tx: Sender<PathBuf>,
    /// Vorrang für Tags gerade sichtbarer Zeilen ([`Library::request_meta_now`]): Pfade auf einem Stapel
    /// (die neuesten zuerst), je Pfad ein Signal im Kanal, damit die Tag-Threads ihn mit `select_biased!` vorziehen.
    urgent: Arc<Mutex<Vec<PathBuf>>>,
    urgent_tx: Sender<()>,
    check_tx: Sender<(PathBuf, Stamp)>,
    scan_tx: Sender<(u64, Vec<(PathBuf, bool)>)>,
    /// Kennung der aktuellen Ordnersuche; ältere Suchen brechen ab.
    scan_current: Arc<AtomicU64>,
    load_tx: Sender<LoadJob>,
    bpm_tx: Sender<BpmJob>,
    /// Aufträge älterer Generationen werden übersprungen ([`Library::cancel_analysis`]).
    bpm_generation: Arc<AtomicU64>,
    sketches: Arc<SketchQueue>,
    /// Ordner des Wellenform-Caches (`None` = nichts auf der Platte merken).
    sketch_cache: Arc<Mutex<Option<PathBuf>>>,
    rx: Receiver<Response>,
}

impl Library {
    /// Startet die Arbeiter. `wake` wird nach jedem Ergebnis aufgerufen, damit die Oberfläche neu zeichnet.
    pub fn start(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (meta_tx, meta_rx) = unbounded::<PathBuf>();
        let (check_tx, check_rx) = unbounded::<(PathBuf, Stamp)>();
        let (urgent_tx, urgent_rx) = unbounded::<()>();
        let urgent = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
        let (load_tx, load_rx) = unbounded::<LoadJob>();
        let (tx, rx) = unbounded::<Response>();
        let (scan_tx, scan_rx) = unbounded::<(u64, Vec<(PathBuf, bool)>)>();
        let scan_current = Arc::new(AtomicU64::new(0));
        let (bpm_tx, bpm_rx) = unbounded::<BpmJob>();
        let bpm_generation = Arc::new(AtomicU64::new(0));

        for i in 0..threads(BPM_THREADS) {
            let (tx, wake, rx, generation) = (tx.clone(), wake.clone(), bpm_rx.clone(), bpm_generation.clone());
            thread::Builder::new()
                .name(format!("fw-bpm-{i}"))
                .spawn(move || {
                    background_priority();
                    for job in rx {
                        if generation.load(Ordering::Relaxed) != job.generation {
                            continue;
                        }
                        let a = analyze(&job);
                        let resp = Response::Analysis { path: job.path, bpm: a.bpm, bpm_analyzed: a.bpm_analyzed, key: a.key, key_analyzed: a.key_analyzed, readable: a.readable, grid: a.grid };
                        if tx.send(resp).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("BPM-Thread startet nicht");
        }

        {
            let tx = tx.clone();
            let wake = wake.clone();
            let current = scan_current.clone();
            thread::Builder::new()
                .name("fw-scan".into())
                .spawn(move || {
                    background_priority();
                    for (token, paths) in scan_rx {
                        if current.load(Ordering::Relaxed) != token {
                            continue;
                        }
                        let total = std::cell::Cell::new(0usize);
                        let count = paths.len().max(1) as f64;
                        // Ordner ohne Titel melden nur den Fortschritt, und das höchstens alle 100 ms
                        let mut last_sent = std::time::Instant::now();
                        for (i, (path, recursive)) in paths.iter().enumerate() {
                            let mut emit = |tracks: Vec<PathBuf>, part: f64| {
                                // Entwicklerschalter: langsames Laufwerk nachstellen (Millisekunden je Ordner)
                                #[cfg(debug_assertions)]
                                if let Some(ms) = std::env::var("FW_SCAN_SLOW").ok().and_then(|v| v.parse().ok()) {
                                    thread::sleep(std::time::Duration::from_millis(ms));
                                }
                                if tracks.is_empty() && last_sent.elapsed() < std::time::Duration::from_millis(100) {
                                    return;
                                }
                                last_sent = std::time::Instant::now();
                                total.set(total.get() + tracks.len());
                                let progress = ((i as f64 + part) / count) as f32;
                                let _ = tx.send(Response::Scan { token, tracks, progress, done: false, limited: false });
                                wake();
                            };
                            if *recursive {
                                walk_audio_progress(path, || current.load(Ordering::Relaxed) == token && total.get() < SCAN_LIMIT, &mut emit);
                            } else if let Ok(listing) = list_dir(path) {
                                // Ohne Unterordner: nur die Titel direkt in diesem Ordner
                                emit(listing.tracks, 1.0);
                            }
                        }
                        if current.load(Ordering::Relaxed) == token {
                            let _ = tx.send(Response::Scan { token, tracks: Vec::new(), progress: 1.0, done: true, limited: total.get() >= SCAN_LIMIT });
                            wake();
                        }
                    }
                })
                .expect("Such-Thread startet nicht");
        }

        for i in 0..threads(TAG_THREADS) {
            let (tx, wake, meta_rx, check_rx) = (tx.clone(), wake.clone(), meta_rx.clone(), check_rx.clone());
            let (urgent_rx, urgent) = (urgent_rx.clone(), urgent.clone());
            thread::Builder::new()
                .name(format!("fw-tags-{i}"))
                .spawn(move || {
                    background_priority();
                    loop {
                        // Sichtbare Zeilen zuerst, dann Tags, die noch niemand kennt, zuletzt das Prüfen gemerkter Tags
                        let resp = crossbeam_channel::select_biased! {
                            recv(urgent_rx) -> job => match job {
                                Ok(()) => match urgent.lock().unwrap_or_else(|e| e.into_inner()).pop() {
                                    Some(path) => read_tags_response(path),
                                    None => continue,
                                },
                                Err(_) => break,
                            },
                            recv(meta_rx) -> job => match job {
                                Ok(path) => read_tags_response(path),
                                Err(_) => break,
                            },
                            recv(check_rx) -> job => match job {
                                Ok((path, stamp)) => match file_stamp(&path) {
                                    Some(now) if now == stamp => Response::MetaValid { path, stamp },
                                    _ => read_tags_response(path),
                                },
                                Err(_) => break,
                            },
                        };
                        if tx.send(resp).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("Tag-Thread startet nicht");
        }

        let sketches = Arc::new(SketchQueue::default());
        let sketch_cache = Arc::new(Mutex::new(None::<PathBuf>));
        {
            let (tx, wake, queue, cache) = (tx.clone(), wake.clone(), sketches.clone(), sketch_cache.clone());
            thread::Builder::new()
                .name("fw-wave".into())
                .spawn(move || {
                    background_priority();
                    loop {
                        let path = {
                            let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                match jobs.pop_back() {
                                    Some(p) => break p,
                                    None => jobs = queue.ready.wait(jobs).unwrap_or_else(|e| e.into_inner()),
                                }
                            }
                        };
                        let dir = cache.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        let sketch = make_sketch(&path, dir.as_deref());
                        if tx.send(Response::Sketch { path, sketch }).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("Wellenform-Thread startet nicht");
        }

        thread::Builder::new()
            .name("fw-loader".into())
            .spawn(move || {
                // Laden ist eine direkte Aktion: nur leicht zurückgestellt, sonst dauerte es neben der Oberfläche ewig
                set_nice(LOADER_NICE);
                for job in load_rx {
                    let partial_tx = tx.clone();
                    let partial_wake = wake.clone();
                    let mut send_partial = |p: Response| {
                        let _ = partial_tx.send(p);
                        partial_wake();
                    };
                    let (resp, picture, analysis) = match load(&job, &mut send_partial) {
                        Ok(r) => (Response::Loaded(Box::new(r.loaded)), r.picture, r.analysis),
                        Err(e) => (Response::Failed { path: job.path.clone(), deck: Some(job.deck), error: format!("{e:#}") }, None, None),
                    };
                    if tx.send(resp).is_err() {
                        break;
                    }
                    wake();
                    // Fehlende Analysen erst jetzt: der Titel liegt schon im Player
                    if let Some(audio) = analysis {
                        let t = std::time::Instant::now();
                        let bpm = job.known_bpm.or_else(|| estimate_bpm(&audio));
                        let key = job.known_key.unwrap_or_else(|| estimate_key(&audio));
                        let grid = (!job.known_grid).then(|| estimate_grid(&audio, bpm));
                        log::info!("Analyse nach dem Laden {} ms", t.elapsed().as_millis());
                        let resp = Response::Analysis {
                            path: job.path.clone(),
                            bpm,
                            bpm_analyzed: job.known_bpm.is_none(),
                            key,
                            key_analyzed: job.known_key.is_none(),
                            readable: true,
                            grid,
                        };
                        if tx.send(resp).is_err() {
                            break;
                        }
                        wake();
                    }
                    // Erst jetzt das Cover: der Titel liegt schon im Player
                    if let Some(data) = picture {
                        // Riesige eingebettete Bilder (beobachtet: 57 MB) kosten auf schwachen Rechnern viele Sekunden
                        // und Hunderte MB Speicher; für einen Plattenteller mit 512 Pixeln lohnt das nicht
                        if data.len() > COVER_MAX_BYTES {
                            log::info!("Cover übersprungen: {} MB in {}", data.len() / 1_000_000, job.path.display());
                            continue;
                        }
                        let t = std::time::Instant::now();
                        match crate::meta::decode_cover(&data) {
                            Ok(cover) => {
                                log::info!("Cover nach {} ms ({} KB)", t.elapsed().as_millis(), data.len() / 1024);
                                if tx.send(Response::Cover { deck: job.deck, token: job.token, cover }).is_err() {
                                    break;
                                }
                                wake();
                            }
                            Err(e) => log::debug!("Cover in {} nicht lesbar: {e}", job.path.display()),
                        }
                    }
                }
            })
            .expect("Lade-Thread startet nicht");

        Self { meta_tx, urgent, urgent_tx, check_tx, scan_tx, scan_current, load_tx, bpm_tx, bpm_generation, sketches, sketch_cache, rx }
    }

    /// Startet eine Suche nach Titeln in `paths`, Ordner für Ordner in dieser Reihenfolge; `true` heißt
    /// mit Unterordnern. Eine laufende Suche bricht ab.
    pub fn scan(&self, token: u64, paths: Vec<(PathBuf, bool)>) {
        self.scan_current.store(token, Ordering::Relaxed);
        let _ = self.scan_tx.send((token, paths));
    }

    /// Bricht die laufende Suche ab. Bereits gefundene Titel bleiben.
    pub fn cancel_scan(&self) {
        self.scan_current.store(0, Ordering::Relaxed);
    }

    pub fn request_meta(&self, path: PathBuf) {
        let _ = self.meta_tx.send(path);
    }

    /// Wie [`Self::request_meta`], aber vor allen wartenden Anfragen, die zuletzt angeforderten zuerst
    /// (für sichtbare Zeilen: sonst warten sie hinter Zehntausenden Titeln einer Suche).
    pub fn request_meta_now(&self, path: PathBuf) {
        self.urgent.lock().unwrap_or_else(|e| e.into_inner()).push(path);
        let _ = self.urgent_tx.send(());
    }

    /// Prüft im Hintergrund, ob gemerkte Tags noch gelten (Datei mit derselben Größe und Änderungszeit).
    /// Antwort [`Response::MetaValid`] oder, bei geänderter Datei, [`Response::Meta`] mit neuen Tags.
    /// Läuft nachrangig: unbekannte Tags ([`Self::request_meta`]) werden zuerst gelesen.
    pub fn check_meta(&self, path: PathBuf, stamp: Stamp) {
        let _ = self.check_tx.send((path, stamp));
    }

    /// Lädt `path` für einen Player. `cover`: das eingebettete Cover nachreichen ([`Response::Cover`]), nur sinnvoll
    /// bei sichtbaren Plattentellern.
    pub fn load(&self, deck: usize, token: u64, path: PathBuf, cover: bool, known: Known) {
        let sketch_cache = self.sketch_cache.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let _ = self.load_tx.send(LoadJob { deck, token, path, sketch_cache, cover, known: known.meta, known_bpm: known.bpm, known_key: known.key, known_grid: known.grid });
    }

    /// Ordner für den Wellenform-Cache; `None` merkt nichts (Entwicklerläufe).
    pub fn set_sketch_cache(&self, dir: Option<PathBuf>) {
        *self.sketch_cache.lock().unwrap_or_else(|e| e.into_inner()) = dir;
    }

    /// Fordert die kleinen Wellenformen von Titeln an, Ergebnisse als [`Response::Sketch`].
    /// Aus dem Cache, sonst wird die Datei im Hintergrund dekodiert. Der erste Titel kommt zuerst dran,
    /// und die ganze Gruppe geht vor ältere Anfragen (etwa die Zeilen, die vor dem Scrollen sichtbar waren).
    pub fn request_sketches(&self, paths: Vec<PathBuf>) {
        self.sketches.jobs.lock().unwrap_or_else(|e| e.into_inner()).extend(paths.into_iter().rev());
        self.sketches.ready.notify_one();
    }

    /// Ermittelt BPM und Tonart im Hintergrund, Ergebnis als [`Response::Analysis`]. Ohne `force` gelten
    /// Angaben im Tag, und es wird nur analysiert, was dort fehlt.
    pub fn analyze(&self, path: PathBuf, force: bool) {
        let generation = self.bpm_generation.load(Ordering::Relaxed);
        let _ = self.bpm_tx.send(BpmJob { generation, path, force });
    }

    /// Verwirft alle wartenden Analyse-Aufträge. Gerade laufende Analysen liefern noch ihr Ergebnis.
    pub fn cancel_analysis(&self) {
        self.bpm_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Alle bis jetzt fertigen Ergebnisse.
    pub fn poll(&self) -> impl Iterator<Item = Response> + '_ {
        self.rx.try_iter()
    }
}

/// Lädt einen Titel für einen Player. Die Datei wird nur einmal gelesen (auf Netzlaufwerken kostete jeder Durchgang
/// Sekunden): Tags und das rohe Cover-Bild kommen aus demselben Puffer wie der Ton. Das Cover-Bild wird erst nach dem
/// Laden dekodiert (siehe [`Response::Cover`]).
/// Ergebnis von [`load`]: der Titel, dazu das rohe Cover-Bild (wird danach dekodiert) und, falls Tempo oder
/// Tonart fehlen, das Audio für die Analyse nach dem Laden.
struct LoadResult {
    loaded: Loaded,
    picture: Option<Vec<u8>>,
    analysis: Option<Arc<TrackAudio>>,
}

fn load(job: &LoadJob, send_partial: &mut dyn FnMut(Response)) -> anyhow::Result<LoadResult> {
    let t0 = std::time::Instant::now();
    let is_mp3 = job.path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    let skip_tags = is_mp3 && !job.cover && job.known.is_some();
    let bytes = if skip_tags {
        read_audio_without_id3(&job.path)?
    } else {
        std::fs::read(&job.path).with_context(|| format!("{} lässt sich nicht öffnen", job.path.display()))?
    };
    let t_read = t0.elapsed();
    let (meta, picture) = match (&job.known, skip_tags) {
        (Some(known), true) => (known.clone(), None),
        _ => read_meta_bytes(&bytes, &job.path).unwrap_or_else(|_| (TrackMeta::from_filename(&job.path), None)),
    };
    let t_meta = t0.elapsed();
    // Vorab-Titel für den Player (nicht beim Vorhören: das beginnt erst bei 30 % des Titels)
    let mut t_partial = None;
    let mut partial = |audio: TrackAudio| {
        t_partial = Some(t0.elapsed());
        send_partial(Response::Partial { deck: job.deck, token: job.token, path: job.path.clone(), meta: meta.clone(), audio: Arc::new(audio) });
    };
    let early = (job.deck != fw_engine::PREVIEW).then_some(Early { secs: PARTIAL_SECS, send: &mut partial });
    let audio = decode_file_bytes_early(bytes, &job.path, None, early)?;
    let t_decode = t0.elapsed();
    let waveform = Waveform::from_audio(&audio);
    let sketch = Arc::new(WaveSketch::from_waveform(&waveform));
    if let Some(dir) = &job.sketch_cache {
        write_cached(dir, &job.path, &sketch);
    }
    // Tempo und Tonart aus Tag oder Cache; was fehlt, kommt nach dem Laden (siehe Lade-Thread)
    let bpm = job.known_bpm.or(meta.bpm);
    let key = job.known_key.or(meta.key.map(Some));
    let missing = bpm.is_none() || key.is_none() || !job.known_grid;
    let mut meta = meta;
    meta.duration_secs = audio.duration_secs();
    let ms = |a: std::time::Duration, b: std::time::Duration| a.saturating_sub(b).as_millis();
    log::info!(
        "Laden {:.1} s ({}): lesen {} ms, Tags {} ms, spielbereit nach {}, dekodiert {} ms, Wellenform {} ms",
        t0.elapsed().as_secs_f32(),
        job.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        t_read.as_millis(),
        ms(t_meta, t_read),
        t_partial.map_or("dem Laden".to_owned(), |t| format!("{} ms", t.as_millis())),
        ms(t_decode, t_meta),
        ms(t0.elapsed(), t_decode),
    );
    // Beim Vorhören und ohne sichtbaren Plattenteller wird kein Cover gezeigt
    let picture = picture.filter(|_| job.cover && job.deck != fw_engine::PREVIEW);
    let audio = Arc::new(audio);
    // Beim Vorhören braucht es keine Analyse
    let analysis = (missing && job.deck != fw_engine::PREVIEW).then(|| Arc::clone(&audio));
    let loaded = Loaded {
        deck: job.deck,
        token: job.token,
        path: job.path.clone(),
        meta,
        audio,
        waveform,
        sketch,
        bpm,
        bpm_analyzed: false,
        key: key.flatten(),
        key_analyzed: false,
    };
    Ok(LoadResult { loaded, picture, analysis })
}

/// Skizze aus dem Cache oder neu berechnet (dann gemerkt). Das dekodierte Audio wird gleich wieder freigegeben.
fn make_sketch(path: &std::path::Path, cache: Option<&std::path::Path>) -> Option<Arc<WaveSketch>> {
    if let Some(s) = cache.and_then(|dir| read_cached(dir, path)) {
        return Some(Arc::new(s));
    }
    match decode_file(path) {
        Ok(audio) => {
            let sketch = WaveSketch::from_waveform(&Waveform::from_audio(&audio));
            if let Some(dir) = cache {
                write_cached(dir, path, &sketch);
            }
            Some(Arc::new(sketch))
        }
        Err(e) => {
            log::debug!("Wellenform von {}: {e:#}", path.display());
            None
        }
    }
}

struct Analysis {
    bpm: Option<f32>,
    bpm_analyzed: bool,
    key: Option<Camelot>,
    key_analyzed: bool,
    readable: bool,
    grid: Option<Option<BeatGrid>>,
}

/// BPM und Tonart eines Titels: aus dem Tag (ohne `force`), sonst aus der Erkennung. Dekodiert wird nur,
/// wenn einer der beiden Werte fehlt, und dann nur einmal für beide; dabei (und mit `force`) entsteht auch das
/// Beatraster.
fn analyze(job: &BpmJob) -> Analysis {
    let meta = if job.force { None } else { read_meta(&job.path).ok() };
    let (tag_bpm, tag_key) = meta.map_or((None, None), |m| (m.bpm, m.key));
    if let (Some(bpm), Some(key)) = (tag_bpm, tag_key) {
        return Analysis { bpm: Some(bpm), bpm_analyzed: false, key: Some(key), key_analyzed: false, readable: true, grid: None };
    }
    match decode_file(&job.path) {
        Ok(audio) => {
            let bpm = tag_bpm.or_else(|| estimate_bpm(&audio));
            Analysis {
                bpm,
                bpm_analyzed: tag_bpm.is_none(),
                key: tag_key.or_else(|| estimate_key(&audio)),
                key_analyzed: tag_key.is_none(),
                readable: true,
                grid: Some(estimate_grid(&audio, bpm)),
            }
        }
        Err(e) => {
            log::debug!("Analyse von {}: {e:#}", job.path.display());
            Analysis { bpm: tag_bpm, bpm_analyzed: false, key: tag_key, key_analyzed: false, readable: false, grid: None }
        }
    }
}

/// Liest die Tags einer Datei; sind sie unlesbar, gelten die Angaben aus dem Dateinamen.
fn read_tags_response(path: PathBuf) -> Response {
    let stamp = file_stamp(&path);
    let meta = match read_meta(&path) {
        Ok(m) => m,
        Err(e) => {
            log::debug!("Tags von {} nicht lesbar: {e}", path.display());
            TrackMeta::from_filename(&path)
        }
    };
    Response::Meta { path, meta, stamp }
}

/// Hintergrundarbeit (Laden, Analyse, Tags, Wellenformen, Ordnersuche) mit niedrigster Priorität (nice 19, ohne
/// Sonderrechte erlaubt; 10 reichte nicht, die Musik kratzte beim Lesen großer Ordner noch): Auf schwachen
/// Rechnern (Netbook, Atom N270) riss sonst das Dekodieren des nächsten Titels nach einer Überblendung den Ton ab.
/// Unter Linux gilt `setpriority` je Thread; anderswo bleibt es beim Standard.
fn background_priority() {
    set_nice(19);
}

/// Nice-Wert des Lade-Threads. Den Ton schützt dabei der Audio-Thread mit hoher Priorität (rtkit, app/src/audio.rs).
const LOADER_NICE: i32 = 5;

/// Setzt den Nice-Wert des aufrufenden Threads (nur Linux, dort gilt er je Thread).
fn set_nice(nice: i32) {
    #[cfg(target_os = "linux")]
    // SAFETY: setzt nur den Nice-Wert des aufrufenden Threads
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS as _, libc::gettid() as libc::id_t, nice);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = nice;
}
