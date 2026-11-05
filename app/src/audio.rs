// SPDX-License-Identifier: GPL-3.0-or-later
//! Audio-Ausgabe über cpal: Geräte auflisten, Stream starten, Engine darin betreiben.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

use anyhow::{Context, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use fw_engine::{Controller, Engine, HeadReceiver, HeadSink, head_link};

use crate::i18n::trf;

/// Größter Block, der pro Durchgang in den Zwischenpuffer gerechnet wird.
const MAX_FRAMES: usize = 4096;

pub struct AudioOut {
    _stream: cpal::Stream,
    pub device: String,
    /// Kennung des tatsächlich geöffneten Geräts.
    pub id: String,
    pub sample_rate: u32,
    /// Fehler des Streams seit dem Start. Hängt er (etwa ALSA über PulseAudio nach einem Aussetzer: dann
    /// `snd_pcm_avail_delay`-Fehler zu Hunderten pro Sekunde), startet die App die Ausgabe neu (`check_audio_errors`).
    pub errors: Arc<AtomicU64>,
    /// Höchste Auslastung des Audio-Threads seit dem letzten Abholen (`take_load`): Rechenzeit je Puffer
    /// geteilt durch dessen Spielzeit, als f32-Bits. Über 1 kam die Ausgabe nicht nach (Aussetzer).
    load: Arc<AtomicU32>,
}

impl AudioOut {
    pub fn take_load(&self) -> f32 {
        f32::from_bits(self.load.swap(0, Ordering::Relaxed))
    }
}

/// Zweiter Stream für ein eigenes Kopfhörergerät.
pub struct HeadOut {
    _stream: cpal::Stream,
    pub device: String,
}

/// Eingabestream des Mikrofons (läuft, solange das Mikrofon an ist).
pub struct MicIn {
    _stream: cpal::Stream,
    pub device: String,
}

/// Ein Ausgabegerät für die Auswahl in den Einstellungen.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    /// Kennung zum Wiederfinden (cpal `DeviceId` als Text).
    pub id: String,
    pub name: String,
    pub max_channels: u16,
}

fn device_name(d: &cpal::Device) -> String {
    d.description()
        .map(|desc| desc.name().to_owned())
        .or_else(|_| d.id().map(|i| i.to_string()))
        .unwrap_or_else(|_| "?".into())
}

/// Alle Ausgabegeräte des Standard-Audiosystems.
pub fn list_output_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.output_devices() else { return Vec::new() };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let max_channels = d.supported_output_configs().ok()?.map(|c| c.channels()).max().unwrap_or(2);
            Some(DeviceInfo { id, name: device_name(&d), max_channels })
        })
        .collect()
}

/// Öffnet das gewünschte (oder das Standard-) Ausgabegerät. Gelingt das nicht, läuft die Engine
/// ohne Ton weiter, damit die Oberfläche bedienbar bleibt.
pub fn start(device_id: Option<&str>, buffer_frames: Option<u32>) -> (Option<AudioOut>, Controller, Option<String>) {
    match try_start(device_id, buffer_frames) {
        Ok((out, ctl)) => (Some(out), ctl, None),
        Err(e) => {
            log::error!("Audio-Ausgabe nicht verfügbar: {e:#}");
            let (_engine, ctl) = Engine::new(44_100, 2);
            (None, ctl, Some(trf("Kein Ton: {}", &[&format!("{e:#}")])))
        }
    }
}

fn find_device(host: &cpal::Host, device_id: Option<&str>) -> Option<cpal::Device> {
    if let Some(wanted) = device_id {
        if let Some(d) = wanted.parse::<cpal::DeviceId>().ok().and_then(|id| host.device_by_id(&id)) {
            return Some(d);
        }
        log::warn!("Gespeichertes Ausgabegerät {wanted} nicht gefunden, nehme den Systemstandard");
    }
    host.default_output_device()
}

/// Puffer für PulseAudio/PipeWire, wenn in den Einstellungen „Standard“ gewählt ist (Frames je Abruf).
#[cfg(target_os = "linux")]
const PULSE_DEFAULT_FRAMES: u32 = 2048;

/// Puffer, wenn keiner eingestellt ist. Ohne Vorgabe nimmt der PulseAudio-Server (auch PipeWire) seinen eigenen
/// Standard mit bis zu 2 s Vorlauf: Ton und Oberfläche lagen dann 1–2 s auseinander (MacBook mit Debian 13).
fn default_buffer(host: &cpal::Host) -> Option<u32> {
    #[cfg(target_os = "linux")]
    if host.id() == cpal::HostId::PulseAudio {
        return Some(PULSE_DEFAULT_FRAMES);
    }
    let _ = host;
    None
}

fn try_start(device_id: Option<&str>, buffer_frames: Option<u32>) -> anyhow::Result<(AudioOut, Controller)> {
    let host = cpal::default_host();
    let buffer_frames = buffer_frames.or_else(|| default_buffer(&host));
    let device = find_device(&host, device_id).context("kein Audio-Ausgabegerät gefunden")?;
    let supported = device.default_output_config().context("Ausgabeformat nicht lesbar")?;
    let format = supported.sample_format();
    let mut config: cpal::StreamConfig = supported.into();
    if let Some(frames) = buffer_frames {
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    let name = device_name(&device);
    let id = device.id().map(|i| i.to_string()).unwrap_or_default();

    // Nicht jedes Gerät nimmt jede Puffergröße an: dann mit dem Standard noch einmal versuchen
    let errors = Arc::new(AtomicU64::new(0));
    let audio_tid = Arc::new(AtomicI32::new(0));
    let load = Arc::new(AtomicU32::new(0));
    let (stream, ctl) = match open(&device, config, format, &errors, &audio_tid, &load) {
        Ok(ok) => ok,
        Err(e) if buffer_frames.is_some() => {
            log::warn!("Puffergröße {buffer_frames:?} abgelehnt ({e:#}), nehme den Standard");
            config.buffer_size = cpal::BufferSize::Default;
            open(&device, config, format, &errors, &audio_tid, &load)?
        }
        Err(e) => return Err(e),
    };
    let sample_rate = config.sample_rate as u32;
    let channels = config.channels as usize;
    log::info!("Audio: {name}, {sample_rate} Hz, {channels} Kanäle, {format:?}, Puffer {:?}", config.buffer_size);
    raise_audio_priority(audio_tid);
    Ok((AudioOut { _stream: stream, device: name, id, sample_rate, errors, load }, ctl))
}

/// Alle Eingabegeräte (Mikrofone) des Standard-Audiosystems.
pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    devices
        .filter_map(|d| {
            let id = d.id().ok()?.to_string();
            let max_channels = d.supported_input_configs().ok()?.map(|c| c.channels()).max().unwrap_or(1);
            Some(DeviceInfo { id, name: device_name(&d), max_channels })
        })
        .collect()
}

/// Öffnet das Mikrofon (`None` = Standardeingang). Es schreibt mono in den Ringpuffer, die Engine liest
/// über den gelieferten [`HeadReceiver`] im Takt des Masters.
pub fn start_mic(device_id: Option<&str>, master_rate: u32) -> anyhow::Result<(MicIn, HeadReceiver)> {
    let host = cpal::default_host();
    let device = match device_id {
        Some(id) => id.parse::<cpal::DeviceId>().ok().and_then(|id| host.device_by_id(&id)).context("Mikrofon nicht gefunden")?,
        None => host.default_input_device().context("Kein Mikrofon gefunden")?,
    };
    let supported = device.default_input_config().context("Eingabeformat nicht lesbar")?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    // Vorrat von etwa 20 ms in Frames des Mikrofons
    let latency = (config.sample_rate as f32 * 0.02) as usize;
    let (sink, rx) = head_link(config.sample_rate, master_rate, latency);
    let channels = config.channels as usize;
    let stream = match format {
        SampleFormat::F32 => build_mic::<f32>(&device, config, sink, channels)?,
        SampleFormat::I16 => build_mic::<i16>(&device, config, sink, channels)?,
        SampleFormat::I32 => build_mic::<i32>(&device, config, sink, channels)?,
        SampleFormat::U16 => build_mic::<u16>(&device, config, sink, channels)?,
        SampleFormat::F64 => build_mic::<f64>(&device, config, sink, channels)?,
        other => bail!("Sampleformat {other:?} wird nicht unterstützt"),
    };
    stream.play().context("Mikrofon-Stream startet nicht")?;
    let name = device_name(&device);
    log::info!("Mikrofon: {name}, {} Hz, {channels} Kanäle", config.sample_rate);
    Ok((MicIn { _stream: stream, device: name }, rx))
}

fn build_mic<T>(device: &cpal::Device, config: cpal::StreamConfig, mut sink: HeadSink, channels: usize) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = channels.max(1);
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            // Alle Kanäle zu mono; ist der Puffer voll (Engine steht), gehen Frames verloren
            for frame in data.chunks(channels) {
                let m = frame.iter().map(|s| <f32 as FromSample<T>>::from_sample_(*s)).sum::<f32>() / channels as f32;
                let _ = sink.push([m, m]);
            }
        },
        |e| log::warn!("Mikrofon-Stream: {e}"),
        None,
    )?;
    Ok(stream)
}

/// Öffnet ein eigenes Kopfhörergerät. Die Engine schreibt in den gelieferten [`HeadSink`],
/// das Gerät liest in seinem eigenen Takt und gleicht Abweichungen aus.
pub fn start_head(device_id: &str, master_rate: u32, buffer_frames: Option<u32>) -> anyhow::Result<(HeadOut, HeadSink)> {
    let host = cpal::default_host();
    let device = device_id
        .parse::<cpal::DeviceId>()
        .ok()
        .and_then(|id| host.device_by_id(&id))
        .context("Kopfhörergerät nicht gefunden")?;
    let buffer_frames = buffer_frames.or_else(|| default_buffer(&host));
    let supported = device.default_output_config().context("Ausgabeformat nicht lesbar")?;
    let format = supported.sample_format();
    let mut config: cpal::StreamConfig = supported.into();
    if let Some(frames) = buffer_frames {
        config.buffer_size = cpal::BufferSize::Fixed(frames);
    }
    // Vorrat von etwa 20 ms, mindestens zwei Puffer des Master-Geräts
    let latency = ((master_rate as f32 * 0.02) as usize).max(2 * buffer_frames.unwrap_or(256) as usize);
    let (sink, rx) = head_link(master_rate, config.sample_rate, latency);
    let channels = config.channels as usize;
    let stream = match format {
        SampleFormat::F32 => build_head::<f32>(&device, config, rx, channels)?,
        SampleFormat::I16 => build_head::<i16>(&device, config, rx, channels)?,
        SampleFormat::I32 => build_head::<i32>(&device, config, rx, channels)?,
        SampleFormat::U16 => build_head::<u16>(&device, config, rx, channels)?,
        SampleFormat::F64 => build_head::<f64>(&device, config, rx, channels)?,
        other => bail!("Sampleformat {other:?} wird nicht unterstützt"),
    };
    stream.play().context("Kopfhörer-Stream startet nicht")?;
    let name = device_name(&device);
    log::info!("Kopfhörer: {name}, {} Hz, {channels} Kanäle, Vorrat {latency} Frames", config.sample_rate);
    Ok((HeadOut { _stream: stream, device: name }, sink))
}

fn build_head<T>(device: &cpal::Device, config: cpal::StreamConfig, mut rx: HeadReceiver, channels: usize) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut buf = vec![0.0f32; MAX_FRAMES * channels.max(1)];
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for out in data.chunks_mut(buf.len()) {
                let tmp = &mut buf[..out.len()];
                rx.render(tmp, channels);
                for (o, s) in out.iter_mut().zip(tmp.iter()) {
                    *o = T::from_sample(*s);
                }
            }
        },
        |e| log::warn!("Kopfhörer-Stream: {e}"),
        None,
    )?;
    Ok(stream)
}

fn open(device: &cpal::Device, config: cpal::StreamConfig, format: SampleFormat, errors: &Arc<AtomicU64>, tid: &Arc<AtomicI32>, load: &Arc<AtomicU32>) -> anyhow::Result<(cpal::Stream, Controller)> {
    let channels = config.channels as usize;
    let (engine, ctl) = Engine::new(config.sample_rate, channels);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(device, config, engine, channels, errors, tid, load)?,
        SampleFormat::I16 => build::<i16>(device, config, engine, channels, errors, tid, load)?,
        SampleFormat::I32 => build::<i32>(device, config, engine, channels, errors, tid, load)?,
        SampleFormat::U16 => build::<u16>(device, config, engine, channels, errors, tid, load)?,
        SampleFormat::F64 => build::<f64>(device, config, engine, channels, errors, tid, load)?,
        other => bail!("Sampleformat {other:?} wird nicht unterstützt"),
    };
    stream.play().context("Audio-Stream startet nicht")?;
    Ok((stream, ctl))
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mut engine: Engine, channels: usize, errors: &Arc<AtomicU64>, tid: &Arc<AtomicI32>, load: &Arc<AtomicU32>) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut buf = vec![0.0f32; MAX_FRAMES * channels.max(1)];
    let tid = Arc::clone(tid);
    let load = Arc::clone(load);
    let rate = config.sample_rate.max(1) as f32;
    let mut tid_known = false;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // Beim ersten Aufruf melden, in welchem Thread gerechnet wird (für `raise_audio_priority`)
            if !tid_known {
                tid_known = true;
                tid.store(current_tid(), Ordering::Relaxed);
            }
            let started = std::time::Instant::now();
            for out in data.chunks_mut(buf.len()) {
                let tmp = &mut buf[..out.len()];
                engine.process(tmp, channels);
                for (o, s) in out.iter_mut().zip(tmp.iter()) {
                    *o = T::from_sample(*s);
                }
            }
            // Auslastung: Rechenzeit gegen die Spielzeit dieses Puffers (positive f32 sortieren wie ihre Bits)
            let frames = data.len() / channels.max(1);
            if frames > 0 {
                let used = started.elapsed().as_secs_f32() * rate / frames as f32;
                load.fetch_max(used.to_bits(), Ordering::Relaxed);
            }
        },
        {
            let errors = Arc::clone(errors);
            // Gezählt für die App; ins Log nur der erste und dann jeder tausendste (sonst Hunderte Zeilen pro Sekunde)
            move |e| {
                let n = errors.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 || n.is_multiple_of(1000) {
                    log::warn!("Audio-Stream ({n}. Fehler): {e}");
                }
            }
        },
        None,
    )?;
    Ok(stream)
}

/// Kennung des aufrufenden Threads (Linux), sonst 0.
fn current_tid() -> i32 {
    #[cfg(target_os = "linux")]
    // SAFETY: gettid hat keine Vorbedingungen
    return unsafe { libc::gettid() };
    #[allow(unreachable_code)]
    0
}

/// Linux: Den Audio-Thread über rtkit (D-Bus) mit hoher Priorität laufen lassen (nice bis −15). Auf schwachen
/// Rechnern kratzte die Musik sonst, sobald Hintergrundarbeit (Ordner lesen, Analyse) die CPU belegte. Bewusst
/// keine Echtzeit-Priorität: Die verlangt ein CPU-Zeitlimit, bei dessen Überschreitung der Kernel das ganze Programm
/// beendet. macOS und Windows geben Audio-Threads ohnehin hohe Priorität. Läuft in einem eigenen Thread, weil der
/// Audio-Thread erst nach dem Start seine Kennung meldet und D-Bus-Aufrufe dauern können.
fn raise_audio_priority(tid: Arc<AtomicI32>) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let spawned = std::thread::Builder::new().name("fw-rtkit".into()).spawn(move || {
        let mut id = 0;
        for _ in 0..50 {
            id = tid.load(Ordering::Relaxed);
            if id != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if id == 0 {
            log::info!("Audio-Priorität: Audio-Thread meldete sich nicht");
        } else {
            #[cfg(target_os = "linux")]
            match rtkit_high_priority(id) {
                Ok(nice) => log::info!("Audio-Priorität: Thread {id} läuft mit nice {nice} (rtkit)"),
                Err(e) => log::info!("Audio-Priorität nicht erhöht: {e:#}"),
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("Audio-Priorität: Thread nicht gestartet: {e}");
    }
}

/// Fragt rtkit nach dem niedrigsten erlaubten Nice-Wert und setzt ihn für den Thread `tid`.
#[cfg(target_os = "linux")]
fn rtkit_high_priority(tid: i32) -> anyhow::Result<i32> {
    const DEST: &str = "org.freedesktop.RealtimeKit1";
    const PATH: &str = "/org/freedesktop/RealtimeKit1";
    let conn = zbus::blocking::Connection::system().context("System-D-Bus nicht erreichbar")?;
    let proxy = zbus::blocking::Proxy::new(&conn, DEST, PATH, DEST).context("rtkit nicht gefunden")?;
    let min_nice: i32 = proxy.get_property("MinNiceLevel").unwrap_or(-15);
    let nice = min_nice.max(-15);
    proxy.call_method("MakeThreadHighPriority", &(tid as u64, nice)).context("rtkit lehnt ab")?;
    Ok(nice)
}
