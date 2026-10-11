// SPDX-License-Identifier: GPL-3.0-or-later
use std::fs::File;
use std::path::Path;

use anyhow::{Context, anyhow};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use fw_engine::TrackAudio;

/// Dekodiert eine Audiodatei vollständig in den Speicher (Stereo, f32).
/// Mono wird verdoppelt, mehr als zwei Kanäle werden auf die ersten beiden reduziert.
pub fn decode_file(path: &Path) -> anyhow::Result<TrackAudio> {
    let file = File::open(path).with_context(|| format!("{} lässt sich nicht öffnen", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .with_context(|| format!("{}: Format nicht erkannt", path.display()))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("keine Audiospur"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("Audiospur ohne Codec-Angaben"))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .context("Codec wird nicht unterstützt")?;

    let mut frames: Vec<[f32; 2]> = Vec::new();
    let mut sample_rate = params.sample_rate.unwrap_or(44_100);
    let mut tmp: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // Ein kaputtes Paket überspringen statt den ganzen Titel zu verwerfen
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = buf.spec();
        sample_rate = spec.rate();
        let ch = spec.channels().count().max(1);
        buf.copy_to_vec_interleaved(&mut tmp);
        frames.reserve(tmp.len() / ch);
        for f in tmp.chunks_exact(ch) {
            frames.push(if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
        }
    }

    if frames.is_empty() {
        return Err(anyhow!("{}: keine Audiodaten", path.display()));
    }
    Ok(TrackAudio::new(sample_rate, frames))
}
