// SPDX-License-Identifier: GPL-3.0-or-later
//! Aktionen der Oberflächenmodule (Player, Mixer, Auto-Fade, Playlists, Ordnerbrowser) ausführen.

use super::*;

impl FadeWerkDjApp {
    /// Player leeren: Titel aus der Engine nehmen, die Platte gleitet hinaus.
    pub(super) fn eject(&mut self, d: usize, now: f64) {
        self.ctl.send(Command::Eject { deck: d });
        let v = &mut self.decks[d];
        // Platte mit ihrem Label/Cover noch zeigen, während sie hinausgleitet
        let (title, artist) = v.meta.as_ref().map_or_else(Default::default, |m| (m.title.clone(), m.artist.clone()));
        v.ejected = Some((title, artist, v.cover.take(), now));
        v.path = None;
        v.meta = None;
        v.waveform = None;
        v.audio = None;
        v.bpm = None;
        v.cover = None;
        v.loaded_id = 0;
        v.spent = false;
        v.pending = None;
        v.loop_beats = None;
        v.loop_range = None;
    }

    /// „Alles leeren“: laufende Player stoppen sofort, eine Überblendung bricht ab, beide Player und beide
    /// Playlists werden leer (für einen neuen Abend). Gefragt wird vorher in `clear_all_dialog`.
    pub(super) fn clear_all(&mut self, now: f64) {
        self.autofade.cancel();
        self.browser_action(BrowserAction::StopPreview, now);
        for d in 0..2 {
            if self.has_track(d) || self.decks[d].pending.is_some() {
                self.eject(d, now);
            }
            self.playlists[d].clear_keep(false);
        }
        self.notify(tr("Beide Playlists und Player sind leer."), now);
    }

    /// Rückfrage zu „Alles leeren“; spielt etwas, steht dabei, dass die Musik sofort stoppt.
    pub(super) fn clear_all_dialog(&mut self, ctx: &egui::Context, now: f64) {
        if !self.confirm_clear_all {
            return;
        }
        let playing: Vec<&str> = (0..2).filter(|&d| self.ctl.shared.decks[d].is_playing()).map(theme::deck_letter).collect();
        let mut close = false;
        let mut confirm = false;
        let modal = egui::Modal::new(egui::Id::new("alles_leeren"))
            .frame(egui::Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(tr("Alles leeren?")).font(crate::fonts::bold(17.0)));
                ui.add_space(6.0);
                ui.label(tr("Beide Playlists und beide Player werden geleert. Deine Musikdateien bleiben unberührt."));
                if !playing.is_empty() {
                    ui.add_space(4.0);
                    let who = playing.join(" + ");
                    ui.label(RichText::new(trf("Player {} spielt gerade, die Musik stoppt sofort.", &[&who])).color(theme::cue()));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let font = egui::FontId::proportional(13.0);
                    if crate::hw::button(ui, vec2(130.0, 28.0), tr("Alles leeren"), font.clone(), crate::hw::Look::Color(theme::cue()), true, true).clicked() {
                        confirm = true;
                    }
                    if crate::hw::button(ui, vec2(100.0, 28.0), tr("Abbrechen"), font, crate::hw::Look::Plain, false, true).clicked() {
                        close = true;
                    }
                });
            });
        if confirm {
            self.confirm_clear_all = false;
            self.clear_all(now);
        } else if close || modal.should_close() {
            self.confirm_clear_all = false;
        }
    }

    pub(super) fn deck_action(&mut self, d: usize, a: DeckAction, now: f64) {
        let st_pos = self.ctl.shared.decks[d].position_secs();
        let st_dur = self.ctl.shared.decks[d].duration_secs();
        match a {
            DeckAction::TogglePlay => {
                let starting = !self.ctl.shared.decks[d].is_playing();
                self.ctl.send(Command::TogglePlay { deck: d });
                if starting {
                    self.decks[d].spent = false;
                }
                if starting && self.autofade.fade().is_none() {
                    let (ga, gb) = self.crossfader_gains();
                    let gain = if d == 0 { ga } else { gb };
                    let other_idle = !self.ctl.shared.decks[1 - d].is_playing();
                    if self.autofade.enabled && other_idle && self.has_track(d) && self.crossfader != fw_engine::automix::side(d) {
                        // Der andere Player steht: Crossfader zu diesem hinüberblenden
                        self.autofade.glide_to(d, self.crossfader);
                    } else if gain > 0.5 {
                        self.autofade.active = d;
                    }
                }
            }
            DeckAction::Cue => self.ctl.send(Command::Cue { deck: d }),
            DeckAction::Stop => self.ctl.send(Command::Stop { deck: d }),
            DeckAction::Prev => match self.playlists[d].prev_index() {
                Some(i) if st_pos <= 3.0 && self.has_track(d) => {
                    self.load_index(d, i, false, true);
                }
                _ => self.ctl.send(Command::Seek { deck: d, secs: 0.0 }),
            },
            DeckAction::Next => {
                let wrap = self.decks[d].repeat == Repeat::List;
                match self.playlists[d].next_index(wrap) {
                    Some(i) => {
                        self.load_index(d, i, false, true);
                    }
                    None => self.notify(trf("Playlist {}: kein weiterer Titel", &[&theme::deck_letter(d)]), now),
                }
            }
            DeckAction::Eject => {
                if self.on_air(d) {
                    self.notify(trf("Player {} ist gerade zu hören. Erst stoppen oder wegblenden.", &[&theme::deck_letter(d)]), now);
                    return;
                }
                self.eject(d, now);
            }
            DeckAction::Reverse(on) => {
                self.decks[d].reverse = on;
                self.ctl.send(Command::SetReverse { deck: d, on });
            }
            DeckAction::CycleRepeat => {
                let r = self.decks[d].repeat.next();
                self.decks[d].repeat = r;
                self.ctl.send(Command::SetRepeatTrack { deck: d, on: r == Repeat::Track });
            }
            DeckAction::Pfl(on) => {
                self.decks[d].pfl = on;
                self.ctl.send(Command::SetPfl { deck: d, on });
                if on {
                    self.enable_headphones(now);
                }
            }
            DeckAction::Sync => {
                self.decks[d].sync = None;
                let o = 1 - d;
                match (self.decks[d].bpm, self.decks[o].bpm.filter(|_| self.has_track(o))) {
                    (Some(mine), Some(theirs)) => {
                        let target = theirs * (1.0 + self.decks[o].pitch_pct / 100.0);
                        let pct = (target / mine - 1.0) * 100.0;
                        if pct.abs() > PITCH_RANGE {
                            self.notify(trf("Zu weit auseinander ({} ↔ {} BPM), Pitch reicht nur ±{} %", &[&format!("{mine:.1}"), &format!("{target:.1}"), &format!("{PITCH_RANGE:.0}")]), now);
                        } else {
                            let pct = (pct * 10.0).round() / 10.0;
                            self.decks[d].pitch_pct = pct;
                            self.ctl.send(Command::SetPitch { deck: d, pitch: pct / 100.0 });
                        }
                    }
                    _ => self.notify(tr("Für Sync brauchen beide Player einen Titel mit BPM"), now),
                }
            }
            DeckAction::Loop(beats) => {
                let v = &mut self.decks[d];
                if v.loop_beats == Some(beats) {
                    v.loop_beats = None;
                    v.loop_range = None;
                } else if let Some(bpm) = v.bpm {
                    let beat = 60.0 / (bpm as f64);
                    let range = (st_pos, (st_pos + beats as f64 * beat).min(st_dur));
                    v.loop_beats = Some(beats);
                    v.loop_range = Some(range);
                }
                let range = v.loop_range;
                self.ctl.send(Command::SetLoop { deck: d, range });
            }
            DeckAction::Filter(value) => {
                self.filter[d] = value;
                self.ctl.send(Command::SetFilter { deck: d, value });
            }
            DeckAction::Pitch(p) => {
                self.decks[d].sync = None;
                self.decks[d].pitch_pct = p;
                self.ctl.send(Command::SetPitch { deck: d, pitch: p / 100.0 });
            }
            DeckAction::Seek(s) => self.ctl.send(Command::Seek { deck: d, secs: s }),
            DeckAction::ScratchBegin => {
                self.decks[d].scratching = true;
                self.decks[d].scratch_pos = st_pos;
                self.ctl.send(Command::ScratchBegin { deck: d });
            }
            DeckAction::ScratchBy(ds) => {
                if self.decks[d].scratching {
                    let p = (self.decks[d].scratch_pos + ds).clamp(0.0, st_dur);
                    self.decks[d].scratch_pos = p;
                    self.ctl.send(Command::ScratchTo { deck: d, secs: p });
                }
            }
            DeckAction::ScratchEnd => {
                self.decks[d].scratching = false;
                self.ctl.send(Command::ScratchEnd { deck: d });
            }
        }
    }

    pub(super) fn mixer_action(&mut self, a: MixerAction, now: f64) {
        match a {
            MixerAction::Volume(d, v) => {
                self.decks[d].volume = v;
                self.ctl.send(Command::SetChannelVolume { deck: d, volume: v });
            }
            MixerAction::HeadMode(m) => {
                if m == HeadMode::Outputs34 && self.ctl.shared.channels < 4 {
                    self.notify(trf("Das Ausgabegerät hat nur {} Kanäle", &[&self.ctl.shared.channels]), now);
                    return;
                }
                self.set_head_mode(m);
            }
            MixerAction::HeadMix(v) => {
                self.head_mix = v;
                self.ctl.send(Command::SetHeadMix(v));
            }
            MixerAction::HeadVolume(v) => {
                self.head_volume = v;
                self.ctl.send(Command::SetHeadVolume(v));
            }
            MixerAction::MasterVolume(v) => {
                self.master_volume = v;
                self.ctl.send(Command::SetMasterVolume(v));
            }
            MixerAction::Eq { deck, band, db } => self.eq_db[deck][band] = db,
            MixerAction::EqKill { deck, band, on } => self.eq_kill[deck][band] = on,
            MixerAction::Record => self.toggle_recording(now),
            MixerAction::Mic => self.toggle_mic(now),
            MixerAction::Fx { slot, to_master } => self.trigger_fx(slot, to_master, now),
            MixerAction::FxVolume(v) => {
                self.fx_volume = v;
                self.ctl.send(Command::SetFxVolume(v));
            }
        }
    }

    /// Effekt-Pad auslösen: über Master, oder zum Vorhören nur auf dem Kopfhörer.
    pub(super) fn trigger_fx(&mut self, slot: usize, to_master: bool, now: f64) {
        if !to_master {
            self.enable_headphones(now);
        }
        self.ctl.send(Command::TriggerFx { slot, to_master });
    }

    /// Geladene Effekt-Klänge an die Engine geben.
    pub(super) fn poll_fx(&mut self, now: f64) {
        for (slot, sound, err) in self.fx.poll() {
            self.ctl.send(Command::LoadFx { slot, sound });
            if err.is_some() {
                self.notify(trf("Effekt {}: Datei nicht lesbar, es klingt der mitgelieferte Effekt", &[&(slot + 1)]), now);
            }
        }
    }

    /// Tasten 1 bis 8 lösen die Effekt-Pads aus (nicht beim Tippen, nicht mit ⌘/Strg/⌥, ohne Tastenwiederholung).
    pub(super) fn fx_keys(&mut self, ctx: &egui::Context, now: f64) {
        for slot in pad_keys(ctx) {
            self.trigger_fx(slot, true, now);
        }
    }

    pub(super) fn fade_action(&mut self, a: FadeAction, now: f64) {
        match a {
            FadeAction::Auto(on) => self.autofade.enabled = on,
            FadeAction::FadeSecs(s) => {
                self.autofade.fade_secs = s;
                self.ctl.send(Command::SetRepeatFade(s));
            }
            FadeAction::Curve(c) => {
                self.curve = c;
                self.ctl.send(Command::SetCurve(c));
            }
            FadeAction::Crossfader(x) => {
                self.autofade.cancel();
                self.set_crossfader(x);
                self.autofade.follow_crossfader(x);
            }
            FadeAction::FadeNow => self.start_fade(true, now),
            FadeAction::Motor(m) => {
                self.motor = m;
                self.ctl.send(Command::SetMotor(m));
            }
            FadeAction::BassSwap(on) => self.bass_swap = on,
            FadeAction::AutoSync(on) => {
                self.auto_sync = on;
                for v in &mut self.decks {
                    v.sync = None;
                }
            }
        }
    }

    pub(super) fn playlist_action(&mut self, d: usize, a: PlaylistAction, now: f64) {
        match a {
            PlaylistAction::Load(i) => {
                if self.ctl.shared.decks[d].is_playing() {
                    self.notify(trf("Titel nicht geladen: Player {} spielt gerade. Erst stoppen.", &[&theme::deck_letter(d)]), now);
                } else {
                    self.load_index(d, i, false, false);
                }
            }
            PlaylistAction::LoadBehindPlayed(i) => {
                if self.ctl.shared.decks[d].is_playing() {
                    self.notify(trf("Titel nicht geladen: Player {} spielt gerade. Erst stoppen.", &[&theme::deck_letter(d)]), now);
                } else {
                    let i = self.playlists[d].move_behind_played(i);
                    self.load_index(d, i, false, false);
                }
            }
            PlaylistAction::Remove(i) => {
                self.playlists[d].remove(i);
            }
            PlaylistAction::RemoveMany(indices) => {
                let indices = self.without_loaded(d, indices, now);
                self.playlists[d].remove_many(&indices);
            }
            PlaylistAction::MoveManyToOther(indices) => {
                self.playlist_action(1 - d, PlaylistAction::Drop { item: DragItem::Rows { list: d, indices }, before: usize::MAX }, now)
            }
            PlaylistAction::Harmonic => self.harmonic_start(now),
            PlaylistAction::SortUndo => {
                if let Some((before, _, _)) = self.sort_undo.take() {
                    self.playlists = before;
                    self.notify(tr("Reihenfolge wiederhergestellt"), now);
                }
            }
            PlaylistAction::Shuffle => {
                let before = self.playlists.clone();
                self.playlists[d].shuffle_upcoming(shuffle_seed());
                self.sort_undo = Some((before, crate::session::fingerprint(&self.playlists), now));
            }
            PlaylistAction::ClearAll => self.confirm_clear_all = true,
            PlaylistAction::Check => self.open_check(d),
            PlaylistAction::RemoveDuplicates => {
                let pl = &self.playlists[d];
                // Der geladene Titel gilt als erstes Exemplar und bleibt, sonst das früheste in der Liste
                let mut order: Vec<usize> = pl.current.filter(|&c| c < pl.entries.len()).into_iter().collect();
                order.extend((0..pl.entries.len()).filter(|&i| Some(i) != pl.current));
                let flags = crate::browser::duplicate_flags(order.iter().map(|&i| pl.entries[i].as_path()), |p| self.metas.get(p));
                let gone: Vec<usize> = order.iter().zip(flags).filter(|(_, dup)| *dup).map(|(&i, _)| i).collect();
                let n = gone.len();
                self.playlists[d].remove_many(&gone);
                let msg = if n == 0 { tr("Keine Dubletten gefunden").to_owned() } else { trf("{} Dubletten entfernt", &[&n]) };
                self.notify(msg, now);
            }
            PlaylistAction::Clear => {
                let keep = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
                self.playlists[d].clear_keep(keep);
            }
            PlaylistAction::NeedMeta(p) => self.request_meta(p),
            PlaylistAction::ShowMeta(p) => self.request_meta_now(p),
            PlaylistAction::NeedSketch(p) => {
                if self.sketch.requested.insert(p.clone()) {
                    self.sketch.batch[d].push(p);
                }
            }
            PlaylistAction::FadeWerk(on) => self.fadewerk = on,
            PlaylistAction::SetFade(i, fade) => {
                let start = |f: crate::fadewerk::TrackFade| f.fade_in.map_or(0.0, |r| r.start as f64);
                let old = start(self.playlists[d].fade(i));
                self.playlists[d].set_fade(i, fade);
                // Liegt der Titel schon gestoppt im Player, rückt er an den neuen Anfang (auch als Cue-Punkt)
                let st = &self.ctl.shared.decks[d];
                if self.playlists[d].current == Some(i) && self.has_track(d) && !st.is_playing() && start(fade) != old {
                    self.ctl.send(Command::Seek { deck: d, secs: start(fade) });
                    self.ctl.send(Command::Cue { deck: d });
                }
            }
            PlaylistAction::FadePreview(i, kind) => self.start_fade_preview(d, i, kind, now),
            PlaylistAction::ResetAllFades => {
                self.playlists.iter_mut().for_each(Playlist::clear_fades);
                self.notify(tr("Alle Fade-Bereiche entfernt"), now);
            }
            PlaylistAction::Preview(p) => self.browser_action(BrowserAction::Preview(p), now),
            PlaylistAction::StopPreview => self.browser_action(BrowserAction::StopPreview, now),
            PlaylistAction::PlayNext(i) => {
                let pl = &mut self.playlists[d];
                let before = pl.current.map_or(0, |c| c + 1);
                pl.move_entry(i, before);
            }
            PlaylistAction::EditTags(p) => self.browser_action(BrowserAction::EditTags(p), now),
            // Wie Ziehen ans Ende der anderen Playlist
            PlaylistAction::MoveToOther(i) => self.playlist_action(1 - d, PlaylistAction::Drop { item: DragItem::Row { list: d, index: i }, before: usize::MAX }, now),
            PlaylistAction::Analyze(p) => self.browser_action(BrowserAction::Analyze { paths: vec![p], force: true }, now),
            PlaylistAction::Drop { item, before } => match item {
                DragItem::Files(paths) => {
                    let at = before.min(self.playlists[d].entries.len());
                    self.playlists[d].insert(at, paths);
                    self.fill_if_empty(d, at);
                }
                DragItem::Row { list, index } if list == d => self.playlists[d].move_entry(index, before),
                DragItem::Row { list, index } => {
                    if self.playlists[list].current == Some(index) && self.has_track(list) {
                        self.notify(trf("Der Titel ist in Player {} geladen und bleibt in dessen Playlist", &[&theme::deck_letter(list)]), now);
                        return;
                    }
                    if let Some(item) = self.playlists[list].remove_with_fade(index) {
                        let at = before.min(self.playlists[d].entries.len());
                        self.playlists[d].insert_with_fades(at, [item]);
                        self.fill_if_empty(d, at);
                    }
                }
                DragItem::Rows { list, indices } if list == d => self.playlists[d].move_entries(&indices, before),
                DragItem::Rows { list, indices } => {
                    let indices = self.without_loaded(list, indices, now);
                    let items = self.playlists[list].remove_many(&indices);
                    if !items.is_empty() {
                        let at = before.min(self.playlists[d].entries.len());
                        self.playlists[d].insert_with_fades(at, items);
                        self.fill_if_empty(d, at);
                    }
                }
            },
        }
    }

    /// Mehrere Einträge der Playlist `d` ohne den Titel, der im Player liegt: der bleibt in seiner Playlist.
    pub(super) fn without_loaded(&mut self, d: usize, mut indices: Vec<usize>, now: f64) -> Vec<usize> {
        if let Some(c) = self.playlists[d].current.filter(|_| self.has_track(d))
            && indices.contains(&c)
        {
            indices.retain(|&i| i != c);
            self.notify(trf("Der Titel ist in Player {} geladen und bleibt in dessen Playlist", &[&theme::deck_letter(d)]), now);
        }
        indices
    }

    /// Titel per Drag & Drop auf einen Player gezogen. Aus der eigenen Playlist wird direkt geladen;
    /// sonst kommt der Titel hinter den aktuellen in die Playlist dieses Players, damit Vor/Zurück
    /// und Auto-Fade danach stimmig weiterlaufen.
    pub(super) fn drop_on_deck(&mut self, d: usize, item: DragItem, now: f64) {
        if self.ctl.shared.decks[d].is_playing() {
            self.notify(trf("Player {} spielt gerade. Erst stoppen.", &[&theme::deck_letter(d)]), now);
            return;
        }
        // Mehrere Zeilen: der Player nimmt die erste
        let item = match item {
            DragItem::Rows { list, indices } => match indices.first() {
                Some(&index) => DragItem::Row { list, index },
                None => return,
            },
            item => item,
        };
        let paths = match item {
            DragItem::Row { list, index } if list == d => {
                self.load_index(d, index, false, false);
                return;
            }
            DragItem::Row { list, index } => match self.playlists[list].entries.get(index) {
                // Kopie samt Fade-Bereichen hinter den aktuellen Titel
                Some(p) => {
                    let item = (p.clone(), self.playlists[list].fade(index));
                    let at = self.drop_insert_point(d);
                    self.playlists[d].insert_with_fades(at, [item]);
                    self.load_index(d, at, false, false);
                    return;
                }
                None => return,
            },
            DragItem::Files(paths) => paths,
            DragItem::Rows { .. } => return,
        };
        if paths.is_empty() {
            return;
        }
        let at = self.drop_insert_point(d);
        self.playlists[d].insert(at, paths);
        self.load_index(d, at, false, false);
    }

    /// Wohin ein auf den Player gezogener Titel in dessen Playlist kommt: hinter den geladenen Titel. War der
    /// noch gar nicht zu hören (steht an seinem Anfang bzw. am Beginn seines Einblendbereichs), davor, damit
    /// er der nächste bleibt und nicht als gespielt übersprungen wird.
    pub(super) fn drop_insert_point(&self, d: usize) -> usize {
        let pl = &self.playlists[d];
        let Some(c) = pl.current.filter(|&c| c < pl.entries.len()) else { return pl.entries.len() };
        let start = pl.fade(c).fade_in.map_or(0.0, |r| r.start as f64);
        let untouched = self.has_track(d) && !self.decks[d].spent && self.ctl.shared.decks[d].position_secs() <= start + 1.0;
        if untouched { c } else { c + 1 }
    }

    pub(super) fn browser_action(&mut self, a: BrowserAction, now: f64) {
        match a {
            BrowserAction::Add { list, paths } => {
                let n = paths.len();
                let first = self.playlists[list].entries.len();
                self.playlists[list].push(paths);
                self.fill_if_empty(list, first);
                self.notify(trf("{} Titel in Playlist {}", &[&n, &theme::deck_letter(list)]), now);
            }
            BrowserAction::AddShuffled { list, mut paths } => {
                shuffle(&mut paths);
                self.browser_action(BrowserAction::Add { list, paths }, now);
            }
            BrowserAction::AddNext { list, paths } => {
                let n = paths.len();
                let pl = &mut self.playlists[list];
                let at = pl.current.map_or(0, |c| c + 1).min(pl.entries.len());
                pl.insert(at, paths);
                self.fill_if_empty(list, at);
                self.notify(trf("{} Titel als Nächstes in Playlist {}", &[&n, &theme::deck_letter(list)]), now);
            }
            BrowserAction::Distribute(paths) => {
                // Mit dem Player beginnen, der als Nächstes einblendet: Spielt A, ist B dran
                let st = &self.ctl.shared.decks;
                let start = usize::from(st[0].is_playing() && !st[1].is_playing());
                let n = paths.len();
                let mut split: [Vec<PathBuf>; 2] = Default::default();
                for (k, p) in paths.into_iter().enumerate() {
                    split[(start + k) % 2].push(p);
                }
                for (list, paths) in split.into_iter().enumerate() {
                    let first = self.playlists[list].entries.len();
                    self.playlists[list].push(paths);
                    self.fill_if_empty(list, first);
                }
                self.notify(trf("{} Titel auf Playlist A und B verteilt", &[&n]), now);
            }
            BrowserAction::LoadDeck { deck, path } => self.drop_on_deck(deck, DragItem::Files(vec![path]), now),
            BrowserAction::Reveal(path) => {
                if let Err(e) = reveal(&path) {
                    log::warn!("Anzeigen im Dateimanager: {e}");
                    self.notify(trf("Lässt sich nicht anzeigen: {}", &[&e]), now);
                }
            }
            BrowserAction::StopScan => self.lib.cancel_scan(),
            BrowserAction::FolderRecursion(list) => {
                self.settings.folder_recursion = list;
                self.save_settings();
            }
            BrowserAction::EditTags(path) => self.tag_editor = Some(TagEditor::open(path)),
            BrowserAction::OnlineTags(paths) => {
                match &mut self.tag_batch {
                    None => self.tag_batch = Some(crate::tag_batch::TagBatch::new(paths)),
                    // Läuft schon (evtl. im Hintergrund): erst den laufenden Durchlauf zeigen
                    Some(b) => b.bring_back(),
                }
            }
            BrowserAction::SearchMemory(remember, hide_duplicates, whole_words, search) => {
                self.settings.remember_search = remember;
                self.settings.hide_duplicates = hide_duplicates;
                self.settings.search_whole_words = whole_words;
                self.settings.advanced_search = search;
                self.save_settings();
            }
            BrowserAction::TrackColumns(visible, widths) => {
                self.settings.track_columns = visible;
                self.settings.column_widths = widths;
                self.save_settings();
            }
            BrowserAction::TrackSort(sort) => {
                self.settings.track_sort = sort;
                self.save_settings();
            }
            BrowserAction::TreeWidth(w) => {
                self.settings.tree_width = w;
                self.save_settings();
            }
            BrowserAction::TreeHeight(h) => {
                self.settings.tree_height = h;
                self.save_settings();
            }
            BrowserAction::FolderColumns(on) => {
                self.settings.folder_columns = on;
                self.save_settings();
            }
            BrowserAction::ColumnsShare(share) => {
                self.settings.columns_share = share;
                self.save_settings();
            }
            BrowserAction::Preview(path) => {
                let token = self.token();
                self.preview = Some(PreviewView { path: path.clone(), token, ended_seen: 0, clip: None });
                self.lib.load(PREVIEW, token, path, false, fw_library::Known::default());
                self.enable_headphones(now);
            }
            BrowserAction::StopPreview => {
                self.ctl.send(Command::Eject { deck: PREVIEW });
                self.preview = None;
            }
            BrowserAction::PreviewSeek(s) => self.ctl.send(Command::Seek { deck: PREVIEW, secs: s }),
            BrowserAction::NeedMeta(p) => self.request_meta(p),
            BrowserAction::ShowMeta(p) => self.request_meta_now(p),
            BrowserAction::Analyze { paths, force } => {
                for p in paths {
                    // Vielleicht nur umgezogen (Ordner umbenannt): dann sind BPM und Tonart schon bekannt
                    if !force {
                        self.resolve_bpm(&p, fw_library::file_stamp(&p));
                    }
                    // Ohne `force` nur Titel, denen BPM oder Tonart fehlt
                    if (force || self.needs_analysis(&p)) && self.bpm_pending.insert(p.clone()) {
                        self.bpm_total += 1;
                        self.lib.analyze(p, force);
                    }
                }
            }
            BrowserAction::ShowTagBatch => {
                if let Some(b) = &mut self.tag_batch {
                    b.bring_back();
                }
            }
            BrowserAction::StopTagBatch => {
                if let Some(b) = &self.tag_batch {
                    b.stop();
                }
            }
            BrowserAction::StopBpm => {
                self.lib.cancel_analysis();
                self.bpm_pending.clear();
                self.bpm_total = 0;
            }
            BrowserAction::AddRoot => {
                if let Some(dir) = rfd::FileDialog::new().set_title(tr("Musikordner hinzufügen")).pick_folder()
                    && self.browser.add_root(dir.clone())
                    // Eine Netzwerkfreigabe, die beim Start noch nicht eingehängt war, fehlte im Baum, steht aber schon hier
                    && !self.settings.folders.contains(&dir)
                {
                    self.settings.folders.push(dir);
                    self.save_settings();
                }
            }
            BrowserAction::RemoveRoot(dir) => {
                self.browser.remove_root(&dir);
                self.settings.folders.retain(|p| p != &dir);
                self.save_settings();
            }
            BrowserAction::HistoryRename { date, name } => self.history.rename(&date, &name),
            BrowserAction::HistoryRestore(date) => self.restore_history(&date, now),
            BrowserAction::HistoryRemove { date, slots } => {
                self.history.remove_entries(&date, &slots);
                self.notify(trf("{} Einträge aus der Historie entfernt", &[&slots.len()]), now);
            }
            BrowserAction::HistoryDelete(date) => {
                if let Some(day) = self.history.day(&date) {
                    self.history_dialog = Some(HistoryDialog::Delete { date, label: day.label(), count: day.entries.len() });
                }
            }
            BrowserAction::HistoryExport { date, format } => self.export_history(&date, format, now),
            BrowserAction::HistoryImport => {
                if self.import_rx.is_none()
                    && let Some(file) = rfd::FileDialog::new()
                        .set_title(tr("Historie importieren"))
                        .add_filter(tr("Historie oder Playlist"), &["json", "m3u", "m3u8"])
                        .pick_file()
                {
                    let ctx = self.repaint_ctx.clone();
                    self.import_rx = Some(crate::history::import_in_background(file, self.music_roots(), move || ctx.request_repaint()));
                }
            }
            BrowserAction::OpenSettings => self.settings_ui.toggle(),
            BrowserAction::ToggleSide => {
                self.settings.browser_side = !self.settings.browser_side;
                self.save_due = Some(now + 1.0);
            }
            BrowserAction::TreeLeft(on) => {
                self.settings.browser_window_tree_left = on;
                self.save_due = Some(now + 1.0);
            }
            BrowserAction::Detach(on) => {
                self.settings.browser_detached = on;
                self.browser_window_init = None;
                self.browser_window_lost = 0;
                self.cross = Default::default();
                self.save_due = Some(now + 1.0);
            }
        }
    }
}
