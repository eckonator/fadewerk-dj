// SPDX-License-Identifier: GPL-3.0-or-later
use std::f32::consts::FRAC_PI_2;

/// Stellen der Stützpunkte einer eigenen Blendkurve (Anteil des Crossfader-Wegs von A nach B).
pub const CURVE_KNOTS: [f32; 4] = [0.2, 0.4, 0.6, 0.8];

/// Blendkurve des Crossfaders.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Curve {
    /// Gleiche Leistung: in der Mitte kein Lautstärkeloch.
    #[default]
    Soft,
    Linear,
    /// Schnitt: schon nach kurzem Weg voll offen, zum Scratchen.
    Cut,
    /// Eigene Kurve: Pegel von A an den Stellen [`CURVE_KNOTS`]. B ist das Spiegelbild,
    /// damit A genau so ausblendet, wie B einblendet.
    Custom([f32; 4]),
}

impl Curve {
    /// Pegel für A und B bei Crossfader-Stellung `x` von -1 (A) bis +1 (B).
    pub fn gains(self, x: f32) -> (f32, f32) {
        let p = ((x + 1.0) * 0.5).clamp(0.0, 1.0);
        match self {
            Curve::Soft => ((p * FRAC_PI_2).cos(), (p * FRAC_PI_2).sin()),
            Curve::Linear => (1.0 - p, p),
            Curve::Cut => (((1.0 - p) / 0.18).min(1.0), (p / 0.18).min(1.0)),
            Curve::Custom(pts) => (custom_gain(&pts, p), custom_gain(&pts, 1.0 - p)),
        }
    }

    /// Pegel von A an den Stützstellen, als Ausgangspunkt für eine eigene Kurve.
    pub fn points(self) -> [f32; 4] {
        match self {
            Curve::Custom(pts) => pts,
            c => CURVE_KNOTS.map(|k| c.gains(k * 2.0 - 1.0).0),
        }
    }
}

/// Monotone kubische Interpolation (Fritsch-Carlson) durch (0,1), die Stützpunkte und (1,0).
/// Bleibt fallend und schwingt nicht über, egal wie die Punkte gezogen werden.
fn custom_gain(pts: &[f32; 4], p: f32) -> f32 {
    let xs = [0.0, CURVE_KNOTS[0], CURVE_KNOTS[1], CURVE_KNOTS[2], CURVE_KNOTS[3], 1.0];
    let ys = [1.0, pts[0], pts[1], pts[2], pts[3], 0.0];
    // Feste Arrays: läuft im Audio-Thread, dort wird nichts alloziert
    const N: usize = 6;
    let n = N;
    let d: [f32; N - 1] = std::array::from_fn(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]));
    let mut m = [0.0f32; N];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) / 2.0 };
    }
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let a = m[k] / d[k];
            let b = m[k + 1] / d[k];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[k] = t * a * d[k];
                m[k + 1] = t * b * d[k];
            }
        }
    }
    let k = xs.windows(2).position(|w| p <= w[1]).unwrap_or(n - 2);
    let h = xs[k + 1] - xs[k];
    let t = ((p - xs[k]) / h).clamp(0.0, 1.0);
    let (t2, t3) = (t * t, t * t * t);
    let v = (2.0 * t3 - 3.0 * t2 + 1.0) * ys[k] + (t3 - 2.0 * t2 + t) * h * m[k] + (-2.0 * t3 + 3.0 * t2) * ys[k + 1] + (t3 - t2) * h * m[k + 1];
    v.clamp(0.0, 1.0)
}

/// Wohin das Kopfhörersignal geht.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HeadMode {
    /// Kein Kopfhörersignal, Master auf allen Kanälen.
    #[default]
    Off,
    /// Eine Stereo-Soundkarte: links Master (mono), rechts Kopfhörer (mono).
    Split,
    /// Nur das Kopfhörersignal, zum Ausprobieren ohne zweiten Ausgang.
    Solo,
    /// Mehrkanal-Interface: Master auf 1/2, Kopfhörer auf 3/4.
    Outputs34,
    /// Eigenes Kopfhörergerät: das Kopfhörersignal geht über [`crate::HeadSink`] an ein zweites Gerät.
    Device,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_are_fully_open_or_closed() {
        for c in [Curve::Soft, Curve::Linear, Curve::Cut, Curve::Custom([0.9, 0.7, 0.3, 0.1])] {
            let (a, b) = c.gains(-1.0);
            assert!((a - 1.0).abs() < 1e-6 && b.abs() < 1e-6, "{c:?}");
            let (a, b) = c.gains(1.0);
            assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6, "{c:?}");
        }
    }

    #[test]
    fn soft_curve_keeps_power_in_the_middle() {
        let (a, b) = Curve::Soft.gains(0.0);
        assert!((a * a + b * b - 1.0).abs() < 1e-5);
    }

    #[test]
    fn custom_curve_passes_its_points_and_mirrors() {
        let c = Curve::Custom([0.95, 0.8, 0.4, 0.05]);
        for (k, want) in CURVE_KNOTS.iter().zip([0.95, 0.8, 0.4, 0.05]) {
            let (a, b) = c.gains(k * 2.0 - 1.0);
            assert!((a - want).abs() < 1e-5, "A bei {k}: {a}");
            // B ist das Spiegelbild von A
            let (a_m, _) = c.gains((1.0 - k) * 2.0 - 1.0);
            assert!((b - a_m).abs() < 1e-5);
        }
    }

    #[test]
    fn custom_curve_never_rises_or_overshoots() {
        let c = Curve::Custom([1.0, 1.0, 0.0, 0.0]);
        let mut last = 1.0f32;
        for i in 0..=200 {
            let (a, _) = c.gains(i as f32 / 100.0 - 1.0);
            assert!((0.0..=1.0).contains(&a));
            assert!(a <= last + 1e-6, "steigt bei {i}");
            last = a;
        }
    }

    #[test]
    fn presets_turn_into_matching_custom_curves() {
        let soft = Curve::Custom(Curve::Soft.points());
        for i in 0..=20 {
            let x = i as f32 / 10.0 - 1.0;
            assert!((soft.gains(x).0 - Curve::Soft.gains(x).0).abs() < 0.03, "bei {x}");
        }
    }
}
