// SPDX-License-Identifier: GPL-3.0-or-later
//! Audio-Ausgabe, Kopfhörergerät, Aufnahme und Mixer-Zustand an die Engine.

use super::*;

impl FadeWerkDjApp {
    /// Bringt die Engine auf den Stand der Oberfläche (Start und nach einem Gerätewechsel).
    pub(super) fn push_mixer_state(&mut self) {
        self.ctl.send(Command::SetCrossfader(self.crossfader));
        self.split_sent = None;
        self.ctl.send(Command::SetCurve(self.curve));
        self.ctl.send(Command::SetRepeatFade(self.autofade.fade_secs));
        self.ctl.send(Command::SetMotor(self.motor));
        self.ctl.send(Command::SetMasterVolume(self.master_volume));
        self.ctl.send(Command::SetFxVolume(self.fx_volume));
        for (slot, sound) in self.fx.sounds.iter().enumerate() {
            if let Some(sound) = sound {
                self.ctl.send(Command::LoadFx { slot, sound: sound.clone() });
            }
        }
        self.ctl.send(Command::SetHeadVolume(self.head_volume));
        self.ctl.send(Command::SetHeadMix(self.head_mix));
        self.ctl.send(Command::SetHeadMode(self.head_mode));
        self.ctl.send(Command::SetLimiter(self.settings.limiter));
        self.push_mic_settings();
        for d in 0..2 {
            let v = &self.decks[d];
            self.ctl.send(Command::SetChannelVolume { deck: d, volume: v.volume });
            self.ctl.send(Command::SetPfl { deck: d, on: v.pfl });
            self.ctl.send(Command::SetPitch { deck: d, pitch: v.pitch_pct / 100.0 });
            self.ctl.send(Command::SetRepeatTrack { deck: d, on: v.repeat == Repeat::Track });
            self.ctl.send(Command::SetReverse { deck: d, on: v.reverse });
        }
        self.eq_sent = [[f32::NAN; 3]; 2];
        self.push_eq();
        for d in 0..2 {
            self.ctl.send(Command::SetFilter { deck: d, value: self.filter[d] });
        }
    }

    pub(super) fn push_mic_settings(&mut self) {
        self.ctl.send(Command::SetMicGain(self.settings.mic_gain));
        self.ctl.send(Command::SetTalkover { on: self.settings.talkover, depth: fw_engine::db_to_gain(self.settings.talkover_db) });
    }

    /// MIC-Taste. Geöffnet wird das Mikrofon erst hier, damit das System nicht ohne Grund nach der
    /// Mikrofon-Freigabe fragt.
    pub(super) fn toggle_mic(&mut self, now: f64) {
        if self.mic.take().is_some() {
            self.ctl.send(Command::SetMic(None));
            return;
        }
        self.start_mic(now);
    }

    pub(super) fn start_mic(&mut self, now: f64) {
        let Some(master) = &self.audio else {
            self.notify(tr("Ohne Audio-Ausgabe geht das Mikrofon nicht"), now);
            return;
        };
        match audio::start_mic(self.settings.mic_device.as_deref(), master.sample_rate) {
            Ok((mic, rx)) => {
                self.ctl.send(Command::SetMic(Some(rx)));
                self.notify(trf("Mikrofon an: {}", &[&mic.device]), now);
                self.mic = Some(mic);
            }
            Err(e) => {
                log::warn!("Mikrofon: {e:#}");
                self.notify(trf("Mikrofon lässt sich nicht öffnen: {}", &[&e]), now);
            }
        }
    }

    /// Aufnahme des Mix starten oder beenden.
    pub(super) fn toggle_recording(&mut self, now: f64) {
        if let Some(rec) = &self.recording {
            if !rec.is_stopping() {
                self.ctl.send(Command::SetRecording(false));
                rec.stop();
            }
            return;
        }
        let Some(rx) = self.record_rx.take() else {
            self.notify(tr("Aufnahme nicht möglich: keine Audio-Ausgabe"), now);
            return;
        };
        let dir = self.settings.record_dir.clone().unwrap_or_else(crate::recorder::default_dir);
        match crate::recorder::Recording::start(&dir, self.settings.record_format, self.ctl.shared.clone(), rx, self.audio_gen) {
            Ok(rec) => {
                self.record_dropped_at_start = self.ctl.shared.record_dropped();
                self.ctl.send(Command::SetRecording(true));
                log::info!("Aufnahme: {}", rec.path.display());
                self.notify(trf("Aufnahme läuft: {}", &[&file_label(&rec.path)]), now);
                self.recording = Some(rec);
            }
            Err((e, rx)) => {
                log::warn!("Aufnahme: {e:#}");
                self.record_rx = rx;
                self.notify(trf("Aufnahme nicht möglich: {}", &[&format!("{e:#}")]), now);
            }
        }
    }

    /// Fertige (oder abgebrochene) Aufnahme abschließen und melden.
    pub(super) fn poll_recording(&mut self, now: f64) {
        if !self.recording.as_ref().is_some_and(|r| r.is_done()) {
            return;
        }
        let Some(rec) = self.recording.take() else { return };
        // Endete der Thread wegen eines Fehlers, soll die Engine nicht weiter in den Puffer schreiben
        self.ctl.send(Command::SetRecording(false));
        let generation = rec.generation;
        let secs = rec.secs();
        let (rx, result) = rec.finish();
        if generation == self.audio_gen && self.record_rx.is_none() {
            self.record_rx = rx;
        }
        let lost = self.ctl.shared.record_dropped().saturating_sub(self.record_dropped_at_start);
        match result {
            Ok(files) => {
                let names: Vec<String> = files.iter().map(|f| file_label(f)).collect();
                log::info!("Aufnahme fertig ({secs:.0} s): {}", names.join(", "));
                let mut text = trf("Aufnahme gespeichert ({}): {}", &[&format_duration(secs), &names.join(", ")]);
                if lost > 0 && generation == self.audio_gen {
                    text.push_str(&trf(" – mit Aussetzern ({} ms fehlen)", &[&(lost * 1000 / self.ctl.shared.sample_rate.max(1) as u64)]));
                }
                self.notice = Some((text, now + 6.0));
            }
            Err(e) => {
                log::warn!("Aufnahme abgebrochen: {e:#}");
                self.notice = Some((trf("Aufnahme abgebrochen: {}", &[&format!("{e:#}")]), now + 8.0));
            }
        }
    }

    /// Schickt geänderte EQ-Werte an die Engine. Mit Bass-Swap sind während einer Überblendung erst
    /// die Bässe des neuen Titels aus, ab der Mitte die des alten.
    pub(super) fn push_eq(&mut self) {
        let mut swap = [1.0f32; 2];
        if self.bass_swap
            && let Some(f) = self.autofade.fade()
            && f.is_handover()
        {
            let late = f.progress() >= 0.5;
            swap[f.to] = if late { 1.0 } else { 0.0 };
            swap[f.from] = if late { 0.0 } else { 1.0 };
        }
        for (d, swap) in swap.into_iter().enumerate() {
            for band in 0..3 {
                let mut gain = if self.eq_kill[d][band] { 0.0 } else { db_to_gain(self.eq_db[d][band]) };
                if band == eq::LOW {
                    gain *= swap;
                }
                if gain != self.eq_sent[d][band] {
                    self.eq_sent[d][band] = gain;
                    self.ctl.send(Command::SetEq { deck: d, band, gain });
                }
            }
        }
    }

    /// Öffnet das eigene Kopfhörergerät aus den Einstellungen (falls gewählt) und verbindet es mit der Engine.
    /// Ohne `wanted` bleibt es zu: Ein Gerät, das beim Start fehlte und erst später auftaucht (etwa AirPods),
    /// soll beim Neustart der Ausgabe nicht unbemerkt mitlaufen, sondern erst nach erneuter Auswahl.
    pub(super) fn start_head_audio(&mut self, now: f64, wanted: bool) {
        self.head_audio = None;
        self.ctl.send(Command::SetHeadSink(None));
        let Some(id) = self.settings.head_device.clone() else {
            if self.head_mode == HeadMode::Device {
                self.set_head_mode(HeadMode::Split);
            }
            return;
        };
        if !wanted {
            if self.head_mode == HeadMode::Device {
                self.set_head_mode(HeadMode::Off);
            }
            return;
        }
        let Some(master) = self.audio.as_ref() else { return };
        if master.id == id {
            self.notify(tr("Das Kopfhörergerät ist dasselbe wie die Lautsprecher. Bitte ein anderes wählen oder im Mixer Split L/R nutzen."), now);
            return;
        }
        match audio::start_head(&id, master.sample_rate, self.settings.buffer_frames) {
            Ok((out, sink)) => {
                self.ctl.send(Command::SetHeadSink(Some(sink)));
                self.notify(trf("Kopfhörer: {}", &[&out.device]), now);
                self.head_audio = Some(out);
                // Wer ein Kopfhörergerät wählt, will es auch hören
                if matches!(self.head_mode, HeadMode::Off | HeadMode::Split | HeadMode::Outputs34) {
                    self.set_head_mode(HeadMode::Device);
                }
            }
            Err(e) => {
                log::warn!("Kopfhörergerät: {e:#}");
                self.notify(trf("Kopfhörergerät nicht verfügbar: {}", &[&format!("{e:#}")]), now);
                if self.head_mode == HeadMode::Device {
                    self.set_head_mode(HeadMode::Split);
                }
            }
        }
    }

    /// Modus für Vorhören, wenn der Kopfhörer noch aus ist: eigenes Gerät, sonst Split.
    pub(super) fn enable_headphones(&mut self, now: f64) {
        if self.head_mode != HeadMode::Off {
            return;
        }
        if self.head_audio.is_some() {
            self.set_head_mode(HeadMode::Device);
            self.notify(tr("Vorhören über das Kopfhörergerät"), now);
        } else {
            self.set_head_mode(HeadMode::Split);
            self.notify(tr("Kopfhörer: links Master, rechts Vorhören"), now);
        }
    }

    /// Testton aus den Einstellungen. Auf dem Kopfhörer geht er an das eigene Gerät, falls eines läuft,
    /// sonst an den im Mixer gewählten Weg (ist der Kopfhörer aus, wie beim Vorhören an).
    pub(super) fn test_tone(&mut self, to_master: bool, now: f64) {
        if !to_master {
            if self.head_audio.is_some() && self.head_mode != HeadMode::Device {
                self.set_head_mode(HeadMode::Device);
                self.notify(tr("Vorhören über das Kopfhörergerät"), now);
            } else {
                self.enable_headphones(now);
            }
        }
        self.ctl.send(Command::TestTone { to_master });
    }

    /// Exklusivmodus (macOS) für das laufende Master-Gerät, wenn eingestellt.
    pub(super) fn apply_exclusive(&mut self, now: f64) {
        if !self.settings.exclusive_output || !cfg!(target_os = "macos") {
            return;
        }
        let Some(audio) = &self.audio else { return };
        match crate::quiet::set_exclusive(&audio.id, true) {
            Ok(()) => log::info!("Ausgabegerät exklusiv: {}", audio.device),
            Err(e) => {
                log::warn!("Exklusivmodus: {e}");
                self.notify(trf("Ausgabegerät lässt sich nicht exklusiv belegen: {}", &[&e]), now);
            }
        }
    }

    pub(super) fn release_exclusive(&self) {
        if let Some(audio) = &self.audio
            && let Err(e) = crate::quiet::set_exclusive(&audio.id, false)
        {
            log::warn!("Exklusivmodus freigeben: {e}");
        }
    }

    /// Startet die Audio-Ausgabe mit den aktuellen Einstellungen neu. Geladene Titel laufen an derselben Stelle weiter.
    /// `done`: Hinweis bei Erfolg statt „Audiogerät gewechselt“.
    /// Meldet der Audio-Stream Fehler in Massen, hängt er (beobachtet auf einem Netbook: ALSA über PulseAudio
    /// nach einem Aussetzer, danach nur noch `snd_pcm_avail_delay`-Fehler, Ton stottert, Audio-Thread bei 80 %).
    /// Dann die Ausgabe neu öffnen; die Titel laufen an derselben Stelle weiter. Höchstens alle 10 s.
    pub(super) fn check_audio_errors(&mut self, now: f64) {
        let Some(count) = self.audio.as_ref().map(|a| a.errors.load(std::sync::atomic::Ordering::Relaxed)) else { return };
        let (seen, at) = self.audio_errors_seen;
        if count < seen {
            // Neuer Stream, der Zähler beginnt von vorn
            self.audio_errors_seen = (count, now);
            return;
        }
        if now - at < 1.0 {
            return;
        }
        self.audio_errors_seen = (count, now);
        let per_sec = (count - seen) as f64 / (now - at);
        if per_sec >= AUDIO_ERROR_STORM && now - self.audio_restarted >= 10.0 {
            log::warn!("Audio-Ausgabe hängt ({per_sec:.0} Fehler/s), starte sie neu");
            self.audio_restarted = now;
            self.restart_audio(now, Some(tr("Die Audio-Ausgabe hing und wurde neu gestartet").to_owned()));
            self.audio_errors_seen = (0, now);
        }
    }

    pub(super) fn restart_audio(&mut self, now: f64, done: Option<String>) {
        let st: Vec<(f64, bool)> = (0..2).map(|d| (self.ctl.shared.decks[d].position_secs(), self.ctl.shared.decks[d].is_playing())).collect();
        // Kopfhörergerät nur wieder öffnen, wenn es lief (siehe `start_head_audio`)
        let head_was_on = self.head_audio.is_some();
        let mic_was_on = self.mic.take().is_some();
        // Eine laufende Aufnahme gehört zur alten Ausgabe: sauber abschließen
        if let Some(rec) = &self.recording
            && !rec.is_stopping()
        {
            rec.stop();
            self.notify(tr("Aufnahme beendet, weil die Audio-Ausgabe neu startet"), now);
        }
        // Alte Geräte zuerst freigeben, manche Treiber erlauben nur einen Stream
        self.release_exclusive();
        self.head_audio = None;
        self.audio = None;
        let (audio, ctl, err) = audio::start(self.settings.output_device.as_deref(), self.settings.buffer_frames);
        self.audio = audio;
        self.ctl = ctl;
        self.audio_gen += 1;
        self.record_rx = self.ctl.take_recording();
        self.audio_error = err.clone();
        self.push_mixer_state();
        self.start_head_audio(now, head_was_on);
        if mic_was_on {
            self.start_mic(now);
        }
        for (d, &(pos, playing)) in st.iter().enumerate() {
            self.decks[d].ended_seen = 0;
            self.decks[d].reverse_ended_seen = 0;
            if let Some(track) = self.decks[d].audio.clone() {
                let id = self.decks[d].loaded_id;
                self.ctl.send(Command::Load { deck: d, id, track, keep_playing: false, start_secs: pos });
                let range = self.decks[d].loop_range;
                self.ctl.send(Command::SetLoop { deck: d, range });
                // Ohne Anlauf weiter: ein Gerätewechsel oder Neustart soll nicht wie ein neuer Start klingen
                if playing {
                    self.ctl.send(Command::Resume { deck: d });
                }
            }
        }
        self.preview = None;
        match err {
            Some(e) => self.notify(e, now),
            None => {
                let name = self.audio.as_ref().map_or(String::new(), |a| a.device.clone());
                self.notify(done.unwrap_or_else(|| trf("Audiogerät gewechselt: {}", &[&name])), now);
            }
        }
        // Zuletzt, damit ein Fehler dabei den Hinweis oben ersetzt
        if self.quiet.is_some() {
            self.apply_exclusive(now);
        }
    }
}
