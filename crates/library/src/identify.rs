// SPDX-License-Identifier: GPL-3.0-or-later
//! Akustischer Fingerabdruck für die Titelerkennung über AcoustID (Chromaprint-Verfahren).
//!
//! Gerechnet wird lokal aus den ersten zwei Minuten; nach außen geht nur der komprimierte Abdruck.

use std::path::Path;

use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};

/// So viel vom Titelanfang fließt in den Abdruck (wie `fpcalc`).
pub const FINGERPRINT_SECS: f64 = 120.0;

/// Fingerabdruck in der Form, die die AcoustID-Schnittstelle erwartet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// Komprimiert, Base64 (URL-sicher, ohne Auffüllung).
    pub encoded: String,
    /// Länge des ganzen Titels in Sekunden.
    pub duration_secs: u32,
}

/// Dekodiert den Titelanfang und berechnet den Abdruck. Die Länge stammt aus den Dateiangaben,
/// sonst aus dem Dekodierten.
pub fn fingerprint(path: &Path) -> anyhow::Result<Fingerprint> {
    let audio = crate::decode::decode_file_head(path, Some(FINGERPRINT_SECS))?;
    let config = Configuration::preset_test2();
    let mut printer = Fingerprinter::new(&config);
    printer.start(audio.sample_rate, 2).map_err(|e| anyhow::anyhow!("Fingerabdruck: {e:?}"))?;
    let mut buf = Vec::with_capacity(8192);
    for chunk in audio.frames.chunks(4096) {
        buf.clear();
        buf.extend(chunk.iter().flat_map(|f| f.map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)));
        printer.consume(&buf);
    }
    printer.finish();
    anyhow::ensure!(!printer.fingerprint().is_empty(), "Titel zu kurz für einen Fingerabdruck");
    let compressed = FingerprintCompressor::from(&config).compress(printer.fingerprint());
    let duration = lofty::read_from_path(path)
        .map(|f| lofty::prelude::AudioFile::properties(&f).duration().as_secs_f64())
        .ok()
        .filter(|d| *d > 0.0)
        .unwrap_or_else(|| audio.duration_secs());
    Ok(Fingerprint { encoded: base64_url(&compressed), duration_secs: duration.round() as u32 })
}

/// Base64 mit dem URL-Alphabet und ohne `=` am Ende (so wie `fpcalc` ausgibt).
fn base64_url(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (*b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_without_padding() {
        assert_eq!(base64_url(b""), "");
        assert_eq!(base64_url(b"f"), "Zg");
        assert_eq!(base64_url(b"fo"), "Zm8");
        assert_eq!(base64_url(b"foo"), "Zm9v");
        assert_eq!(base64_url(b"foob"), "Zm9vYg");
        assert_eq!(base64_url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn same_audio_same_fingerprint() {
        // 20 s Akkordfolge als WAV, selbst erzeugt
        let dir = std::env::temp_dir().join(format!("fw-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("akkorde.wav");
        let rate = 22_050u32;
        let samples: Vec<i16> = (0..rate * 20)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let base = [220.0, 261.6, 196.0, 293.7][(t / 2.5) as usize % 4];
                let v = [1.0, 1.25, 1.5].iter().map(|m| (t * base * m * std::f32::consts::TAU).sin()).sum::<f32>() / 4.0;
                (v * 20_000.0) as i16
            })
            .collect();
        let mut wav = Vec::new();
        let data_len = samples.len() as u32 * 2;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(&path, wav).unwrap();
        let a = fingerprint(&path).unwrap();
        let b = fingerprint(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(a, b);
        assert_eq!(a.duration_secs, 20);
        // Chromaprint-Abdrücke beginnen mit der Kennung des Verfahrens (1 = „test2“) → „AQ“
        assert!(a.encoded.starts_with("AQ"), "{}", a.encoded);
        assert!(a.encoded.len() > 50);
    }
}

/// Gibt den Abdruck einer echten Datei aus: `FW_TEST_FILE=… cargo test -p fw-library print_fingerprint -- --ignored --nocapture`
#[test]
#[ignore]
fn print_fingerprint() {
    let path = std::env::var("FW_TEST_FILE").expect("FW_TEST_FILE");
    let fp = fingerprint(Path::new(&path)).unwrap();
    println!("DURATION={}\nFINGERPRINT={}", fp.duration_secs, fp.encoded);
}
