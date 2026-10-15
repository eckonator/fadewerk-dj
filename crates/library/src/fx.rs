// SPDX-License-Identifier: GPL-3.0-or-later
//! Mitgelieferte Effekte für die Pads, beim Start synthetisiert (keine fremden Samples).
//! Reihenfolge: Air Horn, Sirene, Laser, Boom, Riser, Drop, Rewind, Bremse.

use std::f32::consts::TAU;

use fw_engine::TrackAudio;

/// Abtastrate der erzeugten Klänge.
const SR: f32 = 44_100.0;

/// Anzahl der mitgelieferten Effekte.
pub const BUILTIN_COUNT: usize = 8;

/// Erzeugt den mitgelieferten Effekt Nummer `index` (0 bis 7).
pub fn builtin(index: usize) -> TrackAudio {
    let frames = match index {
        0 => air_horn(),
        1 => siren(),
        2 => laser(),
        3 => boom(),
        4 => riser(),
        5 => drop(),
        6 => rewind(),
        _ => brake(),
    };
    TrackAudio::new(SR as u32, normalize(frames, 0.85))
}

fn secs(n: f32) -> usize {
    (n * SR) as usize
}

fn mono(v: Vec<f32>) -> Vec<[f32; 2]> {
    v.into_iter().map(|s| [s, s]).collect()
}

fn normalize(mut v: Vec<[f32; 2]>, peak: f32) -> Vec<[f32; 2]> {
    let max = v.iter().flat_map(|f| f.iter()).fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 0.0 {
        let g = peak / max;
        for f in &mut v {
            f[0] *= g;
            f[1] *= g;
        }
    }
    v
}

/// Kleiner, reproduzierbarer Zufallsgenerator für Rauschen (−1 bis 1).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// Einpoliger Tiefpass.
struct LowPass {
    y: f32,
}

impl LowPass {
    fn run(&mut self, x: f32, cutoff: f32) -> f32 {
        let a = 1.0 - (-TAU * cutoff / SR).exp();
        self.y += a * (x - self.y);
        self.y
    }
}

/// Bandbegrenzter Sägezahn aus Obertönen bis knapp unter die halbe Abtastrate.
fn saw(phase: f32, freq: f32) -> f32 {
    let mut s = 0.0;
    let mut k = 1;
    while k as f32 * freq < SR * 0.45 && k <= 24 {
        s += (phase * k as f32).sin() / k as f32;
        k += 1;
    }
    s * 0.6
}

/// Hüllkurve: Anstieg und Ausklang in Sekunden, dazwischen voll.
fn gate(t: f32, start: f32, len: f32, attack: f32, release: f32) -> f32 {
    let x = t - start;
    if x < 0.0 || x > len {
        return 0.0;
    }
    (x / attack).min(1.0) * ((len - x) / release).min(1.0)
}

/// Air Horn: drei kurze Stöße und ein langer, zwei Töne im Quartabstand, angezerrt.
fn air_horn() -> Vec<[f32; 2]> {
    let n = secs(1.55);
    let blasts = [(0.0, 0.11), (0.15, 0.11), (0.30, 0.11), (0.45, 1.05)];
    let base = 466.0;
    let voices = [base, base * 1.006, base * 1.335, base * 1.335 * 0.995];
    let mut phases = [0.0f32; 4];
    let mut lp = LowPass { y: 0.0 };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let env: f32 = blasts.iter().map(|&(s, l)| gate(t, s, l, 0.008, 0.04)).sum();
        // Jeder Stoß zieht am Anfang kurz von unten in den Ton
        let since = blasts.iter().filter(|(s, _)| t >= *s).map(|(s, _)| t - s).fold(f32::MAX, f32::min);
        let bend = 1.0 - 0.06 * (-since * 40.0).exp();
        let mut s = 0.0;
        for (f, ph) in voices.iter().zip(phases.iter_mut()) {
            let freq = f * bend;
            *ph = (*ph + TAU * freq / SR) % TAU;
            s += saw(*ph, freq);
        }
        let s = (s * 1.4).tanh();
        out.push(lp.run(s, 3200.0) * env);
    }
    mono(out)
}

/// Sirene: auf und ab heulender Ton.
fn siren() -> Vec<[f32; 2]> {
    let n = secs(2.6);
    let mut ph = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let freq = 950.0 + 450.0 * (TAU * 1.3 * t - 1.57).sin();
        ph = (ph + TAU * freq / SR) % TAU;
        // Rechteckähnlich aus ungeraden Obertönen
        let s = ph.sin() + (3.0 * ph).sin() / 3.0 + (5.0 * ph).sin() / 5.0;
        out.push(s * gate(t, 0.0, 2.6, 0.05, 0.4));
    }
    // Leichtes Echo, abwechselnd links und rechts
    let d = secs(0.09);
    (0..n)
        .map(|i| {
            let e = if i >= d { out[i - d] * 0.35 } else { 0.0 };
            [out[i] + e, out[i] - e * 0.5]
        })
        .collect()
}

/// Laser: drei schnell fallende Töne, abwechselnd links und rechts.
fn laser() -> Vec<[f32; 2]> {
    let n = secs(0.8);
    let shots = [0.0f32, 0.17, 0.34];
    let mut out = vec![[0.0f32; 2]; n];
    for (k, &start) in shots.iter().enumerate() {
        let len = 0.24;
        let mut ph = 0.0f32;
        let pan = if k % 2 == 0 { [1.0, 0.45] } else { [0.45, 1.0] };
        for (i, frame) in out.iter_mut().enumerate().skip(secs(start)).take(secs(len)) {
            let p = (i as f32 / SR - start) / len;
            let freq = 160.0 * (2600.0f32 / 160.0).powf(1.0 - p);
            ph = (ph + TAU * freq / SR) % TAU;
            let s = 0.6 * ph.sin() + 0.4 * ph.sin().signum() * 0.5;
            let env = (1.0 - p).powf(1.5) * (p * 200.0).min(1.0);
            frame[0] += s * env * pan[0];
            frame[1] += s * env * pan[1];
        }
    }
    out
}

/// Boom: tiefer Schlag mit fallender Tonhöhe und kurzem Rauschen.
fn boom() -> Vec<[f32; 2]> {
    let n = secs(2.2);
    let mut ph = 0.0f32;
    let mut noise = Noise(0x1234_5678);
    let mut lp = LowPass { y: 0.0 };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let freq = 38.0 + 85.0 * (-t * 7.0).exp();
        ph = (ph + TAU * freq / SR) % TAU;
        let body = ph.sin() * (-t * 2.0).exp();
        let hit = lp.run(noise.next(), 500.0) * (-t * 14.0).exp() * 2.5;
        out.push(((body + hit) * 1.6).tanh() * (t * 400.0).min(1.0));
    }
    mono(out)
}

/// Riser: Rauschen und Ton steigen über vier Sekunden an.
fn riser() -> Vec<[f32; 2]> {
    let total = 4.0;
    let n = secs(total);
    let mut ph = 0.0f32;
    let mut noise = Noise(0x9e37_79b9);
    let (mut lp1, mut lp2) = (LowPass { y: 0.0 }, LowPass { y: 0.0 });
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let p = t / total;
        let freq = 140.0 * (1400.0f32 / 140.0).powf(p);
        ph = (ph + TAU * freq / SR) % TAU;
        // Band aus zwei Tiefpässen: oberer folgt dem Anstieg, der untere wird abgezogen
        let x = noise.next();
        let cut = 400.0 + 7000.0 * p * p;
        let band = lp1.run(x, cut) - lp2.run(x, cut * 0.35);
        let env = p * p * ((total - t) / 0.03).min(1.0);
        out.push((0.5 * ph.sin() + 1.4 * band) * env);
    }
    mono(out)
}

/// Drop: fallender Ton mit Rauschen, klingt aus.
fn drop() -> Vec<[f32; 2]> {
    let total = 2.4;
    let n = secs(total);
    let mut ph = 0.0f32;
    let mut noise = Noise(0x2545_f491);
    let mut lp = LowPass { y: 0.0 };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let p = t / total;
        let freq = 40.0 * (1300.0f32 / 40.0).powf(1.0 - p);
        ph = (ph + TAU * freq / SR) % TAU;
        let air = lp.run(noise.next(), 200.0 + 6000.0 * (1.0 - p)) * 0.6;
        let env = (1.0 - p) * (t * 300.0).min(1.0);
        out.push((ph.sin() + air) * env);
    }
    mono(out)
}

/// Ein paar Takte selbst gebauter Musik (Kick, Hi-Hat, Akkorde, Bass) für Rewind und Bremse.
fn groove() -> Vec<[f32; 2]> {
    let bpm = 126.0;
    let beat = 60.0 / bpm;
    let total = beat * 8.0;
    let n = secs(total);
    let mut noise = Noise(0x0bad_cafe);
    let mut hp = LowPass { y: 0.0 };
    let chord = [261.6f32, 311.1, 392.0];
    let bass = [65.4f32, 65.4, 77.8, 87.3, 65.4, 65.4, 98.0, 87.3];
    let mut out = Vec::with_capacity(n);
    let mut kick_ph = 0.0f32;
    for i in 0..n {
        let t = i as f32 / SR;
        let b = (t / beat).floor();
        let tb = t - b * beat;
        // Kick auf jedem Schlag
        let kf = 45.0 + 110.0 * (-tb * 30.0).exp();
        kick_ph = if tb < 1.0 / SR { 0.0 } else { (kick_ph + TAU * kf / SR) % TAU };
        let kick = kick_ph.sin() * (-tb * 9.0).exp();
        // Hi-Hat auf der Achtel dazwischen
        let th = (t - (b + 0.5) * beat).max(-1.0);
        let x = noise.next();
        let hat = if th >= 0.0 { (x - hp.run(x, 6000.0)) * (-th * 60.0).exp() * 0.5 } else { 0.0 };
        // Akkord auf 2 und 4
        let stab = if b as i32 % 2 == 1 {
            chord.iter().map(|f| saw(TAU * f * tb, *f)).sum::<f32>() * 0.35 * (-tb * 6.0).exp()
        } else {
            0.0
        };
        let bf = bass[b as usize % bass.len()];
        let bassline = (TAU * bf * t).sin() * 0.6 * (-((tb / beat) * 2.0)).exp();
        let s = kick + hat + stab + bassline;
        out.push([s + 0.15 * stab, s - 0.15 * stab]);
    }
    out
}

/// Liest `src` mit der Geschwindigkeit `speed(t)` (Vielfaches, negativ = rückwärts) ab `start` Frames.
/// Die Lautstärke folgt der Geschwindigkeit wie beim Tonabnehmer.
fn vary_speed(src: &[[f32; 2]], start: f32, total: f32, speed: impl Fn(f32) -> f32) -> Vec<[f32; 2]> {
    let n = secs(total);
    let mut pos = start;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SR;
        let v = speed(t);
        let i0 = pos.floor().max(0.0) as usize;
        let f = pos - pos.floor();
        let a = src.get(i0).copied().unwrap_or([0.0; 2]);
        let b = src.get(i0 + 1).copied().unwrap_or([0.0; 2]);
        let amp = v.abs().min(1.0);
        out.push([(a[0] + (b[0] - a[0]) * f) * amp, (a[1] + (b[1] - a[1]) * f) * amp]);
        pos = (pos + v).max(0.0);
    }
    out
}

/// Rewind: die Musik wird schnell rückwärts gespult.
fn rewind() -> Vec<[f32; 2]> {
    let src = groove();
    let total = 1.25;
    let start = src.len() as f32 - 1.0;
    let mut out = vary_speed(&src, start, total, |t| -(1.0 + 9.0 * (t / 0.9).min(1.0).powi(2)));
    // Ausblenden am Ende
    let fade = secs(0.15);
    let len = out.len();
    for (k, f) in out[len - fade..].iter_mut().enumerate() {
        let g = 1.0 - k as f32 / fade as f32;
        f[0] *= g;
        f[1] *= g;
    }
    out
}

/// Bremse: die Platte wird per Hand angehalten, die Musik läuft tiefer und langsamer aus.
fn brake() -> Vec<[f32; 2]> {
    let src = groove();
    vary_speed(&src, 0.0, 1.6, |t| (1.0 - t / 1.4).max(0.0).powf(1.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_effect_is_audible_and_short() {
        for i in 0..BUILTIN_COUNT {
            let a = builtin(i);
            assert!(a.duration_secs() > 0.3 && a.duration_secs() < 5.0, "Effekt {i}: {} s", a.duration_secs());
            let peak = (0..a.len_frames()).map(|k| a.sample_at(k as f64)[0].abs()).fold(0.0f32, f32::max);
            assert!(peak > 0.5 && peak <= 0.86, "Effekt {i}: Spitze {peak}");
        }
    }
}
