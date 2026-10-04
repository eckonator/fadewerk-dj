// SPDX-License-Identifier: GPL-3.0-or-later
//! Vorhören im Fade-Editor: ein Bereich allein (mit seinem Ein- oder Ausblenden) oder der fertig gemischte
//! Übergang mit dem Nachbartitel aus der anderen Playlist. Gemischt wird vorab in einem eigenen Thread,
//! gespielt über die Vorhör-Stimme (nur Kopfhörer).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use fw_engine::{Curve, TrackAudio};

use crate::fadewerk::{self, Region, TrackFade};

/// So viele Sekunden laufen vor bzw. nach dem Bereich mit, damit man den Zusammenhang hört.
const CONTEXT_SECS: f64 = 4.0;

/// Was vorgehört wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Einblendbereich allein.
    In,
    /// Ausblendbereich allein.
    Out,
    /// Übergang vom Titel davor in diesen.
    From,
    /// Übergang von diesem in den Titel danach.
    To,
}

impl Kind {
    /// Gehört zum Einblend-Ausschnitt (links) im Editor?
    pub fn is_head(self) -> bool {
        matches!(self, Kind::In | Kind::From)
    }
}

/// Nachbartitel für einen Übergang.
#[derive(Clone, Debug)]
pub struct Partner {
    pub list: usize,
    pub name: String,
}

/// Was der offene Editor über das Vorhören wissen muss.
#[derive(Clone, Debug, Default)]
pub struct EditorAudio {
    /// Titel davor und danach (für „Übergang“).
    pub partners: [Option<Partner>; 2],
    /// Läuft gerade ein Vorhören für den offenen Eintrag: was, und an welcher Stelle des Titels
    /// (`None`, solange noch gemischt wird).
    pub playing: Option<(Kind, Option<f32>)>,
}

/// Ein Titel für das Mischen. Liegt er schon in einem Player, wird dessen Audio genommen.
pub struct Source {
    pub path: PathBuf,
    pub audio: Option<Arc<TrackAudio>>,
    pub fade: TrackFade,
}

impl Source {
    fn audio(&self) -> Result<Arc<TrackAudio>, String> {
        match &self.audio {
            Some(a) => Ok(a.clone()),
            None => fw_library::decode::decode_file(&self.path).map(Arc::new).map_err(|e| format!("{e:#}")),
        }
    }
}

/// Fertig gemischter Ausschnitt. Stelle im bearbeiteten Titel = Stelle im Ausschnitt + `offset`.
pub struct Clip {
    pub audio: Arc<TrackAudio>,
    pub offset: f64,
}

pub struct Job {
    pub kind: Kind,
    pub edited: Source,
    pub partner: Option<Source>,
    pub curve: Curve,
    pub fade_secs: f32,
}

/// Mischt im Hintergrund, das Ergebnis kommt über den Kanal.
pub fn spawn(job: Job) -> Receiver<Result<Clip, String>> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("fw-fade-preview".into()).spawn(move || {
        let _ = tx.send(render(&job));
    });
    if let Err(e) = spawned {
        log::warn!("Vorhören: Thread nicht gestartet: {e}");
    }
    rx
}

fn render(job: &Job) -> Result<Clip, String> {
    let edited = job.edited.audio()?;
    let half = job.fade_secs * 0.5;
    let dur = edited.duration_secs();
    match job.kind {
        Kind::In => {
            let r = job.edited.fade.fade_in.unwrap_or(Region { start: 0.0, end: half });
            Ok(region_clip(&edited, r, true, job.curve))
        }
        Kind::Out => {
            let r = job.edited.fade.fade_out.unwrap_or(Region { start: (dur as f32 - half).max(0.0), end: dur as f32 });
            Ok(region_clip(&edited, r, false, job.curve))
        }
        Kind::From | Kind::To => {
            let partner = job.partner.as_ref().ok_or_else(|| "kein Nachbartitel".to_owned())?;
            let other = partner.audio()?;
            Ok(if job.kind == Kind::From {
                transition_clip(&other, partner.fade, &edited, job.edited.fade, job.curve, job.fade_secs, false)
            } else {
                transition_clip(&edited, job.edited.fade, &other, partner.fade, job.curve, job.fade_secs, true)
            })
        }
    }
}

/// Pegel des einblendenden Titels bei Fortschritt `p` (0 bis 1), in der Form der Blendkurve.
fn rise(curve: Curve, p: f64) -> f32 {
    curve.gains((p.clamp(0.0, 1.0) * 2.0 - 1.0) as f32).1
}

fn fall(curve: Curve, p: f64) -> f32 {
    curve.gains((p.clamp(0.0, 1.0) * 2.0 - 1.0) as f32).0
}

/// Fortschritt von `t` im Abschnitt `a..b`, bei Länge 0 ein harter Sprung.
fn progress(t: f64, a: f64, b: f64) -> f64 {
    if b - a <= 1e-6 { if t >= a { 1.0 } else { 0.0 } } else { (t - a) / (b - a) }
}

/// Ein Bereich allein: Einblenden ab seinem Anfang (davor ist der Titel nicht zu hören) und ein paar Sekunden
/// weiter, Ausblenden ein paar Sekunden davor bis zu seinem Ende.
fn region_clip(track: &TrackAudio, r: Region, fade_in: bool, curve: Curve) -> Clip {
    let (start, end) = (r.start as f64, r.end as f64);
    let (from, to) = if fade_in { (start, end + CONTEXT_SECS) } else { ((start - CONTEXT_SECS).max(0.0), end) };
    let rate = track.sample_rate as f64;
    let n = ((to - from) * rate).max(0.0) as usize;
    let frames = (0..n)
        .map(|k| {
            let t = from + k as f64 / rate;
            let p = progress(t, start, end);
            let g = if fade_in { rise(curve, p) } else { fall(curve, p) };
            let s = track.sample_at(t * rate);
            [s[0] * g, s[1] * g]
        })
        .collect();
    Clip { audio: Arc::new(TrackAudio::new(track.sample_rate, frames)), offset: from }
}

/// Übergang von `old` nach `new` so, wie Auto-Fade ihn spielt (ohne Pitch). `edited_is_old` legt fest, auf
/// welchen der beiden Titel sich `Clip::offset` bezieht.
fn transition_clip(old: &TrackAudio, old_fade: TrackFade, new: &TrackAudio, new_fade: TrackFade, curve: Curve, fade_secs: f32, edited_is_old: bool) -> Clip {
    let plan = fadewerk::plan(old_fade, old.duration_secs(), new_fade, fade_secs, 1.0, 1.0);
    let t0 = plan.end - plan.lead as f64;
    let new_start = new_fade.fade_in.map_or(0.0, |r| r.start as f64);
    let from = (t0 - CONTEXT_SECS).max(0.0);
    let to = plan.end + CONTEXT_SECS;
    // Gemischt wird in der Abtastrate des alten Titels, der neue wird beim Lesen umgerechnet
    let rate = old.sample_rate as f64;
    let new_rate = new.sample_rate as f64;
    let n = ((to - from) * rate).max(0.0) as usize;
    let frames = (0..n)
        .map(|k| {
            let t = from + k as f64 / rate;
            let (go, gn) = match plan.split {
                // Zweiphasig: erst kommt der neue hoch, dann geht der alte
                Some(sp) if t < sp.out_start => (1.0, if t < t0 { 0.0 } else { rise(curve, progress(t, t0, sp.out_start)) }),
                Some(sp) => (fall(curve, progress(t, sp.out_start, sp.out_end)), 1.0),
                None => {
                    let p = progress(t, t0, plan.end).clamp(0.0, 1.0);
                    if t < t0 { (1.0, 0.0) } else { (fall(curve, p), rise(curve, p)) }
                }
            };
            let a = old.sample_at(t * rate);
            let tn = new_start + (t - t0);
            let b = if tn >= new_start { new.sample_at(tn * new_rate) } else { [0.0; 2] };
            [a[0] * go + b[0] * gn, a[1] * go + b[1] * gn]
        })
        .collect();
    let offset = if edited_is_old { from } else { new_start + from - t0 };
    Clip { audio: Arc::new(TrackAudio::new(old.sample_rate, frames)), offset }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gleichspannung als Titel: so lässt sich der Pegel direkt ablesen.
    fn dc(rate: u32, secs: f64, v: f32) -> TrackAudio {
        TrackAudio::new(rate, vec![[v, v]; (secs * rate as f64) as usize])
    }

    fn at(clip: &Clip, track_secs: f64) -> f32 {
        let rate = clip.audio.sample_rate as f64;
        clip.audio.frames[((track_secs - clip.offset) * rate) as usize][0]
    }

    #[test]
    fn region_fades_in_and_out() {
        let t = dc(100, 60.0, 1.0);
        let c = region_clip(&t, Region { start: 10.0, end: 20.0 }, true, Curve::Linear);
        assert_eq!(c.offset, 10.0);
        assert!((at(&c, 15.0) - 0.5).abs() < 0.02);
        assert_eq!(at(&c, 22.0), 1.0);
        let c = region_clip(&t, Region { start: 40.0, end: 40.0 }, false, Curve::Linear);
        assert_eq!(c.offset, 36.0);
        assert_eq!(at(&c, 39.9), 1.0, "harter Schluss: bis dahin voll");
        assert_eq!(c.audio.duration_secs(), 4.0);
    }

    #[test]
    fn transition_mixes_like_auto_fade() {
        // Alt = 1, neu = 2: in Phase 1 summiert sich der Pegel, danach bleibt nur der neue
        let old = dc(100, 60.0, 1.0);
        let new = dc(50, 60.0, 2.0);
        let r = |start, end| Some(Region { start, end });
        let of = TrackFade { fade_in: None, fade_out: r(50.0, 54.0) };
        let nf = TrackFade { fade_in: r(5.0, 7.0), fade_out: None };
        let c = transition_clip(&old, of, &new, nf, Curve::Linear, 8.0, true);
        assert_eq!(c.offset, 44.0, "4 s vor dem Beginn der Überblendung (48 s)");
        assert_eq!(at(&c, 47.0), 1.0);
        assert!((at(&c, 49.0) - 2.0).abs() < 0.05, "Phase 1 halb: 1 + 0,5 · 2");
        assert!((at(&c, 52.0) - 2.5).abs() < 0.05, "Phase 2 halb: 0,5 + 2");
        assert_eq!(at(&c, 55.0), 2.0);
        // Aus Sicht des neuen Titels: dessen Einblendbeginn (5 s) liegt bei 48 s des alten
        let c = transition_clip(&old, of, &new, nf, Curve::Linear, 8.0, false);
        assert_eq!(c.offset, 1.0);
    }
}
