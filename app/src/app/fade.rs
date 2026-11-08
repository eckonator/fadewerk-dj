// SPDX-License-Identifier: GPL-3.0-or-later
//! Auto-Fade, Crossfader, Übergangsleiste, FadeWerk-Vorhören und Auto-Sync.

use super::*;

impl FadeWerkDjApp {
    /// Pegel von A und B am Crossfader, wie die Engine sie gerade rechnet.
    pub(super) fn crossfader_gains(&self) -> (f32, f32) {
        if self.split_sent == Some(true) { self.curve.split_gains(self.crossfader) } else { self.curve.gains(self.crossfader) }
    }

    pub(super) fn on_air(&self, d: usize) -> bool {
        let st = &self.ctl.shared.decks[d];
        let (ga, gb) = self.crossfader_gains();
        let g = if d == 0 { ga } else { gb };
        self.has_track(d) && (st.is_playing() || st.speed() != 0.0) && g * self.decks[d].volume > 0.05
    }

    pub(super) fn set_crossfader(&mut self, x: f32) {
        self.crossfader = x.clamp(-1.0, 1.0);
        self.ctl.send(Command::SetCrossfader(self.crossfader));
    }

    pub(super) fn set_head_mode(&mut self, mode: HeadMode) {
        self.head_mode = mode;
        self.ctl.send(Command::SetHeadMode(mode));
    }

    /// Titel vor und nach Eintrag `i` der Playlist `d` in der anderen Playlist, wenn abwechselnd gespielt wird,
    /// ausgehend vom aktuellen Stand (aktiver Player, geladene Titel).
    pub(super) fn fade_partners(&self, d: usize, i: usize) -> [Option<(usize, usize)>; 2] {
        let a = self.autofade.active;
        let o = 1 - a;
        let ca = self.playlists[a].current.unwrap_or(0) as isize;
        // Als Nächstes aus der anderen Playlist: der geladene Titel, wenn er noch dran ist, sonst der folgende
        let no = match self.playlists[o].current {
            Some(c) if self.has_track(o) && !self.decks[o].spent => c as isize,
            Some(c) => c as isize + 1,
            None => 0,
        };
        let i = i as isize;
        let (other, before, after) = if d == a { (o, no + (i - ca) - 1, no + (i - ca)) } else { (a, ca + (i - no), ca + (i - no) + 1) };
        let ok = |k: isize| (k >= 0 && (k as usize) < self.playlists[other].entries.len()).then_some((other, k as usize));
        [ok(before), ok(after)]
    }

    /// Nachbartitel und Vorhör-Stand für den offenen Fade-Editor.
    pub(super) fn editor_audio(&self, ctx: &egui::Context) -> fade_preview::EditorAudio {
        let Some((d, i)) = playlist_ui::open_editor(ctx).filter(|_| self.fadewerk) else { return Default::default() };
        let name = |(l, k): (usize, usize)| {
            let path = &self.playlists[l].entries[k];
            let m = self.metas.get(path).cloned().unwrap_or_else(|| TrackMeta::from_filename(path));
            fade_preview::Partner { list: l, name: m.title }
        };
        let partners = self.fade_partners(d, i).map(|p| p.map(name));
        let playing = self.preview.as_ref().and_then(|p| p.clip.as_ref()).filter(|c| c.list == d && c.index == i).map(|c| {
            let pos = c.rendering.is_none().then(|| (self.ctl.shared.decks[PREVIEW].position_secs() + c.offset) as f32);
            (c.kind, pos)
        });
        fade_preview::EditorAudio { partners, playing }
    }

    /// Mischt den Vorhör-Ausschnitt für den Fade-Editor im Hintergrund; gespielt wird er, sobald er fertig ist.
    pub(super) fn start_fade_preview(&mut self, d: usize, i: usize, kind: fade_preview::Kind, now: f64) {
        use fade_preview::Kind;
        let Some(path) = self.playlists[d].entries.get(i).cloned() else { return };
        let source = |app: &Self, l: usize, k: usize| {
            let path = app.playlists[l].entries[k].clone();
            // Liegt der Titel schon in einem Player, muss er nicht noch einmal dekodiert werden
            let audio = app.decks.iter().find(|v| v.path.as_ref() == Some(&path)).and_then(|v| v.audio.clone());
            fade_preview::Source { path, audio, fade: app.playlists[l].fade(k) }
        };
        let partner = match kind {
            Kind::From => self.fade_partners(d, i)[0],
            Kind::To => self.fade_partners(d, i)[1],
            _ => None,
        };
        if matches!(kind, Kind::From | Kind::To) && partner.is_none() {
            return;
        }
        let job = fade_preview::Job {
            kind,
            edited: source(self, d, i),
            partner: partner.map(|(l, k)| source(self, l, k)),
            curve: self.curve,
            fade_secs: self.autofade.fade_secs,
        };
        self.ctl.send(Command::Eject { deck: PREVIEW });
        let token = self.token();
        let clip = ClipView { list: d, index: i, kind, rendering: Some(fade_preview::spawn(job)), offset: 0.0 };
        self.preview = Some(PreviewView { path, token, ended_seen: 0, clip: Some(clip) });
        self.enable_headphones(now);
    }

    /// Fertig gemischten Vorhör-Ausschnitt abspielen.
    pub(super) fn poll_fade_preview(&mut self, now: f64) {
        let Some(p) = self.preview.as_mut() else { return };
        let Some(c) = p.clip.as_mut() else { return };
        let Some(rx) = &c.rendering else { return };
        match rx.try_recv() {
            Ok(Ok(clip)) => {
                c.rendering = None;
                c.offset = clip.offset;
                let token = p.token;
                self.ctl.send(Command::Load { deck: PREVIEW, id: token, track: clip.audio, keep_playing: false, start_secs: 0.0 });
                self.ctl.send(Command::Play { deck: PREVIEW });
            }
            Ok(Err(e)) => {
                log::warn!("Vorhören im Fade-Editor: {e}");
                self.preview = None;
                self.notify(tr("Vorhören nicht möglich, der Titel lässt sich nicht lesen"), now);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.preview = None,
        }
    }

    /// Wie die nächste Überblendung vom aktiven Player aus abläuft (FadeWerk-Bereiche oder Überblendzeit).
    pub(super) fn transition(&self) -> crate::fadewerk::Transition {
        let (a, o) = (self.autofade.active, self.autofade.other());
        let fade_of = |d: usize| self.playlists[d].current.map(|i| self.playlists[d].fade(i)).unwrap_or_default();
        let old = if self.has_track(a) { fade_of(a) } else { Default::default() };
        let ready = self.has_track(o) && !self.decks[o].spent && !self.loading(o);
        let new = if ready { fade_of(o) } else { Default::default() };
        let rate = |d: usize| 1.0 + self.decks[d].pitch_pct / 100.0;
        let dur = self.ctl.shared.decks[a].duration_secs();
        crate::fadewerk::plan(old, dur, new, self.autofade.fade_secs, rate(a), rate(o))
    }

    pub(super) fn step_autofade(&mut self, dt: f32) {
        self.step_fade(dt);
        // Die Engine wertet den Crossfader zweiphasig aus, solange eine FadeWerk-Überblendung läuft
        let split = self.autofade.fade().is_some_and(|f| f.is_split());
        if self.split_sent != Some(split) {
            self.split_sent = Some(split);
            self.ctl.send(Command::SetSplitFade(split));
        }
        if self.autofade.fade().is_none() {
            self.fade_track = None;
        }
    }

    pub(super) fn step_fade(&mut self, dt: f32) {
        // FadeWerk: die Phasen richten sich nach der Stelle im alten Titel, damit er genau am Ende seines
        // Ausblendbereichs still ist (auch mit Pitch). Steht der alte Titel, laufen sie nach Zeit weiter.
        if let Some((d, out_start, out_end)) = self.fade_track
            && self.ctl.shared.decks[d].is_playing()
        {
            let pos = self.ctl.shared.decks[d].position_secs();
            let rate = (1.0 + self.decks[d].pitch_pct as f64 / 100.0).max(0.05);
            let out = ((out_end - out_start) / rate) as f32;
            let left = if self.autofade.in_second_phase() { (out_end - pos) / rate } else { (out_start - pos) / rate };
            self.autofade.retime(left as f32, out);
        }
        if let Some(step) = self.autofade.step(dt) {
            self.set_crossfader(step.crossfader);
            if let Some(old) = step.finished {
                log::info!("Überblendung fertig, Player {} hält bei {:.2} s", theme::deck_letter(old), self.ctl.shared.decks[old].position_secs());
                self.ctl.send(Command::Pause { deck: old });
                if !self.load_next(old, false, false) {
                    self.decks[old].spent = true;
                }
            }
            return;
        }
        let a = self.autofade.active;
        let st = &self.ctl.shared.decks[a];
        if !self.has_track(a) || self.decks[a].repeat == Repeat::Track || self.decks[a].loop_range.is_some() {
            return;
        }
        let plan = self.transition();
        let rate = 1.0 + self.decks[a].pitch_pct as f64 / 100.0;
        let pos = st.position_secs();
        let remaining = (plan.end - pos) / rate;
        // Im Takt: an der letzten Phrasengrenze vor dem geplanten Beginn starten; bei FadeWerk-Bereichen nur am
        // letzten Taktbeginn davor (die Bereiche legen den Beginn fest, höchstens ein Takt früher)
        let planned = plan.end - plan.lead as f64 * rate;
        let on_beat = self.beat_plan(planned);
        let start = match on_beat {
            Some(bp) if plan.split.is_some() => self.autofade.should_start(st.is_playing(), remaining, plan.lead) || pos >= Self::bar_start(bp.old, planned),
            Some(bp) => self.autofade.should_start(st.is_playing(), remaining, plan.lead) || pos >= Self::phrase_start(bp.old, planned),
            None => self.autofade.should_start(st.is_playing(), remaining, plan.lead),
        };
        if start && self.autofade.enabled && self.autofade.fade().is_none() && st.is_playing() {
            self.start_fade(false, 0.0);
        }
    }

    pub(super) fn start_fade(&mut self, manual: bool, now: f64) {
        if self.autofade.fade().is_some() {
            return;
        }
        let to = self.autofade.other();
        if self.has_track(to) && !self.decks[to].spent && !self.loading(to) {
            let from = self.autofade.active;
            let transition = self.transition();
            // Im Takt: ohne Motor-Anlauf an der passenden Stelle, Länge in ganzen Takten. Mit FadeWerk-Bereichen
            // ebenso im Takt starten, die Phasen der Bereiche bleiben
            let pos = self.ctl.shared.decks[from].position_secs();
            let beat = self.ctl.shared.decks[from].is_playing().then(|| self.beat_plan(pos)).flatten();
            if transition.split.is_none()
                && let Some(bp) = beat
            {
                let secs = self.start_on_beat(bp);
                let saved = std::mem::replace(&mut self.autofade.fade_secs, secs);
                self.autofade.start(self.crossfader);
                self.autofade.fade_secs = saved;
                return;
            }
            match beat {
                Some(bp) => {
                    self.start_on_beat(bp);
                }
                None => self.ctl.send(Command::Play { deck: to }),
            }
            match transition.split {
                // Zweiphasig nur aus der Endstellung, sonst sprängen die Pegel beim Umschalten der Auswertung
                Some(sp) if self.crossfader == fw_engine::automix::side(from) => {
                    self.autofade.start_split(self.crossfader, sp.in_secs, sp.out_secs);
                    log::info!(
                        "FadeWerk-Überblendung {}→{}: {:.1} s ein, {:.1} s aus ({:.1}–{:.1}), alter Titel bei {:.1}",
                        theme::deck_letter(from), theme::deck_letter(to), sp.in_secs, sp.out_secs, sp.out_start, sp.out_end,
                        self.ctl.shared.decks[from].position_secs()
                    );
                    self.fade_track = (!manual).then_some((from, sp.out_start, sp.out_end));
                }
                _ => self.autofade.start(self.crossfader),
            }
        } else if self.decks[to].pending.is_none() && !self.load_next(to, false, false) && manual {
            self.notify(trf("Playlist {} hat keinen nächsten Titel", &[&theme::deck_letter(to)]), now);
        }
    }

    /// Pitch, mit dem Player `d` im Tempo des anderen läuft, und der Tempofaktor dazu. Ohne `factor` wird der
    /// nächstliegende aus 1, 2 und ½ gewählt (70 und 140 BPM passen zusammen). `None` ohne BPM.
    pub(super) fn sync_pitch(&self, d: usize, factor: Option<f32>) -> Option<(f32, f32)> {
        let o = 1 - d;
        // Mit Beatraster das genaue Tempo (sonst laufen die Schläge über eine lange Überblendung auseinander)
        let mine = self.tempo_of(d)? as f32;
        let theirs = self.tempo_of(o).filter(|_| self.has_track(o))? as f32 * (1.0 + self.decks[o].pitch_pct / 100.0);
        let pct = |f: f32| (theirs / (mine * f) - 1.0) * 100.0;
        let f = factor.unwrap_or_else(|| [1.0, 2.0, 0.5].into_iter().min_by(|a, b| pct(*a).abs().total_cmp(&pct(*b).abs())).unwrap_or(1.0));
        Some((pct(f), f))
    }

    pub(super) fn send_pitch(&mut self, d: usize, pct: f32) {
        self.decks[d].pitch_pct = pct;
        self.ctl.send(Command::SetPitch { deck: d, pitch: pct / 100.0 });
    }

    /// Auto-Sync: startet ein Player, während der andere zu hören ist, übernimmt er dessen Tempo und hält es,
    /// solange der andere zu hören ist (also auch während der Überblendung). Ist er allein auf Sendung, läuft sein
    /// Pitch mit `AUTO_SYNC_RATE` auf 0 % zurück. Steht er still und ist nicht zu hören, geht der Pitch sofort auf 0.
    pub(super) fn step_auto_sync(&mut self, dt: f32, now: f64) {
        for d in 0..2 {
            let o = 1 - d;
            let playing = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
            let started = playing && !self.decks[d].was_playing;
            self.decks[d].was_playing = playing;
            if !self.auto_sync {
                continue;
            }
            if started && self.on_air(o) {
                self.decks[d].sync = None;
                if let Some((pct, factor)) = self.sync_pitch(d, None) {
                    if pct.abs() <= PITCH_RANGE {
                        self.decks[d].sync = Some(factor);
                        self.send_pitch(d, pct);
                    } else {
                        let theirs = self.decks[o].bpm.unwrap_or(0.0) * (1.0 + self.decks[o].pitch_pct / 100.0);
                        let mine = self.decks[d].bpm.unwrap_or(0.0);
                        self.notify(trf("Auto-Sync: {} ↔ {} BPM liegen zu weit auseinander", &[&format!("{mine:.1}"), &format!("{theirs:.1}")]), now);
                    }
                }
            }
            let Some(factor) = self.decks[d].sync else { continue };
            if !playing {
                if !self.on_air(d) {
                    self.decks[d].sync = None;
                    self.send_pitch(d, 0.0);
                }
            } else if self.on_air(o) {
                // Dem anderen folgen, auch wenn dessen Pitch sich ändert; nach einem Übergang im Takt auch die Schläge
                let lock = if self.beat_lock == Some(d) { self.phase_correction(d, factor) } else { 0.0 };
                if let Some((pct, _)) = self.sync_pitch(d, Some(factor))
                    && pct.abs() <= PITCH_RANGE
                    && (pct + lock - self.decks[d].pitch_pct).abs() > 0.0005
                {
                    self.send_pitch(d, pct + lock);
                }
            } else {
                if self.beat_lock == Some(d) {
                    self.beat_lock = None;
                }
                let p = self.decks[d].pitch_pct;
                // Passt der nächste Übergang nur mit dem jetzigen Pitch in den Takt, bleibt er bis dahin
                if d == self.autofade.active && self.keep_pitch_for_beat() {
                    continue;
                }
                // Sonst soll der Titel beim geplanten Übergang im natürlichen Tempo laufen: Damit rechnen Sortierung
                // und Übergang im Takt. Bleibt dafür zu wenig Zeit, schneller zurück
                let rate = match self.fade_plan() {
                    FadePlan::Planned { rem, .. } if d == self.autofade.active => {
                        let left = (rem - PITCH_ZERO_MARGIN_SECS).max(1.0) as f32;
                        (p.abs() / left).clamp(AUTO_SYNC_RATE, AUTO_SYNC_RATE_MAX)
                    }
                    _ => AUTO_SYNC_RATE,
                };
                let step = rate * dt;
                if p.abs() <= step {
                    self.decks[d].sync = None;
                    self.send_pitch(d, 0.0);
                } else {
                    self.send_pitch(d, p - step * p.signum());
                }
            }
        }
    }

    /// Was Auto-Fade gerade vorhat. Eine Auswertung für Statustext und Übergangsleiste.
    pub(super) fn fade_plan(&self) -> FadePlan {
        if let Some(f) = self.autofade.fade() {
            return FadePlan::Fading { from: f.from, to: f.to, rest: f.remaining_secs() };
        }
        if !self.autofade.enabled {
            return FadePlan::Off;
        }
        let a = self.autofade.active;
        let st = &self.ctl.shared.decks[a];
        if !self.has_track(a) || !st.is_playing() {
            return FadePlan::Stopped(a);
        }
        if self.decks[a].repeat == Repeat::Track || self.decks[a].loop_range.is_some() {
            return FadePlan::Waiting(a);
        }
        let o = 1 - a;
        let other_empty = (!self.has_track(o) || self.decks[o].spent)
            && self.decks[o].pending.is_none()
            && self.playlists[o].next_index(self.decks[o].repeat == Repeat::List).is_none();
        if other_empty {
            return FadePlan::Alone(a);
        }
        let plan = self.transition();
        let rem = (plan.end - st.position_secs()) / (1.0 + self.decks[a].pitch_pct as f64 / 100.0) - plan.lead as f64;
        FadePlan::Planned { to: o, rem: rem.max(0.0), plan }
    }

    /// Statustext zu Auto-Fade (in der Übergangsleiste, solange nichts geplant ist).
    pub(super) fn fade_status(&self) -> String {
        let letter = theme::deck_letter;
        match self.fade_plan() {
            FadePlan::Fading { from, to, rest } => trf("Blende von {} auf {}, noch {} s", &[&letter(from), &letter(to), &format!("{rest:.1}")]),
            FadePlan::Off => tr("Auto-Fade aus, Crossfader per Hand").into(),
            FadePlan::Stopped(a) => trf("Player {} starten, dann blendet Auto-Fade selbst", &[&letter(a)]),
            FadePlan::Waiting(a) => trf("Repeat oder Loop in {}: Auto-Fade wartet", &[&letter(a)]),
            FadePlan::Alone(a) => trf("Playlist {} ist durch, {} spielt allein weiter", &[&letter(1 - a), &letter(a)]),
            FadePlan::Planned { to, rem, .. } => trf("Auto-Fade zu {} in {}", &[&letter(to), &deck_ui::fmt_time(rem)]),
        }
    }

    /// Anzeigename eines Titels: Titel aus den Tags, sonst der Dateiname ohne Endung.
    pub(super) fn title_of(&self, path: &Path) -> String {
        match self.metas.get(path).map(|m| m.title.as_str()).filter(|t| !t.is_empty()) {
            Some(t) => t.to_owned(),
            None => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    /// Titel, der beim nächsten Übergang auf Player `d` hereinkommt: der geladene, sonst der nächste der Playlist.
    pub(super) fn incoming_title(&self, d: usize) -> String {
        if self.has_track(d) && !self.decks[d].spent
            && let Some(m) = self.decks[d].meta.as_ref().filter(|m| !m.title.is_empty())
        {
            return m.title.clone();
        }
        let next = if self.has_track(d) && !self.decks[d].spent { self.playlists[d].current } else { self.playlists[d].next_index(self.decks[d].repeat == Repeat::List) };
        next.and_then(|i| self.playlists[d].entries.get(i)).map(|p| self.title_of(p)).unwrap_or_default()
    }

    /// Übergangsleiste in der Mitte: geplanter oder laufender Übergang und was danach kommt.
    pub(super) fn transition_view(&self, status: String) -> mixer_ui::TransitionView {
        let mut v = mixer_ui::TransitionView {
            status,
            from: self.autofade.active,
            until: None,
            lead: self.autofade.fade_secs,
            fade_secs: self.autofade.fade_secs,
            split_in: None,
            fading: None,
            next: String::new(),
            after: None,
            curve: self.curve,
            bass_swap: self.bass_swap,
            on_beat: false,
            beat_miss: None,
        };
        if let Some(f) = self.autofade.fade() {
            v.on_beat = self.beat_lock == Some(f.to);
            let (progress, rest) = (f.progress(), f.remaining_secs());
            v.from = f.from;
            v.fading = Some((progress, rest));
            v.lead = if progress < 0.97 { (rest / (1.0 - progress)).max(0.5) } else { self.autofade.fade_secs };
        } else if let FadePlan::Planned { rem, plan, .. } = self.fade_plan() {
            let rate = 1.0 + self.decks[self.autofade.active].pitch_pct as f64 / 100.0;
            let check = self.beat_check(plan.end - plan.lead as f64 * rate);
            v.on_beat = check.is_ok();
            // Grund nur, wenn AUTO SYNC an ist (sonst ist „ohne Takt“ ohnehin gewollt)
            v.beat_miss = check.err().filter(|_| self.auto_sync).and_then(|m| m.texts());
            v.until = Some(rem);
            v.lead = plan.lead;
            v.split_in = plan.split.map(|s| s.in_secs);
        } else {
            return v;
        }
        let (from, to) = (v.from, 1 - v.from);
        v.next = self.incoming_title(to);
        // Nach der Übergabe lädt der jetzt spielende Player seinen nächsten Titel
        v.after = self.playlists[from]
            .next_index(self.decks[from].repeat == Repeat::List)
            .and_then(|i| self.playlists[from].entries.get(i))
            .map(|p| (from, self.title_of(p)));
        v
    }
}

/// Was Auto-Fade gerade vorhat (siehe `FadeWerkDjApp::fade_plan`).
pub(super) enum FadePlan {
    /// Blendet gerade von `from` auf `to`, noch `rest` Sekunden.
    Fading { from: usize, to: usize, rest: f32 },
    /// Auto-Fade ist aus.
    Off,
    /// Der aktive Player steht oder ist leer.
    Stopped(usize),
    /// Repeat (Titel) oder Loop im aktiven Player: wartet.
    Waiting(usize),
    /// Die andere Playlist ist durch, der aktive spielt allein weiter.
    Alone(usize),
    /// Überblendung zu `to` beginnt in `rem` Sekunden.
    Planned { to: usize, rem: f64, plan: crate::fadewerk::Transition },
}

impl FadeWerkDjApp {
    /// FadeWerk-Bereiche für einen Titel vorschlagen, sobald Skizze und Beatraster da sind (fehlt eines, wird es
    /// angefordert). `index` = einzelner Eintrag (überschreibt), sonst alle Einträge des Titels ohne eigene Bereiche.
    pub(super) fn queue_fade_suggestion(&mut self, list: usize, path: PathBuf, index: Option<usize>, now: f64) {
        if self.fade_suggest.is_empty() {
            self.fade_suggested = 0;
        }
        if !self.sketch.map.contains_key(&path) && self.sketch.requested.insert(path.clone()) {
            self.sketch.batch[list].push(path.clone());
        }
        if !self.grids.contains_key(&path) && self.bpm_pending.insert(path.clone()) {
            self.bpm_total += 1;
            self.lib.analyze(path.clone(), false, true);
        }
        self.fade_suggest.push((list, path, index));
        self.step_fade_suggestions(now);
        if !self.fade_suggest.is_empty() && index.is_none() && self.fade_suggest.len() == 1 {
            self.notify(tr("Fades werden vorgeschlagen, sobald Wellenform und Beatraster da sind …"), now);
        }
    }

    /// Wartende Vorschläge setzen, deren Skizze und Beatraster inzwischen da sind. Unlesbare Titel fallen weg; ohne
    /// sicheres Raster geht der Vorschlag nur nach der Lautstärke.
    pub(super) fn step_fade_suggestions(&mut self, now: f64) {
        if self.fade_suggest.is_empty() {
            return;
        }
        let fade_secs = self.autofade.fade_secs;
        let mut done = Vec::new();
        for (k, (list, path, index)) in self.fade_suggest.iter().enumerate() {
            let sketch = match self.sketch.map.get(path) {
                None => continue,
                Some(None) => {
                    done.push((k, None));
                    continue;
                }
                Some(Some(sk)) => sk.clone(),
            };
            // Raster abwarten, solange die Analyse noch läuft
            if !self.grids.contains_key(path) && self.bpm_pending.contains(path) {
                continue;
            }
            let fade = crate::fade_suggest::suggest(&sketch, self.grids.get(path).copied().flatten(), fade_secs);
            done.push((k, Some((*list, path.clone(), *index, fade))));
        }
        for (k, item) in done.into_iter().rev() {
            self.fade_suggest.remove(k);
            let Some((list, path, index, fade)) = item else { continue };
            if fade.is_empty() {
                continue;
            }
            let pl = &self.playlists[list];
            let targets: Vec<usize> = match index {
                Some(i) if pl.entries.get(i) == Some(&path) => vec![i],
                Some(_) => pl.entries.iter().position(|p| *p == path).into_iter().collect(),
                None => (0..pl.entries.len()).filter(|&i| pl.entries[i] == path && pl.fade(i).is_empty()).collect(),
            };
            for i in targets {
                self.playlist_action(list, PlaylistAction::SetFade(i, fade), now);
                self.fade_suggested += 1;
            }
        }
        if self.fade_suggest.is_empty() && self.fade_suggested > 1 {
            self.notify(trf("Fades für {} Titel vorgeschlagen", &[&self.fade_suggested]), now);
            self.fade_suggested = 0;
        }
    }
}
