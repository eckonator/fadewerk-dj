// SPDX-License-Identifier: GPL-3.0-or-later
use std::sync::Arc;

use crate::mixer::Curve;
use crate::motor::Motor;
use crate::track::TrackAudio;

/// Wie schnell der Lesekopf der Hand beim Scratchen folgt (1/s).
const SCRATCH_FOLLOW: f64 = 25.0;
/// Höchste Lesegeschwindigkeit beim Scratchen, als Vielfaches der Nenndrehzahl.
const SCRATCH_MAX_RATE: f32 = 10.0;
/// Zeitkonstante der Geschwindigkeitsglättung in Sekunden.
const RATE_SMOOTH_SECS: f32 = 0.005;

/// Ein Player: spielt einen Titel mit Motor, Pitch, Scratch, Cue und Loop.
///
/// Positionen sind intern in Frames des Titels gespeichert, nach außen in Sekunden.
pub struct Deck {
    track: Option<Arc<TrackAudio>>,
    track_id: u64,
    pos: f64,
    playing: bool,
    /// Motordrehzahl ohne Pitch, 0.0 bis 1.0.
    speed: f32,
    /// Pitch als Anteil, 0.08 = +8 %.
    pitch: f32,
    /// Tatsächliche, geglättete Lesegeschwindigkeit. Beim Scratchen auch negativ.
    rate: f32,
    /// Zielposition der Hand in Frames, solange die Platte festgehalten wird.
    scratch: Option<f64>,
    cue: f64,
    loop_range: Option<(f64, f64)>,
    stop_pending: bool,
    repeat_track: bool,
    /// Bei Repeat blendet das Titelende über so viele Sekunden in den Anfang (0 = harter Sprung).
    repeat_fade_secs: f32,
    /// Blendkurve dafür, dieselbe wie beim Crossfader.
    repeat_curve: Curve,
    ended: bool,
}

impl Default for Deck {
    fn default() -> Self {
        Self {
            track: None,
            track_id: 0,
            pos: 0.0,
            playing: false,
            speed: 0.0,
            pitch: 0.0,
            rate: 0.0,
            scratch: None,
            cue: 0.0,
            loop_range: None,
            stop_pending: false,
            repeat_track: false,
            repeat_fade_secs: 0.0,
            repeat_curve: Curve::Soft,
            ended: false,
        }
    }
}

impl Deck {
    /// Lädt einen Titel und gibt den alten zurück (zum Freigeben außerhalb des Audio-Threads).
    /// `keep_playing` lässt einen laufenden Player direkt mit dem neuen Titel weiterlaufen.
    pub fn load(&mut self, track: Arc<TrackAudio>, id: u64, keep_playing: bool) -> Option<Arc<TrackAudio>> {
        let old = self.track.replace(track);
        self.track_id = id;
        self.pos = 0.0;
        self.cue = 0.0;
        self.loop_range = None;
        self.scratch = None;
        self.stop_pending = false;
        self.ended = false;
        if !(keep_playing && self.playing) {
            self.playing = false;
            self.speed = 0.0;
            self.rate = 0.0;
        }
        old
    }

    pub fn eject(&mut self) -> Option<Arc<TrackAudio>> {
        let old = self.track.take();
        *self = Deck {
            pitch: self.pitch,
            repeat_track: self.repeat_track,
            repeat_fade_secs: self.repeat_fade_secs,
            repeat_curve: self.repeat_curve,
            ..Deck::default()
        };
        old
    }

    pub fn play(&mut self) {
        if self.track.is_some() {
            self.playing = true;
            self.stop_pending = false;
            self.ended = false;
        }
    }

    /// Pause: der Teller läuft aus und bleibt an der erreichten Stelle stehen.
    pub fn pause(&mut self) {
        self.playing = false;
        self.stop_pending = false;
    }

    pub fn toggle(&mut self) {
        if self.playing { self.pause() } else { self.play() }
    }

    /// Stopp: der Teller läuft aus, danach springt der Player zum Cue-Punkt.
    pub fn stop(&mut self) {
        if self.playing || self.speed > 0.0 {
            self.playing = false;
            self.stop_pending = true;
        } else {
            self.pos = self.cue;
        }
    }

    /// CUE wie bei CD-Playern: läuft der Player, springt er sofort zum Cue-Punkt und hält.
    /// Steht er, wird die aktuelle Stelle zum neuen Cue-Punkt.
    pub fn cue(&mut self) {
        if self.track.is_none() {
            return;
        }
        if self.playing {
            self.playing = false;
            self.speed = 0.0;
            self.rate = 0.0;
            self.pos = self.cue;
        } else {
            self.cue = self.pos;
        }
    }

    pub fn seek_secs(&mut self, secs: f64) {
        if let Some(t) = &self.track {
            self.pos = (secs * t.sample_rate as f64).clamp(0.0, t.len_frames() as f64);
            self.ended = false;
        }
    }

    pub fn set_pitch(&mut self, pitch: f32) {
        self.pitch = pitch.clamp(-0.5, 0.5);
    }

    pub fn set_repeat_track(&mut self, on: bool) {
        self.repeat_track = on;
    }

    /// Überblendung von Titelende in den Anfang bei Repeat (Dauer in Sekunden und Kurve).
    pub fn set_repeat_fade(&mut self, secs: f32, curve: Curve) {
        self.repeat_fade_secs = secs.max(0.0);
        self.repeat_curve = curve;
    }

    pub fn set_loop_secs(&mut self, range: Option<(f64, f64)>) {
        self.loop_range = match (&self.track, range) {
            (Some(t), Some((a, b))) if b > a => {
                let sr = t.sample_rate as f64;
                Some((a * sr, b * sr))
            }
            _ => None,
        };
    }

    /// Hand auf die Platte: ab jetzt folgt der Lesekopf `scratch_to`.
    pub fn scratch_begin(&mut self) {
        if self.track.is_some() {
            self.scratch = Some(self.pos);
        }
    }

    pub fn scratch_to_secs(&mut self, secs: f64) {
        if let (Some(t), Some(target)) = (&self.track, self.scratch.as_mut()) {
            *target = (secs * t.sample_rate as f64).clamp(0.0, t.len_frames() as f64);
        }
    }

    /// Hand los: läuft der Player, zieht der Motor die Platte aus der aktuellen
    /// Handgeschwindigkeit wieder auf Tempo.
    pub fn scratch_end(&mut self) {
        if self.scratch.take().is_some() {
            self.speed = if self.playing {
                (self.rate / (1.0 + self.pitch)).clamp(0.0, 1.5)
            } else {
                0.0
            };
        }
    }

    pub fn has_track(&self) -> bool { self.track.is_some() }
    pub fn track_id(&self) -> u64 { self.track_id }
    pub fn is_playing(&self) -> bool { self.playing }
    pub fn is_scratching(&self) -> bool { self.scratch.is_some() }
    pub fn speed(&self) -> f32 { self.speed }
    pub fn rate(&self) -> f32 { self.rate }
    pub fn pitch(&self) -> f32 { self.pitch }

    /// Ist der Titel seit dem letzten Aufruf zu Ende gelaufen?
    pub fn take_ended(&mut self) -> bool {
        std::mem::take(&mut self.ended)
    }

    pub fn position_secs(&self) -> f64 {
        self.track.as_ref().map_or(0.0, |t| self.pos / t.sample_rate as f64)
    }

    pub fn duration_secs(&self) -> f64 {
        self.track.as_ref().map_or(0.0, |t| t.duration_secs())
    }

    /// Schreibt das Signal des Players (vor dem Kanalfader) nach `out`.
    pub fn render(&mut self, motor: &Motor, out_rate: f32, out: &mut [[f32; 2]]) {
        let n = out.len();
        let Some(track) = self.track.clone() else {
            out.fill([0.0; 2]);
            return;
        };
        let track_rate = track.sample_rate as f64;
        let len = track.len_frames() as f64;
        let step_scale = track_rate / out_rate as f64;
        let smooth = 1.0 - (-1.0 / (RATE_SMOOTH_SECS * out_rate)).exp();
        // Repeat mit Überblendung: in den letzten `fade` Frames läuft der Anfang des Titels mit,
        // im gleichen Abstand wie das Ende (Lesekopf 2 = pos - fade_start). Höchstens ein Drittel des Titels.
        let fade = if self.repeat_track && self.loop_range.is_none() {
            (self.repeat_fade_secs as f64 * track_rate).min(len / 3.0)
        } else {
            0.0
        };
        let fade_start = len - fade;

        // Motor über den Block vorrechnen, pro Sample linear überblenden
        let dt = n as f32 / out_rate;
        let speed_start = self.speed;
        let speed_end = if self.scratch.is_none() {
            motor.step(self.speed, if self.playing { 1.0 } else { 0.0 }, dt)
        } else {
            self.speed
        };

        for (i, frame) in out.iter_mut().enumerate() {
            let desired = match self.scratch {
                Some(target) => {
                    let v = (target - self.pos) / track_rate * SCRATCH_FOLLOW;
                    (v as f32).clamp(-SCRATCH_MAX_RATE, SCRATCH_MAX_RATE)
                }
                None => {
                    let s = speed_start + (speed_end - speed_start) * (i as f32 / n as f32);
                    s * (1.0 + self.pitch)
                }
            };
            self.rate += (desired - self.rate) * smooth;

            if self.rate.abs() < 1e-4 {
                *frame = [0.0; 2];
                continue;
            }

            let mut s = track.sample_at(self.pos);
            if fade > 0.0 && self.pos > fade_start {
                let p = ((self.pos - fade_start) / fade) as f32;
                let (out_gain, in_gain) = self.repeat_curve.gains(p * 2.0 - 1.0);
                let head = track.sample_at(self.pos - fade_start);
                s = [s[0] * out_gain + head[0] * in_gain, s[1] * out_gain + head[1] * in_gain];
            }
            // Der Tonabnehmer reagiert auf Geschwindigkeit: langsame Platte klingt leiser
            let amp = self.rate.abs().min(1.0);
            *frame = [s[0] * amp, s[1] * amp];

            self.pos += self.rate as f64 * step_scale;

            if let Some((a, b)) = self.loop_range
                && self.rate > 0.0 && self.pos >= b && self.scratch.is_none() {
                    self.pos -= b - a;
                }
            if self.pos < 0.0 {
                self.pos = 0.0;
                if self.scratch.is_none() {
                    self.rate = 0.0;
                }
            } else if self.pos >= len {
                if self.scratch.is_some() {
                    self.pos = len;
                } else if self.repeat_track {
                    // Mit Überblendung ist der Anfang schon hörbar, es geht hinter dem Blendstück weiter
                    self.pos -= fade_start;
                } else {
                    self.pos = len;
                    self.playing = false;
                    self.speed = 0.0;
                    self.rate = 0.0;
                    self.ended = true;
                    out[i + 1..].fill([0.0; 2]);
                    return;
                }
            }
        }

        self.speed = speed_end;
        if self.stop_pending && !self.playing && self.speed == 0.0 && self.rate.abs() < 1e-3 {
            self.stop_pending = false;
            self.rate = 0.0;
            self.pos = self.cue;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 1000.0;

    fn deck_with_dc(secs: usize) -> Deck {
        let mut d = Deck::default();
        d.load(Arc::new(TrackAudio::new(SR as u32, vec![[0.5; 2]; secs * SR as usize])), 1, false);
        d
    }

    fn run(d: &mut Deck, motor: &Motor, secs: f32) {
        let mut buf = vec![[0.0f32; 2]; 10];
        for _ in 0..(secs * SR / 10.0) as usize {
            d.render(motor, SR, &mut buf);
        }
    }

    #[test]
    fn play_ramps_up_instead_of_jumping() {
        let motor = Motor { spin_up_secs: 1.0, spin_down_secs: 1.0 };
        let mut d = deck_with_dc(10);
        d.play();
        run(&mut d, &motor, 0.5);
        assert!(d.speed() > 0.4 && d.speed() < 0.6, "speed {}", d.speed());
        run(&mut d, &motor, 1.0);
        assert_eq!(d.speed(), 1.0);
    }

    #[test]
    fn pause_spins_down_and_keeps_position() {
        let motor = Motor { spin_up_secs: 0.01, spin_down_secs: 1.0 };
        let mut d = deck_with_dc(10);
        d.play();
        run(&mut d, &motor, 1.0);
        d.pause();
        run(&mut d, &motor, 2.0);
        assert_eq!(d.speed(), 0.0);
        // 1 s Spielzeit plus etwa 0,5 s Auslauf (Dreieck unter der Rampe)
        let p = d.position_secs();
        assert!(p > 1.4 && p < 1.6, "pos {p}");
    }

    #[test]
    fn stop_returns_to_cue_after_spin_down() {
        let motor = Motor { spin_up_secs: 0.01, spin_down_secs: 0.2 };
        let mut d = deck_with_dc(10);
        d.seek_secs(2.0);
        d.cue();
        d.play();
        run(&mut d, &motor, 1.0);
        d.stop();
        run(&mut d, &motor, 0.5);
        assert!(!d.is_playing());
        assert!((d.position_secs() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn scratch_follows_hand_backwards() {
        let motor = Motor::default();
        let mut d = deck_with_dc(10);
        d.seek_secs(5.0);
        d.scratch_begin();
        d.scratch_to_secs(4.0);
        run(&mut d, &motor, 0.5);
        assert!((d.position_secs() - 4.0).abs() < 0.05, "pos {}", d.position_secs());
    }

    #[test]
    fn track_end_stops_and_reports() {
        let motor = Motor { spin_up_secs: 0.01, spin_down_secs: 0.01 };
        let mut d = deck_with_dc(1);
        d.play();
        run(&mut d, &motor, 1.5);
        assert!(!d.is_playing());
        assert!(d.take_ended());
        assert!(!d.take_ended());
    }

    #[test]
    fn repeat_track_wraps_around() {
        let motor = Motor { spin_up_secs: 0.01, spin_down_secs: 0.01 };
        let mut d = deck_with_dc(1);
        d.set_repeat_track(true);
        d.play();
        run(&mut d, &motor, 1.5);
        assert!(d.is_playing());
        assert!(d.position_secs() < 0.6);
    }

    #[test]
    fn repeat_track_fades_end_into_start() {
        let motor = Motor { spin_up_secs: 0.001, spin_down_secs: 0.001 };
        // Erste Hälfte 0.2, zweite 0.8: in der Blende mischen sich Ende und Anfang
        let mut samples = vec![[0.2f32; 2]; 3000];
        samples[1500..].fill([0.8; 2]);
        let mut d = Deck::default();
        d.load(Arc::new(TrackAudio::new(SR as u32, samples)), 1, false);
        d.set_repeat_track(true);
        d.set_repeat_fade(0.5, Curve::Linear);
        d.play();
        run(&mut d, &motor, 2.6);
        assert!(d.position_secs() > 2.5);
        let mut buf = vec![[0.0f32; 2]; 10];
        d.render(&motor, SR, &mut buf);
        let mid = buf[5][0];
        assert!(mid > 0.25 && mid < 0.75, "Blende mischt nicht: {mid}");
        // Nach dem Ende geht es hinter dem schon gehörten Anfang weiter
        run(&mut d, &motor, 0.5);
        assert!(d.is_playing());
        let p = d.position_secs();
        assert!(p > 0.5 && p < 0.7, "pos {p}");
    }
}
