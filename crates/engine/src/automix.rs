// SPDX-License-Identifier: GPL-3.0-or-later
//! Auto-Fade: blendet automatisch vom laufenden Player auf den anderen über,
//! sobald der laufende Titel in die Überblendzeit kommt.
//!
//! Reine Steuerlogik ohne Audio. Die Oberfläche ruft sie pro Bild auf und setzt
//! die gelieferten Crossfader-Werte und Start/Stopp-Befehle um.

/// Laufende Überblendung.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub from: usize,
    pub to: usize,
    elapsed: f32,
    duration: f32,
    from_x: f32,
    to_x: f32,
}

impl Fade {
    pub fn remaining_secs(&self) -> f32 {
        (self.duration - self.elapsed).max(0.0)
    }

    /// Fortschritt der Überblendung, 0.0 bis 1.0.
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// Ergebnis eines Schritts während einer Überblendung.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeStep {
    /// Neue Crossfader-Stellung.
    pub crossfader: f32,
    /// Am Ende der Überblendung: dieser Player ist jetzt still und kann gestoppt werden.
    pub finished: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct AutoFade {
    pub enabled: bool,
    pub fade_secs: f32,
    /// Player, der gerade auf Sendung ist (0 = A, 1 = B).
    pub active: usize,
    fade: Option<Fade>,
}

impl Default for AutoFade {
    fn default() -> Self {
        Self { enabled: true, fade_secs: 8.0, active: 0, fade: None }
    }
}

/// Crossfader-Endstellung für einen Player.
pub fn side(deck: usize) -> f32 {
    if deck == 0 { -1.0 } else { 1.0 }
}

impl AutoFade {
    pub fn other(&self) -> usize {
        1 - self.active
    }

    pub fn fade(&self) -> Option<&Fade> {
        self.fade.as_ref()
    }

    /// Soll die automatische Überblendung jetzt starten?
    /// `remaining_secs` ist die Restzeit des aktiven Titels in Echtzeit (Pitch eingerechnet).
    pub fn should_start(&self, active_playing: bool, remaining_secs: f64) -> bool {
        self.enabled && self.fade.is_none() && active_playing && remaining_secs <= self.fade_secs as f64
    }

    /// Startet eine Überblendung vom aktiven auf den anderen Player.
    pub fn start(&mut self, crossfader_now: f32) {
        if self.fade.is_some() {
            return;
        }
        let to = self.other();
        self.fade = Some(Fade {
            from: self.active,
            to,
            elapsed: 0.0,
            duration: self.fade_secs.max(0.05),
            from_x: crossfader_now,
            to_x: side(to),
        });
    }

    /// Bricht eine laufende Überblendung ab, etwa wenn der Crossfader von Hand bewegt wird.
    pub fn cancel(&mut self) {
        self.fade = None;
    }

    /// Ein Zeitschritt. Liefert `None`, wenn gerade nicht geblendet wird.
    pub fn step(&mut self, dt: f32) -> Option<FadeStep> {
        let f = self.fade.as_mut()?;
        f.elapsed += dt;
        let k = (f.elapsed / f.duration).min(1.0);
        let crossfader = f.from_x + (f.to_x - f.from_x) * k;
        if k >= 1.0 {
            let done = *f;
            self.fade = None;
            self.active = done.to;
            return Some(FadeStep { crossfader, finished: Some(done.from) });
        }
        Some(FadeStep { crossfader, finished: None })
    }

    /// Wer ist auf Sendung, wenn der Crossfader von Hand auf `x` steht?
    pub fn follow_crossfader(&mut self, x: f32) {
        if self.fade.is_none() {
            self.active = if x <= 0.0 { 0 } else { 1 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_only_inside_fade_window() {
        let a = AutoFade { fade_secs: 8.0, ..Default::default() };
        assert!(!a.should_start(true, 30.0));
        assert!(a.should_start(true, 7.9));
        assert!(!a.should_start(false, 7.9));
        let off = AutoFade { enabled: false, ..a };
        assert!(!off.should_start(true, 1.0));
    }

    #[test]
    fn fades_across_and_hands_over() {
        let mut a = AutoFade { fade_secs: 2.0, ..Default::default() };
        a.start(-1.0);
        let mid = a.step(1.0).unwrap();
        assert!((mid.crossfader - 0.0).abs() < 1e-6);
        assert_eq!(mid.finished, None);
        let end = a.step(1.0).unwrap();
        assert_eq!(end.crossfader, 1.0);
        assert_eq!(end.finished, Some(0));
        assert_eq!(a.active, 1);
        assert!(a.step(0.1).is_none());
        // nächste Runde geht zurück auf A
        a.start(1.0);
        assert_eq!(a.fade().unwrap().to, 0);
    }
}
