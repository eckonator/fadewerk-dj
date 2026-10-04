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
    /// Am Ende den alten Player anhalten und neu laden lassen. Nicht, wenn der alte ohnehin stand.
    handover: bool,
    /// Zweiphasige Überblendung (FadeWerk), sonst gleichmäßig über `duration`.
    split: Option<Split>,
}

/// Ablauf einer zweiphasigen Überblendung: Phase 1 führt den Crossfader zur Mitte (der neue Titel blendet
/// ein, der alte bleibt voll), Phase 2 zum neuen Player (der alte blendet aus). Gedacht für
/// [`crate::Curve::split_gains`], dort sind beide in der Mitte voll offen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Split {
    second: bool,
    /// Restzeit der laufenden Phase.
    left: f32,
    /// Dauer von Phase 2, solange Phase 1 läuft.
    out_secs: f32,
    x: f32,
}

impl Fade {
    pub fn remaining_secs(&self) -> f32 {
        match self.split {
            Some(s) if s.second => s.left.max(0.0),
            Some(s) => s.left.max(0.0) + s.out_secs,
            None => (self.duration - self.elapsed).max(0.0),
        }
    }

    /// Übergabe von einem laufenden Player an den anderen (nicht bloß Hineinblenden aus der Stille)?
    pub fn is_handover(&self) -> bool {
        self.handover
    }

    /// Zweiphasige Überblendung? Dann wertet die Engine den Crossfader mit `split_gains` aus.
    pub fn is_split(&self) -> bool {
        self.split.is_some()
    }

    /// Fortschritt der Überblendung, 0.0 bis 1.0. Zweiphasig ist 0.5 der Wechsel von Phase 1 zu Phase 2.
    pub fn progress(&self) -> f32 {
        match self.split {
            Some(s) => ((s.x - self.from_x) / (self.to_x - self.from_x)).clamp(0.0, 1.0),
            None => (self.elapsed / self.duration).clamp(0.0, 1.0),
        }
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

    /// Soll die automatische Überblendung jetzt starten? `remaining_secs` ist die Zeit bis zur Stelle, an der
    /// der aktive Titel enden soll, in Echtzeit (Pitch eingerechnet); `lead_secs` die Länge der Überblendung.
    pub fn should_start(&self, active_playing: bool, remaining_secs: f64, lead_secs: f32) -> bool {
        self.enabled && self.fade.is_none() && active_playing && remaining_secs <= lead_secs as f64
    }

    /// Startet eine Überblendung vom aktiven auf den anderen Player.
    pub fn start(&mut self, crossfader_now: f32) {
        self.begin(crossfader_now, None);
    }

    /// Startet eine zweiphasige Überblendung: `in_secs` blendet der neue Titel ein, danach `out_secs` der alte
    /// aus. 0 s ist ein harter Schnitt an dieser Stelle.
    pub fn start_split(&mut self, crossfader_now: f32, in_secs: f32, out_secs: f32) {
        let split = Split { second: false, left: in_secs.max(0.0), out_secs: out_secs.max(0.0), x: crossfader_now };
        self.begin(crossfader_now, Some(split));
    }

    fn begin(&mut self, crossfader_now: f32, split: Option<Split>) {
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
            handover: true,
            split,
        });
    }

    /// Richtet eine zweiphasige Überblendung nach dem alten Titel aus: `phase_left` = Zeit bis zum Ende der
    /// laufenden Phase, `out_secs` = Dauer von Phase 2 (nur in Phase 1 von Bedeutung). So endet der alte
    /// Titel genau am Ende seines Ausblendbereichs, auch wenn sich der Pitch ändert.
    pub fn retime(&mut self, phase_left: f32, out_secs: f32) {
        if let Some(s) = self.fade.as_mut().and_then(|f| f.split.as_mut()) {
            s.left = phase_left.max(0.0);
            if !s.second {
                s.out_secs = out_secs.max(0.0);
            }
        }
    }

    /// In Phase 2 einer zweiphasigen Überblendung?
    pub fn in_second_phase(&self) -> bool {
        self.fade.and_then(|f| f.split).is_some_and(|s| s.second)
    }

    /// Blendet den Crossfader zu `deck`, ohne dass der andere Player danach angehalten oder neu
    /// geladen wird. Für den Start eines Players, während der andere steht.
    pub fn glide_to(&mut self, deck: usize, crossfader_now: f32) {
        if self.fade.is_some() {
            return;
        }
        self.fade = Some(Fade {
            from: 1 - deck,
            to: deck,
            elapsed: 0.0,
            duration: self.fade_secs.max(0.05),
            from_x: crossfader_now,
            to_x: side(deck),
            handover: false,
            split: None,
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
        let (crossfader, done) = match f.split.as_mut() {
            Some(s) => {
                // Der Crossfader läuft in der verbleibenden Phasenzeit zum Ziel der Phase
                let target = if s.second { f.to_x } else { 0.0 };
                let phase_done = s.left <= dt;
                s.x = if phase_done { target } else { s.x + (target - s.x) * dt / s.left };
                s.left -= dt;
                let mut done = phase_done && s.second;
                if phase_done && !s.second {
                    s.second = true;
                    s.left = s.out_secs;
                    // Harter Schluss: Phase 2 ist sofort vorbei
                    if s.left <= 0.0 {
                        s.x = f.to_x;
                        done = true;
                    }
                }
                (s.x, done)
            }
            None => {
                let k = (f.elapsed / f.duration).min(1.0);
                (f.from_x + (f.to_x - f.from_x) * k, k >= 1.0)
            }
        };
        if done {
            let done = *f;
            self.fade = None;
            self.active = done.to;
            return Some(FadeStep { crossfader, finished: done.handover.then_some(done.from) });
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
    fn glide_moves_fader_without_handover() {
        let mut a = AutoFade { fade_secs: 2.0, ..Default::default() };
        a.glide_to(1, -1.0);
        assert_eq!(a.fade().map(|f| (f.from, f.to)), Some((0, 1)));
        let end = a.step(2.0).unwrap();
        assert_eq!(end.crossfader, 1.0);
        assert_eq!(end.finished, None);
        assert_eq!(a.active, 1);
    }

    #[test]
    fn starts_only_inside_fade_window() {
        let a = AutoFade { fade_secs: 8.0, ..Default::default() };
        assert!(!a.should_start(true, 30.0, 8.0));
        assert!(a.should_start(true, 7.9, 8.0));
        assert!(!a.should_start(false, 7.9, 8.0));
        assert!(a.should_start(true, 15.0, 20.0), "FadeWerk-Bereiche können länger sein");
        let off = AutoFade { enabled: false, ..a };
        assert!(!off.should_start(true, 1.0, 8.0));
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

    #[test]
    fn split_fade_runs_both_phases() {
        let mut a = AutoFade::default();
        a.start_split(-1.0, 2.0, 4.0);
        assert!(a.fade().unwrap().is_split());
        assert_eq!(a.fade().unwrap().remaining_secs(), 6.0);
        let x = a.step(1.0).unwrap().crossfader;
        assert!((x + 0.5).abs() < 1e-6, "Phase 1 halb: {x}");
        assert!((a.fade().unwrap().progress() - 0.25).abs() < 1e-6);
        assert_eq!(a.step(1.0).unwrap().crossfader, 0.0);
        assert!(a.in_second_phase());
        assert_eq!(a.step(2.0).unwrap().crossfader, 0.5);
        let end = a.step(2.0).unwrap();
        assert_eq!((end.crossfader, end.finished), (1.0, Some(0)));
        assert_eq!(a.active, 1);
    }

    #[test]
    fn split_fade_follows_retime_and_hard_cuts() {
        // Beide Bereiche 0 s: harter Schnitt im ersten Schritt
        let mut a = AutoFade::default();
        a.start_split(-1.0, 0.0, 0.0);
        assert_eq!(a.step(0.01).unwrap(), FadeStep { crossfader: 1.0, finished: Some(0) });
        // Der alte Titel läuft langsamer: Phase 2 dauert länger als geplant
        let mut a = AutoFade::default();
        a.start_split(-1.0, 1.0, 1.0);
        a.step(1.0);
        a.retime(3.0, 0.0);
        let x = a.step(1.0).unwrap().crossfader;
        assert!((x - 1.0 / 3.0).abs() < 1e-6, "{x}");
        assert!(a.fade().is_some());
    }
}
