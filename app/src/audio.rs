// SPDX-License-Identifier: GPL-3.0-or-later
//! Audio-Ausgabe über cpal: Geräte auflisten, Stream starten, Engine darin betreiben.

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
}

/// Zweiter Stream für ein eigenes Kopfhörergerät.
pub struct HeadOut {
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

fn try_start(device_id: Option<&str>, buffer_frames: Option<u32>) -> anyhow::Result<(AudioOut, Controller)> {
    let host = cpal::default_host();
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
    let (stream, ctl) = match open(&device, config, format) {
        Ok(ok) => ok,
        Err(e) if buffer_frames.is_some() => {
            log::warn!("Puffergröße {buffer_frames:?} abgelehnt ({e:#}), nehme den Standard");
            config.buffer_size = cpal::BufferSize::Default;
            open(&device, config, format)?
        }
        Err(e) => return Err(e),
    };
    let sample_rate = config.sample_rate as u32;
    let channels = config.channels as usize;
    log::info!("Audio: {name}, {sample_rate} Hz, {channels} Kanäle, {format:?}, Puffer {:?}", config.buffer_size);
    Ok((AudioOut { _stream: stream, device: name, id, sample_rate }, ctl))
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

fn open(device: &cpal::Device, config: cpal::StreamConfig, format: SampleFormat) -> anyhow::Result<(cpal::Stream, Controller)> {
    let channels = config.channels as usize;
    let (engine, ctl) = Engine::new(config.sample_rate, channels);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(device, config, engine, channels)?,
        SampleFormat::I16 => build::<i16>(device, config, engine, channels)?,
        SampleFormat::I32 => build::<i32>(device, config, engine, channels)?,
        SampleFormat::U16 => build::<u16>(device, config, engine, channels)?,
        SampleFormat::F64 => build::<f64>(device, config, engine, channels)?,
        other => bail!("Sampleformat {other:?} wird nicht unterstützt"),
    };
    stream.play().context("Audio-Stream startet nicht")?;
    Ok((stream, ctl))
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mut engine: Engine, channels: usize) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let mut buf = vec![0.0f32; MAX_FRAMES * channels.max(1)];
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for out in data.chunks_mut(buf.len()) {
                let tmp = &mut buf[..out.len()];
                engine.process(tmp, channels);
                for (o, s) in out.iter_mut().zip(tmp.iter()) {
                    *o = T::from_sample(*s);
                }
            }
        },
        |e| log::warn!("Audio-Stream: {e}"),
        None,
    )?;
    Ok(stream)
}
