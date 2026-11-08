// SPDX-License-Identifier: GPL-3.0-or-later
//! Beat-genauer Auto-Fade (Bauplan `.claude/plans/beatmatch-fade.md`): Haben beide Titel ein Beatraster, liegen
//! ihre Tempi höchstens 6 % auseinander und ist AUTO SYNC an, dann
//! - beginnt der Übergang an einer Phrasengrenze des alten Titels (8 oder 4 Takte, sonst am Taktbeginn),
//! - steigt der neue Titel ohne Motor-Anlauf so ein, dass seine Takte auf denen des alten liegen,
//! - dauert die Überblendung ganze Takte,
//! - hält Auto-Sync danach nicht nur das Tempo, sondern auch die Schläge zusammen (kleine Pitch-Korrektur).

use super::*;
use fw_library::Segment;
use super::fade::FadePlan;

/// Höchste Tempoanpassung für einen Übergang im Takt (ohne Keylock ändert sich die Tonhöhe mit).
pub(super) const BEAT_MAX_PCT: f32 = 6.0;
/// So viel früher als geplant darf der Übergang höchstens beginnen, um eine Phrasengrenze zu treffen.
const PHRASE_EARLY_SECS: f64 = 12.0;
/// Phase halten: Zeitkonstante der Korrektur und größte Pitch-Abweichung dafür (Prozent).
const PHASE_TAU_SECS: f64 = 2.0;
const PHASE_MAX_PCT: f32 = 0.4;

/// Übergang im Takt: Raster beider Titel und Tempofaktor (wie bei Auto-Sync: 1, 2 oder ½).
#[derive(Clone, Copy, Debug)]
pub(super) struct BeatPlan {
    pub from: usize,
    pub to: usize,
    pub old: Segment,
    new: Segment,
    factor: f64,
    /// Pitch des neuen Titels für gleiches Tempo (Prozent).
    pct: f32,
}

/// Warum ein Übergang nicht im Takt läuft.
pub(super) enum BeatMiss {
    /// AUTO SYNC aus oder kein zweiter Titel: nichts zu erklären.
    Off,
    /// Kein sicheres Raster am Ende des alten (`end`) bzw. am Anfang des neuen Titels; `pending` = wird noch ermittelt.
    NoGrid { end: bool, title: String, pending: bool },
    /// Tempi liegen so viele Prozent auseinander (auch halb/doppelt gerechnet).
    Tempo(f32),
}

impl BeatMiss {
    /// Kurz für die Übergangsleiste und ausführlich als Tooltip; `None` = nichts zeigen.
    pub(super) fn texts(&self) -> Option<(String, String)> {
        Some(match self {
            BeatMiss::Off => return None,
            BeatMiss::NoGrid { pending: true, .. } => (tr("Takt wird ermittelt").to_owned(), tr("Das Beatraster wird gerade berechnet.").to_owned()),
            BeatMiss::NoGrid { end: true, title, .. } => (
                tr("ohne Takt (Ende)").to_owned(),
                trf("Nicht im Takt: Für das Ende von „{}“ gibt es kein sicheres Beatraster (etwa wegen wanderndem Tempo).", &[title]),
            ),
            BeatMiss::NoGrid { title, .. } => (
                tr("ohne Takt (Anfang)").to_owned(),
                trf("Nicht im Takt: Für den Anfang von „{}“ gibt es kein sicheres Beatraster (etwa wegen wanderndem Tempo).", &[title]),
            ),
            BeatMiss::Tempo(p) => {
                let p = format!("{p:.1}");
                (tr("ohne Takt (Tempo)").to_owned(), trf("Nicht im Takt: Die Tempi liegen {} % auseinander, erlaubt sind höchstens 6 %.", &[&p]))
            }
        })
    }
}

impl FadeWerkDjApp {
    /// Raster des Titels in Player `d` an der Stelle `secs` (Anfang bzw. Ende, siehe `BeatGrid::at`).
    pub(super) fn segment_at(&self, d: usize, secs: f64) -> Option<Segment> {
        let v = &self.decks[d];
        let grid = v.path.as_ref().and_then(|p| self.grids.get(p).copied().flatten())?;
        let duration = self.ctl.shared.decks[d].duration_secs().max(v.meta.as_ref().map_or(0.0, |m| m.duration_secs));
        grid.at(secs, duration)
    }

    /// Tempo des Titels in Player `d` an seiner Stelle: aus dem Raster (genauer), sonst die BPM.
    pub(super) fn tempo_of(&self, d: usize) -> Option<f64> {
        self.segment_at(d, self.ctl.shared.decks[d].position_secs()).map(|s| s.bpm).or(self.decks[d].bpm.map(f64::from))
    }

    /// Lässt sich der nächste Übergang im Takt fahren? `old_at` = Stelle im alten Titel, an der er beginnen soll.
    pub(super) fn beat_plan(&self, old_at: f64) -> Option<BeatPlan> {
        self.beat_check(old_at).ok()
    }

    /// Wie `beat_plan`, ohne Takt aber mit dem Grund (für die Übergangsleiste). Läuft der alte Titel allein und kommt
    /// sein Pitch von Auto-Sync, gilt beim Übergang das natürliche Tempo (Auto-Sync bringt den Pitch bis dahin auf 0);
    /// passt nur der aktuelle Pitch, hält Auto-Sync ihn stattdessen (`keep_pitch_for_beat`).
    pub(super) fn beat_check(&self, old_at: f64) -> Result<BeatPlan, BeatMiss> {
        let (from, to) = (self.autofade.active, self.autofade.other());
        let pitch = self.decks[from].pitch_pct as f64;
        let returning = self.decks[from].sync.is_some() && !self.on_air(to);
        if !returning {
            return self.beat_check_at(old_at, pitch);
        }
        let natural = self.beat_check_at(old_at, 0.0);
        match natural {
            Err(BeatMiss::Tempo(_)) => self.beat_check_at(old_at, pitch).or(natural),
            _ => natural,
        }
    }

    /// Soll Auto-Sync den Pitch des alten Titels halten? Ja, wenn der nächste Übergang nur mit ihm im Takt laufen kann.
    pub(super) fn keep_pitch_for_beat(&self) -> bool {
        let Some(old_at) = self.planned_old_at() else { return false };
        let pitch = self.decks[self.autofade.active].pitch_pct as f64;
        pitch != 0.0 && matches!(self.beat_check_at(old_at, 0.0), Err(BeatMiss::Tempo(_))) && self.beat_check_at(old_at, pitch).is_ok()
    }

    /// Stelle im alten Titel, an der der geplante Übergang beginnt.
    pub(super) fn planned_old_at(&self) -> Option<f64> {
        let FadePlan::Planned { plan, .. } = self.fade_plan() else { return None };
        let rate = 1.0 + self.decks[self.autofade.active].pitch_pct as f64 / 100.0;
        Some(plan.end - plan.lead as f64 * rate)
    }

    /// Prüfung mit einem angenommenen Pitch des alten Titels beim Übergang.
    fn beat_check_at(&self, old_at: f64, old_pitch: f64) -> Result<BeatPlan, BeatMiss> {
        if !self.auto_sync {
            return Err(BeatMiss::Off);
        }
        let (from, to) = (self.autofade.active, self.autofade.other());
        if !self.has_track(from) || !self.has_track(to) || self.decks[to].spent || self.loading(to) {
            return Err(BeatMiss::Off);
        }
        let missing = |d: usize, end: bool| {
            let title = self.decks[d].path.as_deref().map(|p| self.title_of(p)).unwrap_or_default();
            let pending = self.decks[d].path.as_ref().is_none_or(|p| !self.grids.contains_key(p));
            BeatMiss::NoGrid { end, title, pending }
        };
        let old = self.segment_at(from, old_at).ok_or_else(|| missing(from, true))?;
        let new = self.segment_at(to, self.ctl.shared.decks[to].position_secs()).ok_or_else(|| missing(to, false))?;
        let theirs = old.bpm * (1.0 + old_pitch / 100.0);
        let pct = |f: f64| (theirs / (new.bpm * f) - 1.0) * 100.0;
        let factor = [1.0, 2.0, 0.5].into_iter().min_by(|a, b| pct(*a).abs().total_cmp(&pct(*b).abs())).unwrap_or(1.0);
        let p = pct(factor) as f32;
        if p.abs() > BEAT_MAX_PCT {
            return Err(BeatMiss::Tempo(p.abs()));
        }
        Ok(BeatPlan { from, to, old, new, factor, pct: p })
    }

    /// Stelle im alten Titel, an der ein Übergang im Takt beginnt: die letzte Phrasengrenze (8, sonst 4 Takte) vor
    /// dem geplanten Beginn, wenn sie höchstens `PHRASE_EARLY_SECS` früher liegt, sonst der letzte Taktbeginn.
    pub(super) fn phrase_start(old: Segment, planned: f64) -> f64 {
        let beats = old.beats_at(planned);
        for bars in [8.0, 4.0, 1.0] {
            let at = old.downbeat + (beats / (4.0 * bars)).floor() * 4.0 * bars * old.beat_secs();
            if planned - at <= PHRASE_EARLY_SECS || bars == 1.0 {
                return at;
            }
        }
        planned
    }

    /// Letzter Taktbeginn im alten Titel vor `planned` (für Übergänge mit FadeWerk-Bereichen).
    pub(super) fn bar_start(old: Segment, planned: f64) -> f64 {
        old.downbeat + (old.beats_at(planned) / 4.0).floor() * 4.0 * old.beat_secs()
    }

    /// Startet den neuen Titel im Takt: Tempo angleichen, Einstieg so wählen, dass seine Takte auf denen des alten
    /// liegen (nahe seiner Startstelle, etwa dem Beginn des Einblendbereichs), dann ohne Motor-Anlauf los.
    /// Liefert die Überblendlänge in ganzen Takten.
    pub(super) fn start_on_beat(&mut self, bp: BeatPlan) -> f32 {
        let old_pos = self.ctl.shared.decks[bp.from].position_secs();
        let new_pos = self.ctl.shared.decks[bp.to].position_secs();
        // Wo im Takt steht der alte Titel? In Schlägen des neuen (Faktor 2: ein neuer Schlag = zwei alte)
        let target = (bp.old.beats_at(old_pos) / bp.factor).rem_euclid(4.0);
        let n = ((bp.new.beats_at(new_pos) - target) / 4.0).round();
        let mut start = bp.new.downbeat + (n * 4.0 + target) * bp.new.beat_secs();
        if start < 0.0 {
            start += 4.0 * bp.new.beat_secs();
        }
        self.decks[bp.to].sync = Some(bp.factor as f32);
        self.send_pitch(bp.to, bp.pct);
        self.ctl.send(Command::Seek { deck: bp.to, secs: start });
        self.ctl.send(Command::Resume { deck: bp.to });
        self.beat_lock = Some(bp.to);
        let rate = 1.0 + self.decks[bp.from].pitch_pct as f64 / 100.0;
        let bar = 4.0 * bp.old.beat_secs() / rate;
        let bars = (self.autofade.fade_secs as f64 / bar).round().max(1.0);
        log::info!(
            "Übergang im Takt {}→{}: alt {:.2} BPM bei {:.2} s, neu {:.2} BPM ab {:.2} s (Pitch {:+.2} %, Faktor {}), {} Takte",
            theme::deck_letter(bp.from), theme::deck_letter(bp.to), bp.old.bpm, old_pos, bp.new.bpm, start, bp.pct, bp.factor, bars
        );
        (bars * bar) as f32
    }

    /// Phase halten: Pitch-Korrektur (Prozent), damit die Schläge des mitgezogenen Players `d` auf denen des
    /// anderen bleiben. Zieht ihn sanft auf den nächsten Schlag; 0 ohne Raster.
    pub(super) fn phase_correction(&self, d: usize, factor: f32) -> f32 {
        let o = 1 - d;
        let (pos_d, pos_o) = (self.ctl.shared.decks[d].position_secs(), self.ctl.shared.decks[o].position_secs());
        let (Some(mine), Some(theirs)) = (self.segment_at(d, pos_d), self.segment_at(o, pos_o)) else { return 0.0 };
        let diff = mine.beats_at(pos_d) - theirs.beats_at(pos_o) / factor as f64;
        let err = diff - diff.round();
        log::debug!("Phase {}: {:+.3} Schläge ({:+.1} ms), Takt {:+.2}", theme::deck_letter(d), err, err * mine.beat_secs() * 1000.0, diff.rem_euclid(4.0));
        if err.abs() < 0.004 {
            return 0.0;
        }
        // Fehler in Sekunden des eigenen Titels, in `PHASE_TAU_SECS` ausgleichen
        let pct = -(err * mine.beat_secs() / PHASE_TAU_SECS * 100.0) as f32;
        pct.clamp(-PHASE_MAX_PCT, PHASE_MAX_PCT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_starts_on_a_phrase_or_bar() {
        // 120 BPM: Schlag 0,5 s, Takt 2 s, 8 Takte 16 s, 4 Takte 8 s; Taktbeginn bei 1 s
        let seg = Segment { bpm: 120.0, downbeat: 1.0 };
        // 8-Takt-Grenzen bei 1, 17, 33 …: geplant 40 s → 33 s (7 s früher)
        assert!((FadeWerkDjApp::phrase_start(seg, 40.0) - 33.0).abs() < 1e-9);
        // geplant 48 s: letzte 8-Takt-Grenze 33 s ist 15 s früher (zu viel) → 4 Takte: 41 s
        assert!((FadeWerkDjApp::phrase_start(seg, 48.0) - 41.0).abs() < 1e-9);
        // Genau auf einer Grenze bleibt es dabei
        assert!((FadeWerkDjApp::phrase_start(seg, 17.0) - 17.0).abs() < 1e-9);
        // Sehr langsames Tempo (40 BPM, 4 Takte = 24 s): dann der letzte Taktbeginn
        let slow = Segment { bpm: 40.0, downbeat: 0.0 };
        assert!((FadeWerkDjApp::phrase_start(slow, 100.0) - 96.0).abs() < 1e-9);
    }
}
