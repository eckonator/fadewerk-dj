// SPDX-License-Identifier: GPL-3.0-or-later
/// Motor des virtuellen Plattentellers.
///
/// Die Drehzahl läuft linear auf das Ziel zu: beim Start in `spin_up_secs`
/// von 0 auf Nenndrehzahl, beim Stoppen in `spin_down_secs` zurück auf 0.
/// Dadurch klingen Start, Pause und Stopp wie bei einem echten Plattenspieler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motor {
    pub spin_up_secs: f32,
    pub spin_down_secs: f32,
}

impl Default for Motor {
    fn default() -> Self {
        Self { spin_up_secs: 0.5, spin_down_secs: 1.6 }
    }
}

impl Motor {
    /// Drehzahl nach `dt` Sekunden. 1.0 ist die Nenndrehzahl.
    pub fn step(&self, current: f32, target: f32, dt: f32) -> f32 {
        if current < target {
            (current + dt / self.spin_up_secs.max(1e-3)).min(target)
        } else {
            (current - dt / self.spin_down_secs.max(1e-3)).max(target)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spins_up_and_down_linearly() {
        let m = Motor { spin_up_secs: 0.5, spin_down_secs: 2.0 };
        assert!((m.step(0.0, 1.0, 0.25) - 0.5).abs() < 1e-6);
        assert_eq!(m.step(0.9, 1.0, 0.25), 1.0);
        assert!((m.step(1.0, 0.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(m.step(0.1, 0.0, 1.0), 0.0);
    }
}
