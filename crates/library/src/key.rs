// SPDX-License-Identifier: GPL-3.0-or-later
//! Tonart eines Titels in Camelot-Schreibweise (1A bis 12B): Erkennung, Schreibweisen aus Tags, Nachbarschaft.
//!
//! Erkennung: Der Titel wird auf Mono und rund 11 kHz gebracht, je Fenster eine FFT. Spitzen im Spektrum
//! zwischen 55 Hz und 2 kHz werden ihrem Halbton zugeordnet (nach geschätzter Stimmung, nicht stur 440 Hz)
//! und über den Titel zu einem Chroma-Vektor (12 Tonklassen) summiert. Der wird mit den Profilen der
//! 24 Dur- und Moll-Tonarten verglichen; die beste Korrelation gewinnt.

use fw_engine::TrackAudio;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Tonart als Platz auf dem Camelot-Rad: Zahl 1 bis 12, A = Moll, B = Dur.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Camelot {
    number: u8,
    minor: bool,
}

impl Camelot {
    pub fn new(number: u8, minor: bool) -> Option<Self> {
        (1..=12).contains(&number).then_some(Self { number, minor })
    }

    /// Aus Grundton (0 = C, 1 = C♯ … 11 = H) und Tongeschlecht.
    pub fn from_pitch(pc: u8, minor: bool) -> Self {
        // Dur: C = 8B, je Quinte (7 Halbtöne) eins weiter; Moll wie seine Durparallele (a-Moll = 8A wie C-Dur)
        let major_pc = if minor { (pc + 3) % 12 } else { pc % 12 };
        Self { number: ((7 * major_pc as u32 + 7) % 12) as u8 + 1, minor }
    }

    pub fn number(self) -> u8 {
        self.number
    }

    pub fn is_minor(self) -> bool {
        self.minor
    }

    /// „8A“, „12B“.
    pub fn label(self) -> String {
        format!("{}{}", self.number, if self.minor { 'A' } else { 'B' })
    }

    /// 0 bis 23, zum Speichern und Sortieren (1A, 1B, 2A, …).
    pub fn code(self) -> u8 {
        (self.number - 1) * 2 + u8::from(!self.minor)
    }

    pub fn from_code(code: u8) -> Option<Self> {
        (code < 24).then(|| Self { number: code / 2 + 1, minor: code.is_multiple_of(2) })
    }

    /// Klingt der Titel um `semitones` Halbtöne höher (Pitch ohne Keylock): ein Halbton = sieben Plätze weiter.
    pub fn shifted(self, semitones: i32) -> Self {
        let n = (self.number as i32 - 1 + 7 * semitones).rem_euclid(12) as u8 + 1;
        Self { number: n, ..self }
    }

    /// Harmonisch verträglich nach den Camelot-Regeln: gleiche Zahl (auch A↔B) oder ±1 bei gleichem Buchstaben.
    pub fn compatible(self, other: Self) -> bool {
        if self.number == other.number {
            return true;
        }
        let d = (self.number as i32 - other.number as i32).rem_euclid(12);
        self.minor == other.minor && (d == 1 || d == 11)
    }
}

/// Liest die Tonart aus einem Tag in üblichen Schreibweisen: Camelot („8A“), Notenname („Am“, „A minor“,
/// „F#“, „Bbm“, „a-Moll“, „H-Dur“) oder Open Key („1m“, „6d“). Unbekanntes ergibt `None`.
pub fn parse_key(text: &str) -> Option<Camelot> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_lowercase();
    // Camelot und Open Key: Zahl plus ein Buchstabe
    let digits: String = lower.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() {
        let rest = lower[digits.len()..].trim();
        let n: u8 = digits.parse().ok()?;
        return match rest {
            "a" => Camelot::new(n, true),
            "b" => Camelot::new(n, false),
            // Open Key: 1d = C-Dur = 8B, 1m = a-Moll = 8A
            "m" | "d" => Camelot::new(n, rest == "m").filter(|_| (1..=12).contains(&n)).map(|c| Camelot { number: (n + 6) % 12 + 1, ..c }),
            _ => None,
        };
    }
    let mut chars = lower.chars();
    let letter = chars.next()?;
    let mut pc: i32 = match letter {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' | 'h' => 11,
        _ => return None,
    };
    let mut rest: String = chars.collect();
    // Deutsch: „b“ allein ist B♭, „h“ ist H; „es“, „as“, „is“ als Vorzeichen
    if letter == 'b' && (rest.starts_with("-moll") || rest.starts_with("-dur") || rest.starts_with(" moll") || rest.starts_with(" dur")) {
        pc = 10;
    }
    loop {
        if let Some(r) = rest.strip_prefix('#').or_else(|| rest.strip_prefix('♯')).or_else(|| rest.strip_prefix("is")) {
            pc += 1;
            rest = r.to_owned();
        } else if let Some(r) = rest.strip_prefix('♭').or_else(|| rest.strip_prefix("es")).or_else(|| (letter == 'a' || letter == 'e').then(|| rest.strip_prefix('s')).flatten()) {
            pc -= 1;
            rest = r.to_owned();
        } else if rest.starts_with('b') && !rest.starts_with("b ") {
            // „Bb“, „Eb“ (das kleine b direkt nach dem Buchstaben)
            pc -= 1;
            rest = rest[1..].to_owned();
        } else {
            break;
        }
    }
    let rest = rest.trim_start_matches(['-', ' ']).trim();
    let minor = match rest {
        "" | "maj" | "major" | "dur" => false,
        "m" | "min" | "minor" | "moll" => true,
        _ => return None,
    };
    Some(Camelot::from_pitch(pc.rem_euclid(12) as u8, minor))
}

/// Rate, auf die vor der Analyse heruntergerechnet wird (Obertöne bis 2 kHz reichen für die Tonart).
const TARGET_RATE: f64 = 11_025.0;
const FFT_SIZE: usize = 8192;
const HOP: usize = 4096;
/// Höchstens so viele Sekunden aus der Titelmitte (Intro und Outro sind oft untypisch).
const WINDOW_SECS: f64 = 150.0;
const MIN_HZ: f32 = 55.0;
/// Darunter gilt die Tonart als unsicher. Gemessen an 457 Titeln (Rock, Pop, Disco, Metal): Musik mit
/// Akkorden liegt meist über 0,6; unter 0,45 waren es Rap, Punk, Basslinien ohne Akkorde und Rauschen (0,30).
const MIN_CORRELATION: f32 = 0.45;
const MAX_HZ: f32 = 2000.0;

/// Tonart-Profile nach Temperley (Kostka-Payne), Grundton zuerst.
const MAJOR: [f32; 12] = [0.748, 0.060, 0.488, 0.082, 0.670, 0.460, 0.096, 0.715, 0.104, 0.366, 0.057, 0.400];
const MINOR: [f32; 12] = [0.712, 0.084, 0.474, 0.618, 0.049, 0.460, 0.105, 0.747, 0.404, 0.067, 0.133, 0.330];

/// Schätzt die Tonart, `None` bei zu kurzem oder kaum tonalem Material (Sprache, reines Schlagzeug).
pub fn estimate_key(audio: &TrackAudio) -> Option<Camelot> {
    let chroma = chroma(audio)?;
    best_key(&chroma)
}

/// Chroma-Vektor (Index 0 = C) aus der Titelmitte.
fn chroma(audio: &TrackAudio) -> Option<[f32; 12]> {
    let sr = audio.sample_rate as f64;
    if sr < 4000.0 {
        return None;
    }
    let step = (sr / TARGET_RATE).round().max(1.0) as usize;
    let rate = (sr / step as f64) as f32;
    let len = audio.frames.len();
    let win = ((WINDOW_SECS * sr) as usize).min(len);
    let start = (len - win) / 2;
    // Mono und herunterrechnen; der Mittelwert über `step` Werte wirkt als einfacher Tiefpass
    let mono: Vec<f32> = audio.frames[start..start + win].chunks(step).map(|c| c.iter().map(|f| 0.5 * (f[0] + f[1])).sum::<f32>() / c.len() as f32).collect();
    if mono.len() < FFT_SIZE * 4 {
        return None;
    }

    let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos()).collect();
    let bin_hz = rate / FFT_SIZE as f32;
    let (lo, hi) = ((MIN_HZ / bin_hz) as usize + 1, ((MAX_HZ / bin_hz) as usize).min(FFT_SIZE / 2 - 2));
    let mut buf = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
    let mut mag = vec![0.0f32; FFT_SIZE / 2];

    // Spitzen sammeln: (Halbtonlage als Bruchzahl, Gewicht); daraus zuerst die Stimmung, dann das Chroma
    let mut peaks: Vec<(f32, f32)> = Vec::new();
    let mut frames = 0usize;
    for pos in (0..mono.len() - FFT_SIZE).step_by(HOP) {
        for (b, (x, w)) in buf.iter_mut().zip(mono[pos..pos + FFT_SIZE].iter().zip(&window)) {
            *b = Complex::new(x * w, 0.0);
        }
        fft.process(&mut buf);
        for (m, c) in mag.iter_mut().zip(&buf) {
            *m = c.norm();
        }
        let frame_max = mag[lo..hi].iter().copied().fold(0.0f32, f32::max);
        if frame_max <= 1e-6 {
            continue;
        }
        frames += 1;
        for b in lo..hi {
            let m = mag[b];
            // Nur echte Spitzen, die nicht im Grundrauschen untergehen
            if m < frame_max * 0.05 || m <= mag[b - 1] || m < mag[b + 1] {
                continue;
            }
            // Lage zwischen den Nachbarbins genauer schätzen (Parabel)
            let (a, c) = (mag[b - 1], mag[b + 1]);
            let delta = 0.5 * (a - c) / (a - 2.0 * m + c).min(-1e-9);
            let hz = (b as f32 + delta.clamp(-0.5, 0.5)) * bin_hz;
            let semis = 12.0 * (hz / 440.0).log2();
            // Lautstärke zusammendrücken: leise Akkordtöne sollen gegen laute Bässe mitzählen
            peaks.push((semis, (m / frame_max).sqrt()));
        }
    }
    if frames < 8 || peaks.len() < 50 {
        return None;
    }

    // Stimmung: mittlere Abweichung vom Halbtonraster (als Winkel gemittelt, damit ±50 Cent nicht kippt)
    let (sx, sy) = peaks.iter().fold((0.0f32, 0.0f32), |(sx, sy), &(s, w)| {
        let phase = std::f32::consts::TAU * (s - s.round());
        (sx + w * phase.cos(), sy + w * phase.sin())
    });
    let tuning = sy.atan2(sx) / std::f32::consts::TAU;

    let mut chroma = [0.0f32; 12];
    for &(s, w) in &peaks {
        let s = s - tuning;
        let near = s.round();
        // Nahe am Halbton zählt voll, auf halbem Weg nichts
        let weight = w * (1.0 - 2.0 * (s - near).abs()).max(0.0);
        // Halbtonlage relativ zu A (440 Hz) → Tonklasse mit C = 0
        let pc = ((near as i32 + 9).rem_euclid(12)) as usize;
        chroma[pc] += weight;
    }
    Some(chroma)
}

/// Beste der 24 Tonarten nach Korrelation mit den Profilen; `None`, wenn das Chroma fast flach ist.
fn best_key(chroma: &[f32; 12]) -> Option<Camelot> {
    let mean = chroma.iter().sum::<f32>() / 12.0;
    if mean <= 0.0 {
        return None;
    }
    let spread = (chroma.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / 12.0).sqrt();
    if spread / mean < 0.05 {
        return None;
    }
    let mut best = (f32::MIN, Camelot::from_pitch(0, false));
    for tonic in 0..12u8 {
        for (profile, minor) in [(&MAJOR, false), (&MINOR, true)] {
            let r = correlation(chroma, profile, tonic as usize);
            if r > best.0 {
                best = (r, Camelot::from_pitch(tonic, minor));
            }
        }
    }
    log::debug!("Tonart {} (Korrelation {:.2})", best.1.label(), best.0);
    // Unsicher (Rap, Schlagzeug, Geräusche): lieber keine Tonart als eine falsche
    (best.0 >= MIN_CORRELATION).then_some(best.1)
}

/// Pearson-Korrelation von `chroma` mit dem um `tonic` gedrehten Profil.
fn correlation(chroma: &[f32; 12], profile: &[f32; 12], tonic: usize) -> f32 {
    let p = |i: usize| profile[(i + 12 - tonic) % 12];
    let (mc, mp) = (chroma.iter().sum::<f32>() / 12.0, profile.iter().sum::<f32>() / 12.0);
    let (mut num, mut dc, mut dp) = (0.0, 0.0, 0.0);
    for (i, &c) in chroma.iter().enumerate() {
        let (a, b) = (c - mc, p(i) - mp);
        num += a * b;
        dc += a * a;
        dp += b * b;
    }
    num / (dc * dp).sqrt().max(1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camelot_wheel_matches_the_usual_table() {
        let c = |pc, minor| Camelot::from_pitch(pc, minor).label();
        assert_eq!(c(0, false), "8B"); // C-Dur
        assert_eq!(c(9, true), "8A"); // a-Moll
        assert_eq!(c(7, false), "9B"); // G-Dur
        assert_eq!(c(5, false), "7B"); // F-Dur
        assert_eq!(c(11, false), "1B"); // H-Dur
        assert_eq!(c(4, true), "9A"); // e-Moll
        assert_eq!(c(3, false), "5B"); // Es-Dur
        for code in 0..24 {
            assert_eq!(Camelot::from_code(code).unwrap().code(), code);
        }
    }

    #[test]
    fn tag_notations_are_understood() {
        let l = |s| parse_key(s).map(|c| c.label());
        assert_eq!(l("8A"), Some("8A".into()));
        assert_eq!(l("08b"), Some("8B".into()));
        assert_eq!(l("Am"), Some("8A".into()));
        assert_eq!(l("A minor"), Some("8A".into()));
        assert_eq!(l("C"), Some("8B".into()));
        assert_eq!(l("F#m"), Some("11A".into()));
        assert_eq!(l("Gbm"), Some("11A".into()));
        assert_eq!(l("Bb"), Some("6B".into()));
        assert_eq!(l("Ebm"), Some("2A".into()));
        assert_eq!(l("a-Moll"), Some("8A".into()));
        assert_eq!(l("H-Dur"), Some("1B".into()));
        assert_eq!(l("B-Dur"), Some("6B".into()));
        assert_eq!(l("Fis-Dur"), Some("2B".into()));
        assert_eq!(l("es-Moll"), Some("2A".into()));
        assert_eq!(l("1d"), Some("8B".into()));
        assert_eq!(l("1m"), Some("8A".into()));
        assert_eq!(l("6m"), Some("1A".into()));
        assert_eq!(l("13A"), None);
        assert_eq!(l("o"), None);
        assert_eq!(l(""), None);
    }

    #[test]
    fn neighbours_and_shift() {
        let k = |s| parse_key(s).unwrap();
        assert!(k("8A").compatible(k("8B")));
        assert!(k("8A").compatible(k("9A")));
        assert!(k("1A").compatible(k("12A")));
        assert!(!k("8A").compatible(k("9B")));
        assert!(!k("8A").compatible(k("10A")));
        // C-Dur einen Halbton höher ist Cis-Dur (3B)
        assert_eq!(k("8B").shifted(1).label(), "3B");
        assert_eq!(k("8B").shifted(-1).label(), "1B");
        assert_eq!(k("8B").shifted(12), k("8B"));
    }

    /// Akkordfolge mit Obertönen in einer Tonart, leicht verstimmt.
    fn chord_track(root_hz: f32, minor: bool, detune_cents: f32) -> TrackAudio {
        let sr = 44_100u32;
        let third = if minor { 3.0 } else { 4.0 };
        // I – IV – V – I, je zwei Sekunden, dazu der Grundton im Bass
        let chords = [[0.0, third, 7.0], [5.0, 5.0 + third, 12.0], [7.0, 11.0, 14.0], [0.0, third, 7.0]];
        let tune = 2f32.powf(detune_cents / 1200.0);
        let mut frames = Vec::new();
        for _ in 0..6 {
            for chord in chords {
                for i in 0..(2 * sr) as usize {
                    let t = i as f32 / sr as f32;
                    let mut s = 0.0;
                    for (j, &st) in chord.iter().chain(std::iter::once(&(chord[0] - 12.0))).enumerate() {
                        let f = root_hz * tune * 2f32.powf(st / 12.0);
                        for h in 1..4 {
                            s += (std::f32::consts::TAU * f * h as f32 * t).sin() * 0.2 / h as f32 * if j == 3 { 1.5 } else { 1.0 };
                        }
                    }
                    frames.push([s * 0.3, s * 0.3]);
                }
            }
        }
        TrackAudio::new(sr, frames)
    }

    #[test]
    fn finds_major_and_minor_keys() {
        assert_eq!(estimate_key(&chord_track(261.63, false, 0.0)).map(|c| c.label()), Some("8B".into()), "C-Dur");
        assert_eq!(estimate_key(&chord_track(220.0, true, 0.0)).map(|c| c.label()), Some("8A".into()), "a-Moll");
        assert_eq!(estimate_key(&chord_track(196.0, false, 30.0)).map(|c| c.label()), Some("9B".into()), "G-Dur, 30 Cent zu hoch");
        assert_eq!(estimate_key(&chord_track(164.81, true, -35.0)).map(|c| c.label()), Some("9A".into()), "e-Moll, 35 Cent zu tief");
    }

    #[test]
    fn silence_and_noise_have_no_key() {
        assert_eq!(estimate_key(&TrackAudio::new(44_100, vec![[0.0; 2]; 44_100 * 20])), None);
        let mut x = 12345u32;
        let noise: Vec<[f32; 2]> = (0..44_100 * 20)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let v = (x as f32 / u32::MAX as f32 - 0.5) * 0.5;
                [v, v]
            })
            .collect();
        assert_eq!(estimate_key(&TrackAudio::new(44_100, noise)), None);
    }
}
