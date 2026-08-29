// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, anyhow};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use fw_engine::TrackAudio;

/// WAV-Formatcode für MPEG-Ton (MP3 in einer WAV-Hülle).
const WAVE_FORMAT_MPEGLAYER3: u16 = 0x0055;
/// WAV-Formatcode für MPEG Layer 1/2.
const WAVE_FORMAT_MPEG: u16 = 0x0050;

/// Dekodiert eine Audiodatei vollständig in den Speicher (Stereo, f32).
/// Mono wird verdoppelt, mehr als zwei Kanäle werden auf die ersten beiden reduziert.
pub fn decode_file(path: &Path) -> anyhow::Result<TrackAudio> {
    let bytes = std::fs::read(path).with_context(|| format!("{} lässt sich nicht öffnen", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    match decode_bytes(bytes, ext.as_deref()) {
        Ok(t) => Ok(t),
        Err(e) => {
            // MP3 in einer WAV-Hülle (alte Rip-Programme): den Datenblock als MP3 dekodieren
            let bytes = std::fs::read(path)?;
            match wav_payload(&bytes) {
                Some((WAVE_FORMAT_MPEGLAYER3 | WAVE_FORMAT_MPEG, data)) => decode_bytes(data.to_vec(), Some("mp3"))
                    .with_context(|| format!("{}: MP3 in WAV-Hülle nicht lesbar", path.display())),
                _ => Err(e.context(format!("{}", path.display()))),
            }
        }
    }
}

/// Formatcode und Datenblock einer RIFF/WAVE-Datei, `None` bei anderen Dateien.
/// Ein vorangestellter ID3v2-Tag wird übersprungen (kommt bei solchen Dateien vor).
fn wav_payload(bytes: &[u8]) -> Option<(u16, &[u8])> {
    let bytes = &bytes[id3v2_len(bytes).min(bytes.len())..];
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut format = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 2 <= bytes.len() {
            format = Some(u16::from_le_bytes([bytes[body], bytes[body + 1]]));
        } else if id == b"data" {
            // Manche Dateien geben eine zu große Länge an: dann bis zum Dateiende
            let end = body.saturating_add(len).min(bytes.len());
            return Some((format?, &bytes[body..end]));
        }
        // Blöcke sind auf gerade Längen aufgefüllt
        pos = body.checked_add(len)?.checked_add(len & 1)?;
    }
    None
}

/// Länge eines ID3v2-Tags am Dateianfang (0, wenn keiner da ist).
fn id3v2_len(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return 0;
    }
    // Größe als „syncsafe“-Zahl: 4 × 7 Bit, ohne die 10 Byte Kopf
    let size = bytes[6..10].iter().fold(0usize, |acc, b| (acc << 7) | (*b as usize & 0x7f));
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

fn decode_bytes(bytes: Vec<u8>, ext: Option<&str>) -> anyhow::Result<TrackAudio> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .context("Format nicht erkannt")?;
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
            // Abgeschnittene oder am Ende beschädigte Datei: spielen, was bis dahin lesbar war
            Err(e) if !frames.is_empty() => {
                log::debug!("Lesen endet vorzeitig: {e}");
                break;
            }
            Err(e) => return Err(e.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // Ein kaputtes Paket überspringen statt den ganzen Titel zu verwerfen
            Err(Error::DecodeError(_) | Error::IoError(_)) => continue,
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
        return Err(anyhow!("keine Audiodaten"));
    }
    Ok(TrackAudio::new(sample_rate, frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend_from_slice(&(body.len() as u32).to_le_bytes());
        c.extend_from_slice(body);
        if body.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    #[test]
    fn finds_mp3_payload_in_wav_wrapper() {
        let mut fmt = WAVE_FORMAT_MPEGLAYER3.to_le_bytes().to_vec();
        fmt.resize(30, 0);
        let mut body = b"WAVE".to_vec();
        body.extend(chunk(b"fmt ", &fmt));
        body.extend(chunk(b"fact", &[0, 0, 0, 0]));
        body.extend(chunk(b"data", b"MP3DATA"));
        let file = chunk(b"RIFF", &body);
        assert_eq!(wav_payload(&file), Some((WAVE_FORMAT_MPEGLAYER3, &b"MP3DATA"[..])));
        assert_eq!(wav_payload(b"ID3\x03 kein wav"), None);
        // Mit ID3v2-Tag davor (Größe 5 als syncsafe-Zahl)
        let mut tagged = b"ID3\x03\x00\x00\x00\x00\x00\x05".to_vec();
        tagged.extend_from_slice(b"12345");
        tagged.extend_from_slice(&file);
        assert_eq!(wav_payload(&tagged), Some((WAVE_FORMAT_MPEGLAYER3, &b"MP3DATA"[..])));
    }
}
