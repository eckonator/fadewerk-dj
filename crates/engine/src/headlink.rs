// SPDX-License-Identifier: GPL-3.0-or-later
//! Verbindung zu einem eigenen Kopfhörergerät.
//!
//! Die Engine rechnet im Takt des Master-Geräts und schreibt das Kopfhörersignal in einen
//! Ringpuffer. Das Kopfhörergerät liest ihn in seinem eigenen Takt. Weil zwei Geräte nie exakt
//! gleich schnell laufen (und oft verschiedene Abtastraten haben), rechnet [`HeadReceiver`] um und
//! regelt dabei die Lesegeschwindigkeit minimal nach, sodass der Puffer weder leer- noch überläuft.

use rtrb::{Consumer, Producer, RingBuffer};

/// Größte Abweichung, die der Regler ausgleicht (±0,5 %). Echte Taktabweichungen liegen weit darunter.
const MAX_CORRECTION: f64 = 0.005;

pub type HeadSink = Producer<[f32; 2]>;

/// Legt die Verbindung an. `latency_frames` ist der angestrebte Pufferstand in Frames der Quelle.
pub fn head_link(src_rate: u32, dst_rate: u32, latency_frames: usize) -> (HeadSink, HeadReceiver) {
    let target = latency_frames.max(64);
    let (tx, rx) = RingBuffer::new(target * 8);
    let rx = HeadReceiver {
        rx,
        step: src_rate as f64 / dst_rate.max(1) as f64,
        frac: 0.0,
        prev: [0.0; 2],
        cur: [0.0; 2],
        target,
        primed: false,
        correction: 0.0,
    };
    (tx, rx)
}

/// Läuft im Callback des Kopfhörergeräts.
pub struct HeadReceiver {
    rx: Consumer<[f32; 2]>,
    /// Quell-Frames pro Ausgabe-Frame ohne Korrektur.
    step: f64,
    frac: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    target: usize,
    primed: bool,
    correction: f64,
}

impl HeadReceiver {
    /// Aktueller Pufferstand in Frames.
    pub fn fill(&self) -> usize {
        self.rx.slots()
    }

    /// Füllt einen verschachtelten Ausgabepuffer. Das Kopfhörersignal geht auf die ersten zwei Kanäle.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let fill = self.rx.slots();
        if !self.primed {
            // Erst loslegen, wenn genug Vorrat da ist, sonst stottert es gleich wieder
            if fill >= self.target {
                self.primed = true;
            } else {
                out.fill(0.0);
                return;
            }
        }
        // Zu voll: etwas schneller lesen, zu leer: etwas langsamer
        let err = ((fill as f64 - self.target as f64) / self.target as f64).clamp(-1.0, 1.0);
        self.correction += (err * MAX_CORRECTION - self.correction) * 0.05;
        let step = self.step * (1.0 + self.correction);

        for frame in out.chunks_mut(channels) {
            self.frac += step;
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                self.prev = self.cur;
                match self.rx.pop() {
                    Ok(f) => self.cur = f,
                    Err(_) => {
                        self.primed = false;
                        self.cur = [0.0; 2];
                    }
                }
            }
            let t = self.frac as f32;
            let l = self.prev[0] + (self.cur[0] - self.prev[0]) * t;
            let r = self.prev[1] + (self.cur[1] - self.prev[1]) * t;
            match frame.len() {
                1 => frame[0] = 0.5 * (l + r),
                _ => {
                    frame[0] = l;
                    frame[1] = r;
                    frame[2..].fill(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simuliert zwei Geräte mit verschiedenen Raten und leicht abweichenden Taktgebern über zehn Minuten.
    #[test]
    fn buffer_stays_stable_across_clock_drift() {
        let (src, dst) = (48_000u32, 44_100u32);
        let (mut tx, mut rx) = head_link(src, dst, 1024);
        let src_block = 512usize;
        let dst_block = 441usize;
        // Kopfhörergerät läuft 150 ppm zu schnell
        let src_period = src_block as f64 / src as f64;
        let dst_period = dst_block as f64 / (dst as f64 * (1.0 + 150e-6));
        let (mut t_src, mut t_dst) = (0.0f64, 0.0f64);
        let mut out = vec![0.0f32; dst_block * 2];
        let mut dropped = 0usize;
        let mut underruns_after_start = 0usize;
        let mut started = false;
        while t_src < 600.0 {
            if t_src <= t_dst {
                for _ in 0..src_block {
                    if tx.push([0.5, 0.5]).is_err() {
                        dropped += 1;
                    }
                }
                t_src += src_period;
            } else {
                rx.render(&mut out, 2);
                if rx.primed {
                    started = true;
                } else if started {
                    underruns_after_start += 1;
                }
                t_dst += dst_period;
            }
        }
        assert_eq!(dropped, 0, "Puffer ist übergelaufen");
        assert_eq!(underruns_after_start, 0, "Puffer ist leergelaufen");
        let fill = rx.fill();
        assert!(fill > 300 && fill < 3000, "Pufferstand driftet: {fill}");
    }
}
