// SPDX-License-Identifier: GPL-3.0-or-later
//! Antworten der Bibliothek, Laden von Titeln, BPM/Tonart, harmonisches Sortieren, Wellenform-Skizzen.

use super::*;

/// Höchstens so viele fehlende Titel überspringt `load_next` am Stück.
const MAX_SKIP_MISSING: usize = 50;

impl FadeWerkDjApp {
    /// Gemerkte BPM und Tonart für einen Titel prüfen: geänderte Datei → verwerfen, umgezogene Datei → übernehmen.
    pub(super) fn resolve_bpm(&mut self, path: &Path, stamp: Option<fw_library::Stamp>) {
        use crate::analysis_cache::Resolved;
        self.data_gen += 1;
        match self.caches.bpm.resolve_stamped(path, stamp) {
            Resolved::Value(b) => {
                self.bpms.insert(path.to_path_buf(), b.bpm);
            }
            Resolved::Stale => {
                self.bpms.remove(path);
            }
            Resolved::Unknown => {}
        }
        match self.caches.key.resolve_stamped(path, stamp) {
            Resolved::Value(k) => {
                self.keys.insert(path.to_path_buf(), k.camelot());
            }
            Resolved::Stale => {
                self.keys.remove(path);
            }
            Resolved::Unknown => {}
        }
    }

    /// Klingende Tonart eines Players und ihre Verschiebung in Halbtönen: ohne Keylock hebt bzw. senkt Pitch die
    /// Tonhöhe mit (ab gut ±3 % ein Halbton).
    pub(super) fn deck_key(&self, d: usize) -> (Option<Camelot>, i32) {
        let v = &self.decks[d];
        let Some(path) = v.path.as_deref().filter(|_| v.loaded_id != 0) else { return (None, 0) };
        let semis = (12.0 * (1.0 + v.pitch_pct / 100.0).max(0.01).log2()).round() as i32;
        (self.key_of(path).map(|k| k.shifted(semis)), semis)
    }

    /// Player, nach dessen klingender Tonart sich „passt“ richtet: der auf Sendung; sind beide zu hören,
    /// der, zu dem der Crossfader neigt.
    pub(super) fn key_reference(&self, deck_keys: &[(Option<Camelot>, i32); 2]) -> Option<(usize, Camelot)> {
        let d = match (self.on_air(0), self.on_air(1)) {
            (true, true) => usize::from(self.crossfader > 0.0),
            (true, false) => 0,
            (false, true) => 1,
            (false, false) => return None,
        };
        Some((d, deck_keys[d].0?))
    }

    /// „Harmonisch“: erst fehlende Tonarten (und BPM) der kommenden Titel ermitteln, dann sortieren.
    pub(super) fn harmonic_start(&mut self, now: f64) {
        let missing: Vec<PathBuf> =
            self.playlists.iter().flat_map(|pl| pl.upcoming_entries()).filter(|p| self.needs_analysis(p)).cloned().collect();
        if missing.is_empty() {
            self.harmonic_sort(now);
            return;
        }
        self.harmonic_pending = true;
        self.notify(trf("Ermittle {} fehlende Tonarten, danach wird sortiert …", &[&missing.len()]), now);
        self.browser_action(BrowserAction::Analyze { paths: missing, force: false }, now);
    }

    /// Ordnet die kommenden Titel beider Playlists nach `harmonic::plan`, ausgehend vom Titel auf Sendung.
    pub(super) fn harmonic_sort(&mut self, now: f64) {
        let deck_keys = [self.deck_key(0), self.deck_key(1)];
        let deck_bpm = |d: usize| self.decks[d].bpm.map(|b| b * (1.0 + self.decks[d].pitch_pct / 100.0));
        // Auf Sendung ist der Ausgangspunkt, als Nächstes spielt der andere Player; spielt nichts,
        // gilt der geladene Titel von A, sonst beginnt A
        let (start, first) = match self.key_reference(&deck_keys) {
            Some((d, key)) => (Some(crate::harmonic::Track { key: Some(key), bpm: deck_bpm(d) }), 1 - d),
            None if self.has_track(0) => (Some(crate::harmonic::Track { key: deck_keys[0].0, bpm: deck_bpm(0) }), 1),
            None => (None, 0),
        };
        let track = |p: &PathBuf| crate::harmonic::Track {
            key: self.key_of(p),
            bpm: self.bpms.get(p).copied().or_else(|| self.metas.get(p).and_then(|m| m.bpm)),
        };
        let lists: [Vec<crate::harmonic::Track>; 2] = std::array::from_fn(|d| self.playlists[d].upcoming_entries().iter().map(track).collect());
        let order = crate::harmonic::plan(start, first, [&lists[0], &lists[1]]);
        let before = self.playlists.clone();
        for (pl, o) in self.playlists.iter_mut().zip(&order) {
            pl.reorder_upcoming(o);
        }
        self.sort_undo = Some((before, crate::session::fingerprint(&self.playlists), now));
        let unknown = lists.iter().flatten().filter(|t| t.key.is_none()).count();
        if unknown > 0 {
            self.notify(trf("Harmonisch sortiert; {} Titel ohne Tonart stehen am Ende", &[&unknown]), now);
        } else {
            self.notify(tr("Kommende Titel harmonisch sortiert"), now);
        }
    }

    /// Zustand des Auswahlfelds „Sortieren“. „Rückgängig“ gilt eine Minute und nur, solange niemand die Playlists
    /// verändert hat (sonst würde es Änderungen danach verwerfen).
    pub(super) fn harmonic_state(&mut self, now: f64) -> playlist_ui::Harmonic {
        if self.harmonic_pending {
            return playlist_ui::Harmonic::Analyzing;
        }
        let valid = self.sort_undo.as_ref().is_some_and(|(_, fp, t)| now - t < SORT_UNDO_SECS && *fp == crate::session::fingerprint(&self.playlists));
        if valid {
            playlist_ui::Harmonic::Undo
        } else {
            self.sort_undo = None;
            playlist_ui::Harmonic::Ready
        }
    }

    /// Tonart eines Titels: aus dem Tag, sonst aus der Erkennung.
    pub(super) fn key_of(&self, path: &Path) -> Option<Camelot> {
        self.metas.get(path).and_then(|m| m.key).or_else(|| self.keys.get(path).copied().flatten())
    }

    /// Fehlt dem Titel noch BPM oder Tonart (dann lohnt „Analysieren“ ohne `force`)?
    pub(super) fn needs_analysis(&self, path: &Path) -> bool {
        let tag = self.metas.get(path);
        let bpm_known = self.bpms.contains_key(path) || tag.is_some_and(|m| m.bpm.is_some());
        let key_known = self.keys.contains_key(path) || tag.is_some_and(|m| m.key.is_some());
        !(bpm_known && key_known)
    }

    pub(super) fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    pub(super) fn request_meta(&mut self, path: PathBuf) {
        if self.metas.contains_key(&path) || !self.meta_requested.insert(path.clone()) {
            return;
        }
        // Gemerkte Tags gleich zeigen und im Hintergrund prüfen, ob die Datei sich geändert hat
        match self.caches.tag.get(&path) {
            Some((meta, stamp)) => {
                self.metas.insert(path.clone(), meta);
                self.data_gen += 1;
                self.lib.check_meta(path, stamp);
            }
            None => self.lib.request_meta(path),
        }
    }

    /// Tags einer gerade sichtbaren Zeile: vor allen anderen wartenden Anfragen lesen.
    pub(super) fn request_meta_now(&mut self, path: PathBuf) {
        if self.metas.contains_key(&path) || !self.meta_urgent.insert(path.clone()) {
            return;
        }
        if self.caches.tag.get(&path).is_some() {
            self.request_meta(path);
        } else {
            self.meta_requested.insert(path.clone());
            self.lib.request_meta_now(path);
        }
    }

    pub(super) fn load_index(&mut self, d: usize, i: usize, autoplay: bool, keep_playing: bool) -> bool {
        let Some(path) = self.playlists[d].entries.get(i).cloned() else { return false };
        if !self.screen_known {
            self.deferred_loads.push((d, i, autoplay, keep_playing));
            return true;
        }
        let token = self.token();
        self.decks[d].pending = Some(Pending { token, index: Some(i), autoplay, keep_playing, started: false });
        // Cover nur für sichtbare Plattenteller (nicht in der kompakten Ansicht, nicht bei ausgeblendeten)
        let cover = self.settings.platters && !self.compact();
        // Bekannte Tags mitgeben: ohne Cover muss der Tag-Block einer MP3 dann nicht übers Netz gelesen werden
        let known = fw_library::Known { meta: self.metas.get(&path).cloned(), bpm: self.bpms.get(&path).copied(), key: self.keys.get(&path).copied() };
        self.lib.load(d, token, path, cover, known);
        true
    }

    /// Stelle, an der Player `d` nach einem Absturz mit `path` weitermacht.
    fn resume_secs(&self, d: usize, path: &Path) -> Option<f64> {
        self.resume[d].as_ref().filter(|r| r.path == path).map(|r| r.secs)
    }

    pub(super) fn load_next(&mut self, d: usize, autoplay: bool, keep_playing: bool) -> bool {
        let wrap = self.decks[d].repeat == Repeat::List;
        let pl = &self.playlists[d];
        let Some(first) = pl.next_index(wrap) else { return false };
        // Fehlende Dateien überspringen (Laufwerk abgezogen, Datei verschoben); sie bleiben in der Liste
        let n = pl.entries.len();
        let mut i = first;
        let mut skipped = 0;
        while skipped < n.min(MAX_SKIP_MISSING) && !pl.entries[i].is_file() {
            skipped += 1;
            i += 1;
            if i >= n {
                if !wrap {
                    return false;
                }
                i = 0;
            }
        }
        if skipped == n.min(MAX_SKIP_MISSING) {
            i = first;
        } else if skipped > 0 {
            let now = self.last_logic;
            self.notify(trf("{} fehlende Titel übersprungen (Playlist prüfen im ⋯-Menü)", &[&skipped]), now);
        }
        self.load_index(d, i, autoplay, keep_playing)
    }

    /// Player `d` wartet noch auf einen Titel. Spielt schon dessen Anfang (Laden in Stücken), ist er bereit.
    pub(super) fn loading(&self, d: usize) -> bool {
        self.decks[d].pending.as_ref().is_some_and(|p| !p.started)
    }

    /// Lädt den ersten neuen Titel, wenn der Player leer ist.
    pub(super) fn fill_if_empty(&mut self, d: usize, index: usize) {
        if !self.has_track(d) && self.decks[d].pending.is_none() {
            self.load_index(d, index, false, false);
        }
    }

    pub(super) fn poll_library(&mut self, ctx: &egui::Context, now: f64) {
        let responses: Vec<Response> = self.lib.poll().collect();
        if !responses.is_empty() {
            self.data_gen += 1;
        }
        for resp in responses {
            match resp {
                Response::Meta { path, meta, stamp } => {
                    // Geladene Titel zeigen geänderte Tags sofort
                    for v in self.decks.iter_mut().filter(|v| v.path.as_ref() == Some(&path)) {
                        v.meta = Some(meta.clone());
                    }
                    if let Some(stamp) = stamp
                        && self.persist
                    {
                        self.caches.tag.insert(&path, &meta, stamp);
                    }
                    self.resolve_bpm(&path, stamp);
                    self.metas.insert(path, meta);
                }
                Response::MetaValid { path, stamp } => self.resolve_bpm(&path, Some(stamp)),
                Response::Analysis { path, bpm, bpm_analyzed, key, key_analyzed, readable } => {
                    self.bpm_pending.remove(&path);
                    if self.bpm_pending.is_empty() {
                        self.bpm_total = 0;
                    }
                    if let Some(bpm) = bpm {
                        if bpm_analyzed {
                            self.caches.bpm.insert(&path, crate::analysis_cache::Bpm { bpm });
                        }
                        self.bpms.insert(path.clone(), bpm);
                    }
                    // Nachgereichte Analyse eines eben geladenen Titels: auch der Player bekommt das Tempo
                    if let Some(bpm) = bpm {
                        for v in self.decks.iter_mut().filter(|v| v.path.as_ref() == Some(&path) && v.bpm.is_none()) {
                            v.bpm = Some(bpm);
                        }
                    }
                    // Auch „unsicher“ merken, damit „Analysieren“ den Titel nicht jedes Mal wieder dekodiert
                    if readable && (key_analyzed || key.is_some()) {
                        if key_analyzed {
                            self.caches.key.insert(&path, crate::analysis_cache::Key::new(key));
                        }
                        self.keys.insert(path, key);
                    }
                }
                Response::Sketch { path, sketch } => {
                    self.sketch.map.insert(path, sketch);
                }
                Response::Scan { token, tracks, progress, done, limited } => {
                    self.browser.receive_scan(token, tracks, progress, done, limited);
                    // Langsame Ordner merken (und gemerkte auf den frischen Stand bringen)
                    if done
                        && let Some((_, roots, started)) = self.scan_job.take_if(|j| j.0 == token)
                    {
                        let slow = started.elapsed().as_secs_f64() >= crate::folder_cache::MIN_SCAN_SECS;
                        if self.persist && (slow || self.caches.folder.contains(&roots)) {
                            self.caches.folder.put(roots, self.browser.tracks());
                        }
                    }
                }
                Response::Partial { deck: d, token, path, meta, audio } => {
                    // Der Anfang des Titels: Player spielt schon, der vollständige folgt mit derselben Kennung
                    let Some(p) = self.decks.get(d).and_then(|v| v.pending.as_ref()).filter(|p| p.token == token) else { continue };
                    // FadeWerk: Startstelle im Einblendbereich; liegt sie nicht sicher im Anfang, auf den ganzen warten
                    let start = self.resume_secs(d, &path).unwrap_or_else(|| p.index.and_then(|i| self.playlists[d].fade(i).fade_in).map_or(0.0, |r| r.start as f64));
                    if start > audio.len_frames() as f64 / audio.sample_rate as f64 - 10.0 {
                        continue;
                    }
                    self.resume[d] = None;
                    let (autoplay, keep_playing, index) = (p.autoplay, p.keep_playing, p.index);
                    if let Some(p) = self.decks[d].pending.as_mut() {
                        p.started = true;
                    }
                    self.ctl.send(Command::Load { deck: d, id: token, track: Arc::clone(&audio), keep_playing, start_secs: start });
                    if autoplay {
                        self.ctl.send(Command::Play { deck: d });
                    }
                    let bpm = self.bpms.get(&path).copied().or(meta.bpm);
                    let v = &mut self.decks[d];
                    v.path = Some(path);
                    v.meta = Some(meta);
                    v.waveform = None;
                    v.bpm = bpm;
                    v.cover = None;
                    v.loaded_id = token;
                    v.spent = false;
                    v.audio = Some(audio);
                    v.loop_beats = None;
                    v.loop_range = None;
                    if let Some(i) = index {
                        self.playlists[d].current = Some(i);
                    }
                }
                Response::Loaded(l) => {
                    let l = *l;
                    self.metas.insert(l.path.clone(), l.meta.clone());
                    self.sketch.map.insert(l.path.clone(), Some(l.sketch.clone()));
                    if let Some(bpm) = l.bpm {
                        self.bpms.insert(l.path.clone(), bpm);
                        if l.bpm_analyzed {
                            self.caches.bpm.insert(&l.path, crate::analysis_cache::Bpm { bpm });
                        }
                    }
                    if l.key_analyzed {
                        self.caches.key.insert(&l.path, crate::analysis_cache::Key::new(l.key));
                    }
                    self.keys.insert(l.path.clone(), l.key);
                    self.unreadable.remove(&l.path);
                    if l.deck == PREVIEW {
                        if self.preview.as_ref().is_some_and(|p| p.token == l.token) {
                            let start = l.audio.duration_secs() * 0.3;
                            self.ctl.send(Command::Load { deck: PREVIEW, id: l.token, track: l.audio, keep_playing: false, start_secs: start });
                            self.ctl.send(Command::Play { deck: PREVIEW });
                        }
                        continue;
                    }
                    let d = l.deck;
                    let Some(p) = self.decks[d].pending.take_if(|p| p.token == l.token) else { continue };
                    let audio = l.audio.clone();
                    if p.started {
                        // Der Anfang spielt schon: nur den vollständigen Titel einsetzen, an derselben Stelle
                        self.ctl.send(Command::Complete { deck: d, id: l.token, track: l.audio });
                        let v = &mut self.decks[d];
                        v.meta = Some(l.meta);
                        v.waveform = Some(Arc::new(l.waveform));
                        v.bpm = l.bpm.or(v.bpm);
                        v.audio = Some(audio);
                        continue;
                    }
                    // FadeWerk: der Titel beginnt am Anfang seines Einblendbereichs
                    let fade_start = p.index.and_then(|i| self.playlists[d].fade(i).fade_in).map_or(0.0, |r| r.start as f64);
                    let start = self.resume_secs(d, &l.path).map_or(fade_start, |s| s.min((l.audio.duration_secs() - 1.0).max(0.0)));
                    self.resume[d] = None;
                    self.ctl.send(Command::Load { deck: d, id: l.token, track: l.audio, keep_playing: p.keep_playing, start_secs: start });
                    if p.autoplay {
                        self.ctl.send(Command::Play { deck: d });
                    }
                    let v = &mut self.decks[d];
                    v.path = Some(l.path);
                    v.meta = Some(l.meta);
                    v.waveform = Some(Arc::new(l.waveform));
                    v.bpm = l.bpm;
                    // Das Cover kommt danach (`Response::Cover`)
                    v.cover = None;
                    v.loaded_id = l.token;
                    v.spent = false;
                    v.audio = Some(audio);
                    v.loop_beats = None;
                    v.loop_range = None;
                    if let Some(i) = p.index {
                        self.playlists[d].current = Some(i);
                    }
                }
                Response::Cover { deck, token, cover } => {
                    // Nur, wenn der Titel noch geladen ist (inzwischen ausgeworfen oder ein anderer: verwerfen)
                    if deck < 2 && self.decks[deck].loaded_id == token {
                        self.decks[deck].cover = Some(cover_texture(ctx, deck, &cover));
                    }
                }
                Response::Failed { path, deck, error } => {
                    log::warn!("Laden fehlgeschlagen: {error}");
                    self.unreadable.insert(path.clone());
                    if let Some(d) = deck.filter(|d| *d < 2) {
                        self.decks[d].pending = None;
                    }
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.notify(trf("„{}“ lässt sich nicht abspielen", &[&name]), now);
                }
            }
        }
    }

    pub(super) fn handle_track_ends(&mut self) {
        // Rückwärts am Titelanfang: die Engine hat REV ausgeschaltet, die Taste geht mit aus
        for d in 0..2 {
            let c = self.ctl.shared.decks[d].reverse_ended_count();
            if c != self.decks[d].reverse_ended_seen {
                self.decks[d].reverse_ended_seen = c;
                self.decks[d].reverse = false;
            }
        }
        for d in 0..2 {
            let c = self.ctl.shared.decks[d].ended_count();
            if c != self.decks[d].ended_seen {
                self.decks[d].ended_seen = c;
                if self.autofade.fade().is_some_and(|f| f.from == d || f.to == d) {
                    continue;
                }
                // Ohne Auto-Fade oder wenn der andere Player nichts mehr hat, geht es in dieser Playlist weiter
                let o = 1 - d;
                let other_done = (!self.has_track(o) || self.decks[o].spent) && self.decks[o].pending.is_none();
                let continuous = (!self.autofade.enabled || other_done) && self.autofade.active == d;
                self.load_next(d, continuous, false);
            }
        }
        if let Some(p) = &mut self.preview {
            let c = self.ctl.shared.decks[PREVIEW].ended_count();
            if p.ended_seen == 0 {
                p.ended_seen = c;
            } else if c != p.ended_seen {
                self.preview = None;
            }
        }
    }

    /// Schickt die Wellenform-Anfragen dieses Bildes abwechselnd aus A und B, je von oben nach unten.
    /// So bauen sich beide Playlists gleichzeitig von oben auf (der Thread rechnet nur eine Wellenform zur Zeit).
    pub(super) fn send_sketch_requests(&mut self) {
        let [a, b] = std::mem::take(&mut self.sketch.batch);
        if a.is_empty() && b.is_empty() {
            return;
        }
        let mut order = Vec::with_capacity(a.len() + b.len());
        let (mut a, mut b) = (a.into_iter(), b.into_iter());
        loop {
            match (a.next(), b.next()) {
                (None, None) => break,
                (x, y) => order.extend(x.into_iter().chain(y)),
            }
        }
        self.lib.request_sketches(order);
    }
}
