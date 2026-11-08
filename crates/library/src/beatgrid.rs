// SPDX-License-Identifier: GPL-3.0-or-later
//! Beatraster: genaues Tempo, Lage der Schläge und Taktbeginn über den ganzen Titel.
//!
//! Die Tempo-Erkennung ([`crate::bpm::estimate_bpm`]) liefert einen Startwert auf 0,1 BPM; für ein Raster über
//! vier Minuten ist das zu grob (0,05 BPM = ⅕ Schlag am Ende). Darum wird die Periode fein um den Startwert gesucht:
//! Die Energie-Anstiege (Onsets) des ganzen Titels werden nach ihrer Lage im Schlag in Klassen gefaltet; beim
//! richtigen Tempo sammeln sie sich in einer Klasse, die zugleich die Phase angibt.
//!
//! Titel mit schwankendem Tempo (live gespielt) oder ohne klaren Beat gelten als unsicher (`None`): lieber kein
//! Raster als ein falsches, auf das später ein Übergang baut.

use fw_engine::TrackAudio;

use crate::bpm::estimate_bpm;

const HOP: usize = 128;
/// Klassen je Schlag beim Falten (Auflösung der Phase).
const BINS: usize = 64;
/// Suchbereich um das Starttempo (±0,6 %) und Schrittweite (0,01 %).
const SEARCH: i32 = 60;
const STEP: f64 = 0.0001;
/// So viel stärker als der Durchschnitt muss die beste Klasse sein.
const MIN_PEAK: f32 = 1.8;
/// Höchste Abweichung der Phase zwischen Anfang, Mitte und Ende (Anteil eines Schlags).
const MAX_DRIFT: f64 = 0.125;
/// Grenzfrequenzen der Bänder: Bass (auch für die Taktbetonung) und Höhen.
const BASS_HZ: f32 = 150.0;
const HIGH_HZ: f32 = 4000.0;

/// Raster eines Abschnitts: Tempo und ein Taktbeginn darin (Schlag 1), von dem aus in beide Richtungen gezählt wird.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub bpm: f64,
    /// Ein Taktbeginn in Sekunden ab Titelanfang.
    pub downbeat: f64,
}

impl Segment {
    pub fn beat_secs(&self) -> f64 {
        60.0 / self.bpm
    }

    /// Lage einer Stelle in Schlägen ab dem Taktbeginn (ganzzahlig = auf dem Schlag, Vielfache von 4 = Taktbeginn).
    pub fn beats_at(&self, secs: f64) -> f64 {
        (secs - self.downbeat) / self.beat_secs()
    }
}

/// Beatraster eines Titels. Läuft er gleichmäßig, gilt ein Raster für den ganzen Titel (`start == end`, `steady`).
/// Schwankt das Tempo (Schlagzeug ohne Klick), gibt es getrennte Raster für Anfang und Ende – dort wird übergeblendet;
/// eines davon kann fehlen (etwa ein Intro ohne Schlagzeug).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatGrid {
    pub start: Option<Segment>,
    pub end: Option<Segment>,
    pub steady: bool,
}

impl BeatGrid {
    /// Raster, das an dieser Stelle gilt (vordere Hälfte: Anfang, hintere: Ende), sofern bekannt und nah genug:
    /// Bei schwankendem Tempo nur im Abschnitt selbst und kurz davor/danach.
    pub fn at(&self, secs: f64, duration: f64) -> Option<Segment> {
        if self.steady {
            return self.start;
        }
        if secs < duration / 2.0 {
            self.start.filter(|_| secs <= REGION_SECS + 10.0)
        } else {
            self.end.filter(|_| secs >= duration - REGION_SECS - 10.0)
        }
    }
}

/// Bei schwankendem Tempo: Abschnitte von `WINDOW_SECS`, alle `OFFSET_SECS` versetzt, in den ersten bzw. letzten
/// `REGION_SECS` des Titels; der deutlichste gilt.
pub const WINDOW_SECS: f64 = 30.0;
const OFFSET_SECS: f64 = 10.0;
pub const REGION_SECS: f64 = 60.0;
/// Suchbereich im Abschnitt (±3 %, das Tempo kann dort vom Durchschnitt abweichen) und Mindestschärfe. 1,7 statt
/// 2,0 seit 2026-11-08: An Live-Punk (Böhse Onkelz, 1985) gemessen passten zwei unabhängige 15-s-Hälften bei 1,7
/// genauso oft zusammen wie bei 2,0 (80 % gegen 78 %, Rest echte Tempowanderung), Raster am Ende 6 → 19 von 34.
const WINDOW_SEARCH: i32 = 150;
const WINDOW_STEP: f64 = 0.0002;
const WINDOW_MIN_PEAK: f32 = 1.7;

/// Bestimmt das Beatraster; `hint` = schon bekanntes Tempo (Tag oder Cache), sonst wird es geschätzt.
pub fn estimate_grid(audio: &TrackAudio, hint: Option<f32>) -> Option<BeatGrid> {
    let (onset, bass) = envelopes(audio);
    if onset.len() < 1000 {
        return None;
    }
    let mut bpm0 = hint.filter(|b| b.is_finite() && *b > 0.0).or_else(|| estimate_bpm(audio))? as f64;
    while bpm0 < 70.0 {
        bpm0 *= 2.0;
    }
    while bpm0 > 180.0 {
        bpm0 /= 2.0;
    }
    let rate = audio.sample_rate as f64 / HOP as f64;
    let p0 = rate * 60.0 / bpm0;
    let debug = std::env::var_os("FW_GRID_DEBUG").is_some();

    // Erst beim Tempo selbst, dann am doppelt so schnellen Puls (Achtel): Bei Rock mit treibender Gitarre oder
    // Hi-Hat ist der Achtel-Puls oft schärfer als der Schlag, dann ist jeder zweite Puls ein Schlag
    for sub in [1, 2] {
        if let Some(grid) = grid_at(&onset, &bass, rate, p0 / sub as f64, sub, debug) {
            return Some(grid);
        }
    }
    None
}

/// Raster mit Puls `pulse` (Hops), davon jeder `sub`-te ein Schlag: erst für den ganzen Titel, sonst Anfang und
/// Ende für sich.
fn grid_at(onset: &[f32], bass: &[f32], rate: f64, pulse: f64, sub: usize, debug: bool) -> Option<BeatGrid> {
    if let Some((seg, _)) = search(onset, bass, rate, pulse, sub, 0, onset.len(), SEARCH, STEP, MIN_PEAK, debug) {
        return Some(BeatGrid { start: Some(seg), end: Some(seg), steady: true });
    }
    // Je der Abschnitt in den ersten bzw. letzten `REGION_SECS`, der am nächsten am Rand liegt und sicher genug
    // ist (Intro und Ausklang haben oft wenig Schlagzeug): Bei wanderndem Tempo zählt die Nähe zur Übergangsstelle
    // mehr als die Schärfe
    let win = ((WINDOW_SECS * rate) as usize).min(onset.len() / 3);
    let region = ((REGION_SECS * rate) as usize).clamp(win, onset.len() / 2);
    let step = (OFFSET_SECS * rate) as usize;
    let best_in = |starts: Vec<usize>| {
        starts
            .into_iter()
            .find_map(|from| search(onset, bass, rate, pulse, sub, from, from + win, WINDOW_SEARCH, WINDOW_STEP, WINDOW_MIN_PEAK, debug))
            .map(|(seg, _)| seg)
    };
    let offsets: Vec<usize> = (0..).map(|k| k * step).take_while(|o| o + win <= region).collect();
    let start = best_in(offsets.clone());
    let end = best_in(offsets.iter().map(|o| onset.len() - win - o).collect());
    (start.is_some() || end.is_some()).then_some(BeatGrid { start, end, steady: false })
}

/// Raster im Bereich `from..to` (Hops): Puls mit der schärfsten Faltung um `p0` (± `range` Schritte zu `step`),
/// Phase, Prüfung auf gleichmäßiges Tempo (drei Teile) und Taktbeginn über die Bass-Betonung. Jeder `sub`-te Puls
/// ist ein Schlag; welcher, und welcher Schlag die „1“ ist, entscheidet die Betonung unter den `4 · sub` Lagen im Takt.
/// Liefert das Raster samt Schärfe der Faltung.
#[allow(clippy::too_many_arguments)]
fn search(onset: &[f32], bass: &[f32], rate: f64, p0: f64, sub: usize, from: usize, to: usize, range: i32, step: f64, min_peak: f32, debug: bool) -> Option<(Segment, f32)> {
    let mut best = (f32::MIN, p0, [0f32; BINS]);
    for k in -range..=range {
        let p = p0 * (1.0 + k as f64 * step);
        let bins = fold(onset, p, from, to);
        let peak = bins.iter().copied().fold(f32::MIN, f32::max);
        if peak > best.0 {
            best = (peak, p, bins);
        }
    }
    let (peak, p, bins) = best;
    let mean = bins.iter().sum::<f32>() / BINS as f32;
    if mean <= 0.0 || peak / mean < min_peak {
        if debug {
            eprintln!("  {from}..{to} (Puls/{sub}): Spitze nur {:.2}", peak / mean.max(1e-9));
        }
        return None;
    }
    let phase = peak_phase(&bins);
    let third = (to - from) / 3;
    let phases: Vec<f64> = (0..3).map(|k| peak_phase(&fold(onset, p, from + k * third, from + (k + 1) * third))).collect();
    let drift = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(1.0);
        d.min(1.0 - d)
    };
    if debug {
        eprintln!("  {from}..{to} (Puls/{sub}): Spitze {:.2} Phasen {phases:.3?} gesamt {phase:.3}", peak / mean);
    }
    if phases.iter().any(|&ph| drift(ph, phase) > MAX_DRIFT) {
        return None;
    }
    // Taktbeginn: unter den Lagen im Takt (4 Schläge zu je `sub` Pulsen) die mit der stärksten Bass-Betonung
    let bar = 4 * sub as i64;
    let mut accent = vec![0f32; bar as usize];
    let first_pulse = (from as f64 / p - phase).ceil() as i64;
    let mut k = first_pulse;
    loop {
        let i = ((phase + k as f64) * p).round() as usize;
        if i + 2 >= to.min(bass.len()) {
            break;
        }
        accent[k.rem_euclid(bar) as usize] += bass[i.saturating_sub(2)..=i + 2].iter().copied().fold(0.0, f32::max);
        k += 1;
    }
    let downbeat = (0..bar as usize).max_by(|&a, &b| accent[a].total_cmp(&accent[b])).unwrap_or(0) as i64;
    // Taktbeginn im Bereich (Puls mit k ≡ downbeat mod Takt ab `first_pulse`)
    let k0 = first_pulse + (downbeat - first_pulse).rem_euclid(bar);
    Some((Segment { bpm: 60.0 * rate / (p * sub as f64), downbeat: (phase + k0 as f64) * p / rate }, peak / mean))
}

/// Onset-Kurve und Bass-Kurve je Hop. Die Onsets werden in drei Bändern bestimmt (Bass unter ~150 Hz für die
/// Kick, Höhen über ~4 kHz für Snare und Hi-Hat, dazwischen der Rest), jeweils als Anstieg der Lautstärke, auf den
/// Durchschnitt des Bands normiert und addiert: In dichtem Rock-Sound geht das Schlagzeug in der Gesamtlautstärke sonst unter.
/// Der Wert an Stelle `i` gehört zum Anfang des Hops `i` (die Änderung von Hop `i − 1` auf `i`).
fn envelopes(audio: &TrackAudio) -> (Vec<f32>, Vec<f32>) {
    let sr = audio.sample_rate.max(1) as f32;
    let coef = |hz: f32| 1.0 - (-2.0 * std::f32::consts::PI * hz / sr).exp();
    let (a_low, a_high) = (coef(BASS_HZ), coef(HIGH_HZ));
    let (mut low, mut below_high) = (0f32, 0f32);
    let n = audio.frames.len() / HOP + 1;
    let mut bands = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
    for chunk in audio.frames.chunks(HOP) {
        let mut e = [0f32; 3];
        for f in chunk {
            let m = 0.5 * (f[0] + f[1]);
            low += a_low * (m - low);
            below_high += a_high * (m - below_high);
            let high = m - below_high;
            let mid = below_high - low;
            e[0] += low * low;
            e[1] += mid * mid;
            e[2] += high * high;
        }
        for (b, v) in bands.iter_mut().zip(e) {
            b.push(v.sqrt());
        }
    }
    let rise = |v: &[f32]| -> Vec<f32> { std::iter::once(0.0).chain(v.windows(2).map(|w| (w[1] - w[0]).max(0.0))).collect() };
    let rises: Vec<Vec<f32>> = bands.iter().map(|b| rise(b)).collect();
    // Jedes Band auf seinen Durchschnitt normiert, damit Kick und Hi-Hat gleich viel zählen
    let scale: Vec<f32> = rises.iter().map(|r| {
        let mean = r.iter().sum::<f32>() / r.len().max(1) as f32;
        if mean > 0.0 { 1.0 / mean } else { 0.0 }
    }).collect();
    let onset = (0..rises[0].len()).map(|i| (0..3).map(|b| rises[b][i] * scale[b]).sum()).collect();
    let bass = rises.into_iter().next().unwrap_or_default();
    (onset, bass)
}

/// Faltet die Onsets zwischen `from` und `to` (Hops) nach ihrer Lage im Schlag der Länge `p` (Hops), kreisförmig
/// leicht geglättet.
fn fold(onset: &[f32], p: f64, from: usize, to: usize) -> [f32; BINS] {
    let mut bins = [0f32; BINS];
    for (i, o) in onset.iter().enumerate().take(to).skip(from) {
        if *o > 0.0 {
            let frac = (i as f64 / p).fract();
            bins[(frac * BINS as f64) as usize % BINS] += o;
        }
    }
    let mut out = [0f32; BINS];
    for (i, v) in out.iter_mut().enumerate() {
        *v = 0.25 * bins[(i + BINS - 1) % BINS] + 0.5 * bins[i] + 0.25 * bins[(i + 1) % BINS];
    }
    out
}

/// Phase der stärksten Klasse als Anteil eines Schlags (0 bis 1), parabolisch zwischen den Klassen verfeinert.
fn peak_phase(bins: &[f32; BINS]) -> f64 {
    let m = (0..BINS).max_by(|&a, &b| bins[a].total_cmp(&bins[b])).unwrap_or(0);
    let (y0, y1, y2) = (bins[(m + BINS - 1) % BINS], bins[m], bins[(m + 1) % BINS]);
    let denom = y0 - 2.0 * y1 + y2;
    let shift = if denom.abs() > 1e-12 { (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    // Klasse m deckt [m, m+1) ab; ihre Mitte liegt bei m + 0,5
    ((m as f64 + 0.5 + shift as f64) / BINS as f64).rem_euclid(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Klick-Spur mit Bass-Betonung auf Schlag 1, der erste Taktbeginn bei `first` Sekunden.
    fn track(bpm: f64, first: f64, secs: f64, sr: u32, drift: f64) -> TrackAudio {
        let n = (secs * sr as f64) as usize;
        let mut frames = vec![[0f32; 2]; n];
        let mut t = first - 4.0 * 60.0 / bpm;
        let mut k = 0usize;
        while t < secs {
            let period = 60.0 / (bpm * (1.0 + drift * t / secs));
            if t >= 0.0 {
                let start = (t * sr as f64) as usize;
                let down = k.is_multiple_of(4);
                for j in 0..(sr as usize / 20) {
                    if start + j >= n {
                        break;
                    }
                    let env = (-(j as f32) / (sr as f32 * 0.01)).exp();
                    let tone = if down { (j as f32 * 2.0 * std::f32::consts::PI * 60.0 / sr as f32).sin() } else { (j as f32 * 0.9).sin() * 0.6 };
                    frames[start + j] = [tone * env; 2];
                }
            }
            t += period;
            k += 1;
        }
        TrackAudio::new(sr, frames)
    }

    #[test]
    fn finds_tempo_phase_and_downbeat() {
        for (bpm, first) in [(128.0, 0.37), (123.4, 1.1), (96.0, 0.05)] {
            let grid = estimate_grid(&track(bpm, first, 120.0, 22_050, 0.0), None).expect("Raster");
            assert!(grid.steady);
            let g = grid.start.unwrap();
            assert!((g.bpm - bpm).abs() < 0.02, "Tempo {bpm}: {}", g.bpm);
            let bar = 4.0 * 60.0 / bpm;
            let off = (g.downbeat - first).rem_euclid(bar);
            let off = off.min(bar - off);
            assert!(off < 0.012, "Taktbeginn {bpm}: erwartet {first}, erkannt {} ({off} s daneben)", g.downbeat);
        }
    }

    #[test]
    fn a_rough_hint_is_refined() {
        // Tag sagt 128, gespielt wird 127,7: das Raster muss trotzdem stimmen
        let g = estimate_grid(&track(127.7, 0.5, 120.0, 22_050, 0.0), Some(128.0)).expect("Raster").start.unwrap();
        assert!((g.bpm - 127.7).abs() < 0.02, "{}", g.bpm);
    }

    #[test]
    fn drifting_tempo_gets_separate_start_and_end() {
        // Wird 3 % schneller: kein Raster für den ganzen Titel, aber je eines für Anfang und Ende
        let g = estimate_grid(&track(120.0, 0.2, 120.0, 22_050, 0.03), Some(120.0)).expect("Raster");
        assert!(!g.steady);
        let (start, end) = (g.start.expect("Anfang"), g.end.expect("Ende"));
        assert!((start.bpm - 120.4).abs() < 0.6, "Anfang {}", start.bpm);
        assert!((end.bpm - 123.2).abs() < 0.6, "Ende {}", end.bpm);
        assert!(g.at(10.0, 120.0).is_some() && g.at(110.0, 120.0).is_some());
        // In der Mitte eines längeren Titels gilt keines der beiden
        assert!(g.at(150.0, 300.0).is_none());
    }

    #[test]
    fn silence_has_no_grid() {
        assert_eq!(estimate_grid(&TrackAudio::new(22_050, vec![[0.0; 2]; 22_050 * 30]), Some(120.0)), None);
    }
}

/// Prüfung an echter Musik (nur von Hand): `FW_GRID_DIR=<Ordner> cargo test --release -p fw-library beatgrid_on_folder
/// -- --ignored --nocapture` zeigt je Titel Tempo und Taktbeginn bzw. „unsicher“.
#[cfg(test)]
mod folder {
    #[test]
    #[ignore]
    fn beatgrid_on_folder() {
        let Some(dir) = std::env::var_os("FW_GRID_DIR") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        let (mut ok, mut all) = (0, 0);
        for f in files.iter().filter(|f| crate::scan::is_audio(f)) {
            let Ok(audio) = crate::decode::decode_file(f) else { continue };
            all += 1;
            let rough = crate::bpm::estimate_bpm(&audio);
            let t = std::time::Instant::now();
            let g = super::estimate_grid(&audio, rough);
            println!("  Raster {} ms für {:.0} s Musik", t.elapsed().as_millis(), audio.duration_secs());
            if g.is_some_and(|g| g.end.is_some()) {
                ok += 1;
            }
            let name = f.file_name().unwrap().to_string_lossy();
            let seg = |s: Option<super::Segment>| s.map_or("–".to_owned(), |s| format!("{:.2} BPM ab {:.2} s", s.bpm, s.downbeat));
            match g {
                Some(g) if g.steady => println!("ganz:  {}  (grob {rough:?})  {name}", seg(g.start)),
                Some(g) => println!("teils: Anfang {} | Ende {}  (grob {rough:?})  {name}", seg(g.start), seg(g.end)),
                None => println!("unsicher  (grob {rough:?})  {name}"),
            }
        }
        println!("{ok} von {all} mit Raster am Ende (für den Übergang)");
    }

    /// Wie scharf ist die Faltung in 30-s-Abschnitten mit eigenem Tempo? (Diagnose)
    #[test]
    #[ignore]
    fn beatgrid_windows() {
        let Some(dir) = std::env::var_os("FW_GRID_DIR") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files.iter().filter(|f| crate::scan::is_audio(f)) {
            let Ok(audio) = crate::decode::decode_file(f) else { continue };
            let (onset, _) = super::envelopes(&audio);
            let rate = audio.sample_rate as f64 / super::HOP as f64;
            let Some(bpm) = crate::bpm::estimate_bpm(&audio) else { continue };
            let p0 = rate * 60.0 / bpm as f64;
            let win = (30.0 * rate) as usize;
            let mut out = Vec::new();
            let mut start = 0;
            while start + win <= onset.len() {
                let mut best = (0f32, 0.0);
                for step in -150..=150 {
                    let p = p0 * (1.0 + step as f64 * 0.0001);
                    let bins = super::fold(&onset, p, start, start + win);
                    let mean = bins.iter().sum::<f32>() / 64.0;
                    let peak = bins.iter().copied().fold(0.0, f32::max) / mean.max(1e-9);
                    if peak > best.0 {
                        best = (peak, 60.0 * rate / p);
                    }
                }
                out.push(format!("{:.1}@{:.1}", best.0, best.1));
                start += win;
            }
            println!("{}  {}", out.join(" "), f.file_name().unwrap().to_string_lossy());
        }
    }
}

/// Gegenprobe für die Schwellen (nur von Hand): `FW_GRID_DIR=<Ordner> cargo test --release -p fw-library
/// halves_agreement -- --ignored --nocapture`. Zwei getrennte 15-s-Hälften am Anfang und Ende werden unabhängig
/// ausgewertet; passen Tempo und Schläge an der Grenze zusammen, war das Raster echt (Zufall träfe selten).
#[cfg(test)]
mod experiment {
    /// Unabhängige Hälften (je 15 s, ohne Überlappung): passen Tempo und Schläge an der Grenze zusammen?
    #[test]
    #[ignore]
    fn halves_agreement() {
        let Some(dir) = std::env::var_os("FW_GRID_DIR") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        let ths = [1.4f32, 1.5, 1.6, 1.7, 1.8, 2.0];
        let mut pairs = [0usize; 6];
        let mut agree = [0usize; 6];
        for f in files.iter().filter(|f| crate::scan::is_audio(f)) {
            let Ok(audio) = crate::decode::decode_file(f) else { continue };
            let (onset, bass) = super::envelopes(&audio);
            let rate = audio.sample_rate as f64 / super::HOP as f64;
            let Some(mut bpm) = crate::bpm::estimate_bpm(&audio).map(|b| b as f64) else { continue };
            while bpm < 70.0 { bpm *= 2.0 }
            while bpm > 180.0 { bpm /= 2.0 }
            let half = (15.0 * rate) as usize;
            let mut line = String::new();
            for (region, label) in [(onset.len() - 2 * half - (5.0 * rate) as usize, "E"), ((5.0 * rate) as usize, "A")] {
                for (ti, th) in ths.iter().enumerate() {
                    let p0 = rate * 60.0 / bpm;
                    let a = super::search(&onset, &bass, rate, p0, 1, region, region + half, super::WINDOW_SEARCH, super::WINDOW_STEP, *th, false).map(|x| x.0);
                    let b = super::search(&onset, &bass, rate, p0, 1, region + half, region + 2 * half, super::WINDOW_SEARCH, super::WINDOW_STEP, *th, false).map(|x| x.0);
                    if let (Some(a), Some(b)) = (a, b) {
                        pairs[ti] += 1;
                        let t = (region + half) as f64 / rate;
                        let d = (a.beats_at(t) - b.beats_at(t)).rem_euclid(1.0);
                        let d = d.min(1.0 - d);
                        let ok = d < 0.1 && (a.bpm / b.bpm - 1.0).abs() < 0.015;
                        if ok { agree[ti] += 1; }
                        if ti == 1 { line.push_str(&format!(" {label}:{:.1}/{:.1} d{:.2}{}", a.bpm, b.bpm, d, if ok { "✓" } else { "✗" })); }
                    }
                }
            }
            println!("{}{}", f.file_name().unwrap().to_string_lossy(), line);
        }
        for (ti, th) in ths.iter().enumerate() {
            println!("Schwelle {th}: Paare {}  einig {} ({:.0} %)", pairs[ti], agree[ti], 100.0 * agree[ti] as f64 / pairs[ti].max(1) as f64);
        }
    }
}
