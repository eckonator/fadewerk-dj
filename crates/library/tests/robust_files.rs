// SPDX-License-Identifier: GPL-3.0-or-later
//! Kaputte und ungewöhnliche Dateien: Lesen, Dekodieren, Tags, Cover, Wellenform und BPM dürfen nie abstürzen.
//! Ein Fehler ist in Ordnung (die Oberfläche zeigt ihn), ein Panic nicht.

use std::path::{Path, PathBuf};

use fw_engine::TrackAudio;
use fw_library::{Waveform, bpm::estimate_bpm, decode::decode_file, meta::read_meta, read_cover, read_tags};

/// WAV-Datei mit beliebigem Kopf (PCM): Kanäle, Rate, Bits, Nutzdaten.
fn wav(channels: u16, rate: u32, bits: u16, data: &[u8], claimed_len: Option<u32>) -> Vec<u8> {
    let block = channels * bits / 8;
    let len = claimed_len.unwrap_or(data.len() as u32);
    let mut v = Vec::new();
    v.extend(b"RIFF");
    v.extend((36 + len).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(channels.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * block as u32).to_le_bytes());
    v.extend(block.to_le_bytes());
    v.extend(bits.to_le_bytes());
    v.extend(b"data");
    v.extend(len.to_le_bytes());
    v.extend(data);
    v
}

fn noise(n: usize) -> Vec<u8> {
    let mut x = 0x1234_5678u32;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// Alle Testdateien: (Name, Inhalt).
fn cases() -> Vec<(&'static str, Vec<u8>)> {
    let pcm16 = noise(48_000 * 4);
    vec![
        ("leer.mp3", Vec::new()),
        ("leer.wav", Vec::new()),
        ("zufall.mp3", noise(50_000)),
        ("zufall.flac", noise(50_000)),
        ("zufall.ogg", noise(50_000)),
        ("zufall.m4a", noise(50_000)),
        ("text.wav", b"Das ist keine Audiodatei, sondern Text.".to_vec()),
        ("nur-kopf.wav", wav(2, 44_100, 16, &[], None)),
        ("abgeschnitten.wav", wav(2, 44_100, 16, &pcm16[..1000], Some(10_000_000))),
        ("ein-sample.wav", wav(1, 44_100, 16, &[0, 0], None)),
        ("mono-8bit.wav", wav(1, 22_050, 8, &noise(22_050), None)),
        ("stereo-96k.wav", wav(2, 96_000, 16, &noise(96_000 * 4 / 2), None)),
        ("24bit.wav", wav(2, 48_000, 24, &noise(48_000 * 6 / 4), None)),
        ("acht-kanaele.wav", wav(8, 48_000, 16, &noise(48_000 * 16 / 8), None)),
        ("null-kanaele.wav", wav(0, 44_100, 16, &noise(1000), None)),
        ("null-rate.wav", wav(2, 0, 16, &noise(1000), None)),
        ("ungerade-bits.wav", wav(2, 44_100, 13, &noise(1000), None)),
        // Gültiger WAV-Anfang, danach Müll
        ("muell-danach.wav", {
            let mut v = wav(2, 44_100, 16, &pcm16[..4000], Some(4000));
            v.extend(noise(5000));
            v
        }),
        // MP3-Rahmenkopf gefolgt von Zufall, dazu ein ID3-Kopf mit unsinniger Länge
        ("id3-kaputt.mp3", {
            let mut v = b"ID3\x04\x00\x00\x7f\x7f\x7f\x7f".to_vec();
            v.extend(noise(3000));
            v
        }),
        ("mp3-rahmen.mp3", {
            let mut v = Vec::new();
            for _ in 0..50 {
                v.extend([0xFF, 0xFB, 0x90, 0x64]);
                v.extend(noise(413));
            }
            v
        }),
    ]
}

fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fw-robust-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Führt `f` aus und meldet einen Panic als Fehler mit Dateinamen.
fn no_panic(what: &str, name: &str, f: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    std::panic::catch_unwind(f).err().map(|_| format!("{what} stürzt ab bei {name}"))
}

#[test]
fn broken_and_unusual_files_never_panic() {
    let d = dir("dateien");
    let mut failures = Vec::new();
    for (name, bytes) in cases() {
        let path = d.join(name);
        std::fs::write(&path, &bytes).unwrap();
        let p: &Path = &path;
        failures.extend(no_panic("read_meta", name, || {
            let _ = read_meta(p);
        }));
        failures.extend(no_panic("read_tags", name, || {
            let _ = read_tags(p);
        }));
        failures.extend(no_panic("read_cover", name, || {
            let _ = read_cover(p);
        }));
        failures.extend(no_panic("decode_file + Wellenform + BPM", name, || {
            if let Ok(audio) = decode_file(p) {
                assert!(audio.sample_rate > 0, "Abtastrate 0");
                let _ = Waveform::from_audio(&audio);
                let _ = estimate_bpm(&audio);
            }
        }));
    }
    let _ = std::fs::remove_dir_all(&d);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn missing_file_and_directory_give_errors() {
    let d = dir("fehlt");
    for p in [d.join("gibt-es-nicht.mp3"), d.clone()] {
        assert!(decode_file(&p).is_err());
        assert!(read_meta(&p).is_err());
        assert!(read_cover(&p).is_none());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn tiny_and_silent_audio_is_handled() {
    for frames in [0usize, 1, 2, 100] {
        for rate in [8_000u32, 44_100, 192_000] {
            let a = TrackAudio::new(rate, vec![[0.0; 2]; frames]);
            let _ = Waveform::from_audio(&a);
            assert_eq!(estimate_bpm(&a), None, "Stille hat kein Tempo ({frames} Frames, {rate} Hz)");
        }
    }
}
