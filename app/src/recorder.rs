// SPDX-License-Identifier: GPL-3.0-or-later
//! Mix aufnehmen: Die Engine liefert die Frames des Masters über einen Ringpuffer
//! ([`fw_engine::Controller::take_recording`]), ein eigener Thread `fw-record` schreibt sie als WAV (24 Bit)
//! oder MP3 (320 kbit/s, LAME). WAV-Kopf wird alle paar Sekunden nachgetragen, damit nach einem Absturz
//! eine lesbare Datei bleibt; vor der 4-GB-Grenze von WAV geht es in einer zweiten Datei weiter.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use fw_engine::Shared;
use serde::{Deserialize, Serialize};

pub type FrameRx = rtrb::Consumer<[f32; 2]>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordFormat {
    #[default]
    Wav,
    Mp3,
}

impl RecordFormat {
    fn extension(self) -> &'static str {
        match self {
            RecordFormat::Wav => "wav",
            RecordFormat::Mp3 => "mp3",
        }
    }
}

/// So oft schaut der Schreib-Thread in den Ringpuffer.
const POLL: Duration = Duration::from_millis(20);
/// So oft wird der WAV-Kopf mit der bisherigen Länge nachgetragen.
const HEADER_EVERY: Duration = Duration::from_secs(5);
/// Größte Datenmenge einer WAV-Datei, bevor eine neue begonnen wird (knapp unter 4 GiB).
const WAV_MAX_DATA: u64 = 4_000_000_000;
/// Nach dem Stopp höchstens so lange auf letzte Frames warten (falls die Engine schon weg ist).
const STOP_GRACE: Duration = Duration::from_millis(500);

/// Ordner, in den ohne eigene Wahl aufgenommen wird: Musik/FadeWerk DJ Aufnahmen.
pub fn default_dir() -> PathBuf {
    dirs::audio_dir().or_else(dirs::home_dir).unwrap_or_default().join(crate::i18n::tr("FadeWerk DJ Aufnahmen"))
}

/// Freier Dateiname „Mix 2026-10-21 20-15.wav“, bei Gleichstand mit „(2)“ usw.
fn file_name(dir: &Path, format: RecordFormat) -> PathBuf {
    let now = jiff::Zoned::now();
    let base = format!("Mix {}", now.strftime("%Y-%m-%d %H-%M"));
    let ext = format.extension();
    let mut path = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// Laufende Aufnahme. Zum Beenden [`Recording::stop`], danach [`Recording::is_done`] abwarten und [`Recording::finish`].
pub struct Recording {
    pub path: PathBuf,
    pub sample_rate: u32,
    /// Zähler der Audio-Ausgabe, zu der der Ringpuffer gehört (nach einem Gerätewechsel ist er wertlos).
    pub generation: u64,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    handle: Option<JoinHandle<(FrameRx, Result<Vec<PathBuf>>)>>,
}

impl Recording {
    /// Legt Ordner und Datei an und startet den Schreib-Thread. Scheitert die Datei, kommt der Ringpuffer zurück.
    pub fn start(dir: &Path, format: RecordFormat, shared: Arc<Shared>, mut rx: FrameRx, generation: u64) -> Result<Recording, (anyhow::Error, Option<FrameRx>)> {
        // Reste einer früheren Aufnahme verwerfen
        while rx.pop().is_ok() {}
        let sample_rate = shared.sample_rate;
        let opened = std::fs::create_dir_all(dir)
            .with_context(|| format!("Ordner {} anlegen", dir.display()))
            .map(|_| file_name(dir, format))
            .and_then(|path| Writer::create(&path, format, sample_rate).map(|w| (path, w)));
        let (path, writer) = match opened {
            Ok(x) => x,
            Err(e) => return Err((e, Some(rx))),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let (t_stop, t_frames, t_bytes, t_path) = (stop.clone(), frames.clone(), bytes.clone(), path.clone());
        let spawned = std::thread::Builder::new().name("fw-record".into()).spawn(move || {
            let result = write_loop(&mut rx, writer, &t_path, format, sample_rate, &shared, &t_stop, &t_frames, &t_bytes);
            (rx, result)
        });
        match spawned {
            Ok(handle) => Ok(Recording { path, sample_rate, generation, stop, frames, bytes, handle: Some(handle) }),
            Err(e) => Err((anyhow::anyhow!("Aufnahme-Thread: {e}"), None)),
        }
    }

    /// Bisher aufgenommene Sekunden.
    pub fn secs(&self) -> f64 {
        self.frames.load(Ordering::Relaxed) as f64 / self.sample_rate.max(1) as f64
    }

    /// Bisher geschriebene Bytes (alle Teile zusammen).
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Aufnahme beenden: der Thread schreibt den Rest und schließt die Datei.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Ist der Schreib-Thread fertig (nach dem Stopp oder wegen eines Fehlers)?
    pub fn is_done(&self) -> bool {
        self.handle.as_ref().is_none_or(|h| h.is_finished())
    }

    /// Wartet auf den Schreib-Thread. Liefert den Ringpuffer zurück und die geschriebenen Dateien.
    pub fn finish(mut self) -> (Option<FrameRx>, Result<Vec<PathBuf>>) {
        self.stop();
        match self.handle.take().map(JoinHandle::join) {
            Some(Ok((rx, result))) => (Some(rx), result),
            _ => (None, Err(anyhow::anyhow!("Aufnahme-Thread abgestürzt"))),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_loop(
    rx: &mut FrameRx,
    mut writer: Writer,
    first: &Path,
    format: RecordFormat,
    sample_rate: u32,
    shared: &Shared,
    stop: &AtomicBool,
    frames: &AtomicU64,
    bytes: &AtomicU64,
) -> Result<Vec<PathBuf>> {
    let mut files = vec![first.to_path_buf()];
    let mut buf: Vec<[f32; 2]> = Vec::with_capacity(sample_rate as usize);
    let mut last_header = Instant::now();
    let mut stopped_at: Option<Instant> = None;
    let mut done_bytes = 0u64;
    loop {
        buf.clear();
        while let Ok(f) = rx.pop() {
            buf.push(f);
        }
        if !buf.is_empty() {
            if writer.data_bytes() >= WAV_MAX_DATA && format == RecordFormat::Wav {
                // Neue Datei „… Teil 2.wav“: WAV kann nicht mehr als 4 GiB
                done_bytes += writer.file_bytes();
                writer.finish()?;
                let next = part_path(first, files.len() + 1);
                writer = Writer::create(&next, format, sample_rate)?;
                files.push(next);
            }
            writer.write(&buf)?;
            frames.fetch_add(buf.len() as u64, Ordering::Relaxed);
            bytes.store(done_bytes + writer.file_bytes(), Ordering::Relaxed);
        }
        if last_header.elapsed() >= HEADER_EVERY {
            writer.checkpoint()?;
            last_header = Instant::now();
        }
        if stop.load(Ordering::Relaxed) {
            let since = *stopped_at.get_or_insert_with(Instant::now);
            // Erst aufhören, wenn die Engine nichts mehr schickt und alles gelesen ist
            if buf.is_empty() && (!shared.is_recording() || since.elapsed() > STOP_GRACE) {
                break;
            }
        }
        if buf.is_empty() {
            std::thread::sleep(POLL);
        }
    }
    writer.finish()?;
    Ok(files)
}

fn part_path(first: &Path, n: usize) -> PathBuf {
    let stem = first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = first.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    first.with_file_name(format!("{stem} {} {n}.{ext}", crate::i18n::tr("Teil")))
}

enum Writer {
    Wav { out: BufWriter<File>, data: u64, sample_rate: u32 },
    Mp3 { out: BufWriter<File>, enc: Box<mp3lame_encoder::Encoder>, mp3: Vec<u8>, pcm: Vec<f32>, written: u64 },
}

impl Writer {
    fn create(path: &Path, format: RecordFormat, sample_rate: u32) -> Result<Writer> {
        let file = File::create(path).with_context(|| format!("{} anlegen", path.display()))?;
        let mut out = BufWriter::with_capacity(1 << 16, file);
        match format {
            RecordFormat::Wav => {
                out.write_all(&wav_header(sample_rate, 0))?;
                Ok(Writer::Wav { out, data: 0, sample_rate })
            }
            RecordFormat::Mp3 => {
                use mp3lame_encoder::{Bitrate, Builder, Mode, Quality, VbrMode};
                let fail = |e: mp3lame_encoder::BuildError| anyhow::anyhow!("MP3-Encoder: {e}");
                let mut b = Builder::new().ok_or_else(|| anyhow::anyhow!("MP3-Encoder ließ sich nicht anlegen"))?;
                b.set_num_channels(2).map_err(fail)?;
                b.set_sample_rate(sample_rate).map_err(fail)?;
                // MP3 kennt höchstens 48 kHz; LAME rechnet höhere Raten selbst herunter
                if sample_rate > 48_000 {
                    b.set_output_sample_rate(std::num::NonZeroU32::new(48_000)).map_err(fail)?;
                }
                b.set_brate(Bitrate::Kbps320).map_err(fail)?;
                b.set_vbr_mode(VbrMode::Off).map_err(fail)?;
                b.set_mode(Mode::JointStereo).map_err(fail)?;
                b.set_quality(Quality::NearBest).map_err(fail)?;
                b.set_to_write_vbr_tag(false).map_err(fail)?;
                let enc = b.build().map_err(fail)?;
                Ok(Writer::Mp3 { out, enc: Box::new(enc), mp3: Vec::new(), pcm: Vec::new(), written: 0 })
            }
        }
    }

    fn write(&mut self, frames: &[[f32; 2]]) -> Result<()> {
        match self {
            Writer::Wav { out, data, .. } => {
                for f in frames {
                    for s in f {
                        let v = (s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                        out.write_all(&v.to_le_bytes()[..3])?;
                    }
                }
                *data += frames.len() as u64 * 6;
            }
            Writer::Mp3 { out, enc, mp3, pcm, written } => {
                pcm.clear();
                pcm.extend(frames.iter().flat_map(|f| [f[0].clamp(-1.0, 1.0), f[1].clamp(-1.0, 1.0)]));
                mp3.clear();
                mp3.reserve(mp3lame_encoder::max_required_buffer_size(pcm.len()));
                enc.encode_to_vec(mp3lame_encoder::InterleavedPcm(pcm.as_slice()), mp3).map_err(|e| anyhow::anyhow!("MP3-Encoder: {e}"))?;
                out.write_all(mp3)?;
                *written += mp3.len() as u64;
            }
        }
        Ok(())
    }

    fn data_bytes(&self) -> u64 {
        match self {
            Writer::Wav { data, .. } => *data,
            Writer::Mp3 { written, .. } => *written,
        }
    }

    fn file_bytes(&self) -> u64 {
        match self {
            Writer::Wav { data, .. } => 44 + data,
            Writer::Mp3 { written, .. } => *written,
        }
    }

    /// Trägt die bisherige Länge in den WAV-Kopf ein, damit die Datei auch nach einem Absturz lesbar ist.
    fn checkpoint(&mut self) -> Result<()> {
        match self {
            Writer::Wav { out, data, sample_rate } => {
                out.flush()?;
                let file = out.get_mut();
                file.seek(SeekFrom::Start(0))?;
                file.write_all(&wav_header(*sample_rate, *data))?;
                file.seek(SeekFrom::End(0))?;
            }
            Writer::Mp3 { out, .. } => out.flush()?,
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        if let Writer::Mp3 { out, enc, mp3, .. } = &mut self {
            mp3.clear();
            mp3.reserve(7200);
            enc.flush_to_vec::<mp3lame_encoder::FlushNoGap>(mp3).map_err(|e| anyhow::anyhow!("MP3-Encoder: {e}"))?;
            out.write_all(mp3)?;
        }
        self.checkpoint()?;
        let (Writer::Wav { out, .. } | Writer::Mp3 { out, .. }) = self;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    }
}

/// 44 Byte WAV-Kopf: PCM, Stereo, 24 Bit, `data` Bytes Nutzdaten.
fn wav_header(sample_rate: u32, data: u64) -> [u8; 44] {
    let data = data.min(u32::MAX as u64 - 36) as u32;
    let block_align: u16 = 2 * 3;
    let mut h = [0u8; 44];
    let mut put = |at: usize, b: &[u8]| h[at..at + b.len()].copy_from_slice(b);
    put(0, b"RIFF");
    put(4, &(36 + data).to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes());
    put(22, &2u16.to_le_bytes());
    put(24, &sample_rate.to_le_bytes());
    put(28, &(sample_rate * block_align as u32).to_le_bytes());
    put(32, &block_align.to_le_bytes());
    put(34, &24u16.to_le_bytes());
    put(36, b"data");
    put(40, &data.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use fw_engine::{Command, Engine, Motor, TrackAudio};

    fn record(format: RecordFormat) -> (PathBuf, u64) {
        let dir = std::env::temp_dir().join(format!("fw-record-test-{}-{:?}", std::process::id(), format));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut engine, mut ctl) = Engine::new(44_100, 2);
        let rx = ctl.take_recording().unwrap();
        let tone: Vec<[f32; 2]> = (0..44_100 * 3).map(|i| {
            let v = (i as f32 * 0.03).sin() * 0.5;
            [v, v]
        }).collect();
        ctl.send(Command::SetMotor(Motor::INSTANT));
        ctl.send(Command::Load { deck: 0, id: 1, track: Arc::new(TrackAudio::new(44_100, tone)), keep_playing: false, start_secs: 0.0 });
        ctl.send(Command::Play { deck: 0 });
        ctl.send(Command::SetRecording(true));
        let rec = Recording::start(&dir, format, ctl.shared.clone(), rx, 0).map_err(|(e, _)| e).unwrap();
        let mut out = vec![0.0f32; 512 * 2];
        // Eine Sekunde in Echtzeit-Häppchen, damit der Schreib-Thread mitliest
        for _ in 0..(44_100 / 512) {
            engine.process(&mut out, 2);
            std::thread::sleep(Duration::from_millis(2));
        }
        ctl.send(Command::SetRecording(false));
        rec.stop();
        engine.process(&mut out, 2);
        // Erst lesen, wenn der Thread alles aus dem Ringpuffer geschrieben hat
        while !rec.is_done() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let secs = rec.secs();
        let (back, files) = rec.finish();
        assert!(back.is_some(), "Ringpuffer kommt zurück");
        let files = files.unwrap();
        assert_eq!(files.len(), 1);
        assert!(secs > 0.9, "{secs}");
        let len = std::fs::metadata(&files[0]).unwrap().len();
        (files[0].clone(), len)
    }

    #[test]
    fn writes_valid_wav() {
        let (path, len) = record(RecordFormat::Wav);
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[0..4], b"RIFF");
        let data_len = u32::from_le_bytes(data[40..44].try_into().unwrap()) as u64;
        assert_eq!(data_len + 44, len, "Kopf passt zur Dateilänge");
        assert_eq!(data_len % 6, 0);
        // Lässt sich mit unserem eigenen Dekoder lesen
        let decoded = fw_library::decode::decode_file(&path).expect("WAV lesbar");
        assert!(decoded.duration_secs() > 0.9);
    }

    #[test]
    fn writes_decodable_mp3() {
        let (path, len) = record(RecordFormat::Mp3);
        // 320 kbit/s für gut eine Sekunde ≈ 40 KB
        assert!(len > 30_000, "{len}");
        let decoded = fw_library::decode::decode_file(&path).expect("MP3 lesbar");
        assert!(decoded.duration_secs() > 0.9);
    }
}
