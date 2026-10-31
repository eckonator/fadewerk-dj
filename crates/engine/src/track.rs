// SPDX-License-Identifier: GPL-3.0-or-later
/// Dekodiertes Audio eines Titels: Stereo, f32, in der Original-Abtastrate.
///
/// Die Engine liest mit beliebiger Geschwindigkeit (auch rückwärts) und
/// interpoliert dabei, deshalb wird beim Laden nicht umgerechnet.
pub struct TrackAudio {
    pub sample_rate: u32,
    pub frames: Vec<[f32; 2]>,
    /// Nur der Anfang eines Titels, der noch dekodiert wird: erwartete Gesamtlänge in Frames. Der Player spielt
    /// den Anfang schon, bis die Engine den vollständigen Titel einsetzt (`Command::Complete`).
    expected: Option<usize>,
}

impl TrackAudio {
    pub fn new(sample_rate: u32, frames: Vec<[f32; 2]>) -> Self {
        // Eine Abtastrate von 0 ergäbe Divisionen durch 0 (kaputter Dateikopf)
        Self { sample_rate: sample_rate.max(1), frames, expected: None }
    }

    /// Anfang eines Titels mit der erwarteten Gesamtlänge `expected_frames` (Schätzung, z. B. aus der Bitrate).
    pub fn partial(sample_rate: u32, frames: Vec<[f32; 2]>, expected_frames: usize) -> Self {
        let expected = expected_frames.max(frames.len());
        Self { sample_rate: sample_rate.max(1), frames, expected: Some(expected) }
    }

    /// Nur der Anfang, der Rest kommt noch.
    pub fn is_partial(&self) -> bool {
        self.expected.is_some()
    }

    /// Tatsächlich vorhandene Frames.
    pub fn len_frames(&self) -> usize {
        self.frames.len()
    }

    /// Länge des ganzen Titels in Frames (bei einem Anfang die erwartete).
    pub fn full_len_frames(&self) -> usize {
        self.expected.unwrap_or(self.frames.len())
    }

    /// Dauer des ganzen Titels (bei einem Anfang die erwartete, damit Restzeit und Auto-Fade stimmen).
    pub fn duration_secs(&self) -> f64 {
        self.full_len_frames() as f64 / self.sample_rate as f64
    }

    /// Kubische Hermite-Interpolation an einer gebrochenen Frame-Position.
    /// Außerhalb des Titels kommt Stille.
    #[inline]
    pub fn sample_at(&self, pos: f64) -> [f32; 2] {
        let base = pos.floor();
        let t = (pos - base) as f32;
        let i = base as isize;
        let get = |k: isize| -> [f32; 2] {
            if k < 0 || k as usize >= self.frames.len() {
                [0.0; 2]
            } else {
                self.frames[k as usize]
            }
        };
        let (xm1, x0, x1, x2) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        [
            hermite(xm1[0], x0[0], x1[0], x2[0], t),
            hermite(xm1[1], x0[1], x1[1], x2[1], t),
        ]
    }
}

#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_hits_samples_exactly() {
        let t = TrackAudio::new(10, vec![[0.0, 0.0], [1.0, -1.0], [0.5, 0.25], [0.0, 0.0]]);
        assert_eq!(t.sample_at(1.0), [1.0, -1.0]);
        assert_eq!(t.sample_at(2.0), [0.5, 0.25]);
        assert_eq!(t.sample_at(-5.0), [0.0, 0.0]);
        assert_eq!(t.sample_at(99.0), [0.0, 0.0]);
    }

    #[test]
    fn interpolation_is_smooth_between_samples() {
        let t = TrackAudio::new(10, vec![[0.0; 2], [1.0; 2], [1.0; 2], [0.0; 2]]);
        let mid = t.sample_at(1.5)[0];
        assert!(mid > 0.99 && mid < 1.2, "mid = {mid}");
    }
}
