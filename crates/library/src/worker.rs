// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use fw_engine::TrackAudio;

use crate::bpm::estimate_bpm;
use crate::decode::decode_file;
use crate::meta::{Cover, TrackMeta, read_cover, read_meta};
use crate::scan::{list_dir, walk_audio};
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
    /// Eingebettetes Cover für den Plattenteller.
    pub cover: Option<Cover>,
}

/// Höchstzahl an Titeln pro Ordnersuche, damit ein Klick auf „Benutzerordner“ nicht ewig sucht.
pub const SCAN_LIMIT: usize = 50_000;

pub enum Response {
    Meta(PathBuf, TrackMeta),
    /// Gefundene Titel einer Ordnersuche. `done` kommt einmal am Ende, `limited` wenn die Höchstzahl erreicht ist.
    Scan { token: u64, tracks: Vec<PathBuf>, done: bool, limited: bool },
    Loaded(Box<Loaded>),
    Failed { path: PathBuf, deck: Option<usize>, error: String },
    /// Ergebnis von [`Library::analyze_bpm`]. `analyzed` = aus der Tempo-Erkennung, sonst aus dem Tag.
    /// `bpm` ist `None`, wenn die Datei nicht lesbar ist oder kein Beat erkennbar war.
    Bpm { path: PathBuf, bpm: Option<f32>, analyzed: bool },
    /// Ergebnis von [`Library::request_sketch`], `None` wenn die Datei nicht lesbar ist.
    Sketch { path: PathBuf, sketch: Option<Arc<WaveSketch>> },
}

/// Warteschlange für Wellenform-Skizzen. Die neuesten Anfragen kommen zuerst dran: das sind die Zeilen,
/// die gerade sichtbar sind, während weggescrollte warten.
#[derive(Default)]
struct SketchQueue {
    jobs: Mutex<VecDeque<PathBuf>>,
    ready: Condvar,
}

/// Threads für die BPM-Analyse. Zwei reichen: Dekodieren ist schwer, und während des Auflegens soll
/// genug Rechenzeit für Oberfläche und Laden bleiben.
const BPM_THREADS: usize = 2;

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
}

/// Hintergrund-Arbeiter: ein Thread liest Tags, einer dekodiert und analysiert.
/// Getrennt, damit ein Ordner voller Tag-Anfragen das Laden in einen Player nicht aufhält.
pub struct Library {
    meta_tx: Sender<PathBuf>,
    scan_tx: Sender<(u64, Vec<(PathBuf, bool)>)>,
    /// Kennung der aktuellen Ordnersuche; ältere Suchen brechen ab.
    scan_current: Arc<AtomicU64>,
    load_tx: Sender<LoadJob>,
    bpm_tx: Sender<BpmJob>,
    /// Aufträge älterer Generationen werden übersprungen ([`Library::cancel_bpm`]).
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
        let (load_tx, load_rx) = unbounded::<LoadJob>();
        let (tx, rx) = unbounded::<Response>();
        let (scan_tx, scan_rx) = unbounded::<(u64, Vec<(PathBuf, bool)>)>();
        let scan_current = Arc::new(AtomicU64::new(0));
        let (bpm_tx, bpm_rx) = unbounded::<BpmJob>();
        let bpm_generation = Arc::new(AtomicU64::new(0));

        for i in 0..BPM_THREADS {
            let (tx, wake, rx, generation) = (tx.clone(), wake.clone(), bpm_rx.clone(), bpm_generation.clone());
            thread::Builder::new()
                .name(format!("fw-bpm-{i}"))
                .spawn(move || {
                    for job in rx {
                        if generation.load(Ordering::Relaxed) != job.generation {
                            continue;
                        }
                        let (bpm, analyzed) = analyze(&job);
                        if tx.send(Response::Bpm { path: job.path, bpm, analyzed }).is_err() {
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
                    for (token, paths) in scan_rx {
                        if current.load(Ordering::Relaxed) != token {
                            continue;
                        }
                        let total = std::cell::Cell::new(0usize);
                        for (path, recursive) in &paths {
                            let emit = |tracks: Vec<PathBuf>| {
                                total.set(total.get() + tracks.len());
                                let _ = tx.send(Response::Scan { token, tracks, done: false, limited: false });
                                wake();
                            };
                            if *recursive {
                                walk_audio(path, || current.load(Ordering::Relaxed) == token && total.get() < SCAN_LIMIT, emit);
                            } else if let Ok(listing) = list_dir(path) {
                                // Ohne Unterordner: nur die Titel direkt in diesem Ordner
                                emit(listing.tracks);
                            }
                        }
                        if current.load(Ordering::Relaxed) == token {
                            let _ = tx.send(Response::Scan { token, tracks: Vec::new(), done: true, limited: total.get() >= SCAN_LIMIT });
                            wake();
                        }
                    }
                })
                .expect("Such-Thread startet nicht");
        }

        {
            let tx = tx.clone();
            let wake = wake.clone();
            thread::Builder::new()
                .name("fw-tags".into())
                .spawn(move || {
                    for path in meta_rx {
                        let resp = match read_meta(&path) {
                            Ok(m) => Response::Meta(path, m),
                            Err(e) => {
                                log::debug!("Tags von {} nicht lesbar: {e}", path.display());
                                Response::Meta(path.clone(), TrackMeta::from_filename(&path))
                            }
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
                for job in load_rx {
                    let resp = match load(&job) {
                        Ok(l) => Response::Loaded(Box::new(l)),
                        Err(e) => Response::Failed { path: job.path, deck: Some(job.deck), error: format!("{e:#}") },
                    };
                    if tx.send(resp).is_err() {
                        break;
                    }
                    wake();
                }
            })
            .expect("Lade-Thread startet nicht");

        Self { meta_tx, scan_tx, scan_current, load_tx, bpm_tx, bpm_generation, sketches, sketch_cache, rx }
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

    pub fn load(&self, deck: usize, token: u64, path: PathBuf) {
        let sketch_cache = self.sketch_cache.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let _ = self.load_tx.send(LoadJob { deck, token, path, sketch_cache });
    }

    /// Ordner für den Wellenform-Cache; `None` merkt nichts (Entwicklerläufe).
    pub fn set_sketch_cache(&self, dir: Option<PathBuf>) {
        *self.sketch_cache.lock().unwrap_or_else(|e| e.into_inner()) = dir;
    }

    /// Fordert die kleine Wellenform eines Titels an, Ergebnis als [`Response::Sketch`].
    /// Aus dem Cache, sonst wird die Datei im Hintergrund dekodiert. Neuere Anfragen gehen vor.
    pub fn request_sketch(&self, path: PathBuf) {
        self.sketches.jobs.lock().unwrap_or_else(|e| e.into_inner()).push_back(path);
        self.sketches.ready.notify_one();
    }

    /// Ermittelt die BPM im Hintergrund, Ergebnis als [`Response::Bpm`]. Ohne `force` gilt eine
    /// BPM-Angabe im Tag, und es wird nur analysiert, wo keine steht.
    pub fn analyze_bpm(&self, path: PathBuf, force: bool) {
        let generation = self.bpm_generation.load(Ordering::Relaxed);
        let _ = self.bpm_tx.send(BpmJob { generation, path, force });
    }

    /// Verwirft alle wartenden BPM-Aufträge. Gerade laufende Analysen liefern noch ihr Ergebnis.
    pub fn cancel_bpm(&self) {
        self.bpm_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Alle bis jetzt fertigen Ergebnisse.
    pub fn poll(&self) -> impl Iterator<Item = Response> + '_ {
        self.rx.try_iter()
    }
}

fn load(job: &LoadJob) -> anyhow::Result<Loaded> {
    let meta = read_meta(&job.path).unwrap_or_else(|_| TrackMeta::from_filename(&job.path));
    let audio = decode_file(&job.path)?;
    let waveform = Waveform::from_audio(&audio);
    let sketch = Arc::new(WaveSketch::from_waveform(&waveform));
    if let Some(dir) = &job.sketch_cache {
        write_cached(dir, &job.path, &sketch);
    }
    let bpm_analyzed = meta.bpm.is_none();
    let bpm = meta.bpm.or_else(|| estimate_bpm(&audio));
    let mut meta = meta;
    meta.duration_secs = audio.duration_secs();
    // Beim Vorhören dreht keine Platte
    let cover = if job.deck != fw_engine::PREVIEW { read_cover(&job.path) } else { None };
    Ok(Loaded { deck: job.deck, token: job.token, path: job.path.clone(), meta, audio: Arc::new(audio), waveform, sketch, bpm, bpm_analyzed, cover })
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

/// BPM eines Titels: aus dem Tag (ohne `force`) oder aus der Tempo-Erkennung. Liefert (BPM, analysiert).
fn analyze(job: &BpmJob) -> (Option<f32>, bool) {
    if !job.force
        && let Some(bpm) = read_meta(&job.path).ok().and_then(|m| m.bpm)
    {
        return (Some(bpm), false);
    }
    match decode_file(&job.path) {
        Ok(audio) => (estimate_bpm(&audio), true),
        Err(e) => {
            log::debug!("BPM-Analyse von {}: {e:#}", job.path.display());
            (None, true)
        }
    }
}
