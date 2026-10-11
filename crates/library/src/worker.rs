// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender, unbounded};
use fw_engine::TrackAudio;

use crate::bpm::estimate_bpm;
use crate::decode::decode_file;
use crate::meta::{TrackMeta, read_meta};
use crate::scan::{list_dir, walk_audio};
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
    pub bpm: Option<f32>,
}

/// Höchstzahl an Titeln pro Ordnersuche, damit ein Klick auf „Benutzerordner“ nicht ewig sucht.
pub const SCAN_LIMIT: usize = 50_000;

pub enum Response {
    Meta(PathBuf, TrackMeta),
    /// Gefundene Titel einer Ordnersuche. `done` kommt einmal am Ende, `limited` wenn die Höchstzahl erreicht ist.
    Scan { token: u64, tracks: Vec<PathBuf>, done: bool, limited: bool },
    Loaded(Box<Loaded>),
    Failed { path: PathBuf, deck: Option<usize>, error: String },
}

struct LoadJob {
    deck: usize,
    token: u64,
    path: PathBuf,
}

/// Hintergrund-Arbeiter: ein Thread liest Tags, einer dekodiert und analysiert.
/// Getrennt, damit ein Ordner voller Tag-Anfragen das Laden in einen Player nicht aufhält.
pub struct Library {
    meta_tx: Sender<PathBuf>,
    scan_tx: Sender<(u64, Vec<(PathBuf, bool)>)>,
    /// Kennung der aktuellen Ordnersuche; ältere Suchen brechen ab.
    scan_current: Arc<AtomicU64>,
    load_tx: Sender<LoadJob>,
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

        Self { meta_tx, scan_tx, scan_current, load_tx, rx }
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
        let _ = self.load_tx.send(LoadJob { deck, token, path });
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
    let bpm = meta.bpm.or_else(|| estimate_bpm(&audio));
    let mut meta = meta;
    meta.duration_secs = audio.duration_secs();
    Ok(Loaded { deck: job.deck, token: job.token, path: job.path.clone(), meta, audio: Arc::new(audio), waveform, bpm })
}
