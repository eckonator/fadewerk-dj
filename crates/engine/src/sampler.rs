// SPDX-License-Identifier: GPL-3.0-or-later
//! Effekt-Pads: kurze Klänge, die über Master oder nur auf dem Kopfhörer abgespielt werden.

use std::sync::Arc;

use crate::track::TrackAudio;

/// Anzahl der Pads.
pub const FX_SLOTS: usize = 8;

#[derive(Clone, Copy, Default)]
struct Voice {
    /// Lesekopf in Frames des Klangs.
    pos: f64,
    playing: bool,
    /// Über Master (sonst nur auf dem Kopfhörer, zum Vorhören).
    to_master: bool,
}

/// Spielt die Klänge der Pads. Erneutes Auslösen startet den Klang von vorn.
pub struct Sampler {
    sounds: [Option<Arc<TrackAudio>>; FX_SLOTS],
    voices: [Voice; FX_SLOTS],
    volume: f32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self { sounds: Default::default(), voices: [Voice::default(); FX_SLOTS], volume: 0.8 }
    }
}

impl Sampler {
    /// Legt einen Klang auf ein Pad und gibt den alten zurück (zum Freigeben außerhalb des Audio-Threads).
    pub fn load(&mut self, slot: usize, sound: Arc<TrackAudio>) -> Option<Arc<TrackAudio>> {
        let v = self.voices.get_mut(slot)?;
        *v = Voice::default();
        self.sounds[slot].replace(sound)
    }

    /// Startet den Klang eines Pads von vorn, über Master oder nur auf dem Kopfhörer.
    pub fn trigger(&mut self, slot: usize, to_master: bool) {
        if let Some(v) = self.voices.get_mut(slot)
            && self.sounds[slot].is_some()
        {
            *v = Voice { pos: 0.0, playing: true, to_master };
        }
    }

    pub fn stop(&mut self, slot: usize) {
        if let Some(v) = self.voices.get_mut(slot) {
            v.playing = false;
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
    }

    /// Bitmaske der Pads, die gerade klingen.
    pub fn playing_mask(&self) -> u32 {
        self.voices.iter().enumerate().filter(|(_, v)| v.playing).fold(0, |m, (i, _)| m | 1 << i)
    }

    /// Addiert die laufenden Klänge zu `master` bzw. `cue` (gleich lang).
    pub fn render_add(&mut self, out_rate: f32, master: &mut [[f32; 2]], cue: &mut [[f32; 2]]) {
        for (voice, sound) in self.voices.iter_mut().zip(&self.sounds) {
            let Some(sound) = sound else { continue };
            if !voice.playing {
                continue;
            }
            let step = sound.sample_rate as f64 / out_rate as f64;
            let len = sound.len_frames() as f64;
            let target = if voice.to_master { &mut *master } else { &mut *cue };
            for frame in target.iter_mut() {
                if voice.pos >= len {
                    voice.playing = false;
                    break;
                }
                let s = sound.sample_at(voice.pos);
                frame[0] += s[0] * self.volume;
                frame[1] += s[1] * self.volume;
                voice.pos += step;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_plays_once_and_restarts() {
        let mut s = Sampler::default();
        s.set_volume(1.0);
        assert!(s.load(0, Arc::new(TrackAudio::new(100, vec![[0.5; 2]; 10]))).is_none());
        let (mut m, mut c) = (vec![[0.0f32; 2]; 6], vec![[0.0f32; 2]; 6]);
        s.trigger(0, true);
        s.render_add(100.0, &mut m, &mut c);
        assert_eq!(m[5], [0.5, 0.5]);
        assert_eq!(c[0], [0.0, 0.0], "nichts auf dem Kopfhörer");
        assert_eq!(s.playing_mask(), 1);
        let (mut m, mut c) = (vec![[0.0f32; 2]; 6], vec![[0.0f32; 2]; 6]);
        s.render_add(100.0, &mut m, &mut c);
        assert_eq!(m[3], [0.5, 0.5]);
        assert_eq!(m[4], [0.0, 0.0], "nach 10 Frames zu Ende");
        assert_eq!(s.playing_mask(), 0);
        // Vorhören geht nur auf den Kopfhörer
        s.trigger(0, false);
        let (mut m, mut c) = (vec![[0.0f32; 2]; 2], vec![[0.0f32; 2]; 2]);
        s.render_add(100.0, &mut m, &mut c);
        assert_eq!((m[0], c[0]), ([0.0, 0.0], [0.5, 0.5]));
    }
}
