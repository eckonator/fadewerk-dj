// SPDX-License-Identifier: GPL-3.0-or-later
//! Historie: gespielte Titel aufzeichnen, wiederherstellen, exportieren und importieren.

use super::*;

impl FadeWerkDjApp {
    /// Historie: Titel eintragen, sobald sie lange genug über Master zu hören waren.
    pub(super) fn record_plays(&mut self) {
        let dt = self.history_clock.elapsed().as_secs_f64();
        self.history_clock = std::time::Instant::now();
        for d in 0..2 {
            let st = &self.ctl.shared.decks[d];
            // Zu Ende gelaufen: spielt der Titel noch einmal, zählt er erneut
            let ended = st.ended_count();
            if ended != self.history_ended[d] {
                self.history_ended[d] = ended;
                self.play_trackers[d].restart();
            }
            let id = if self.settings.record_history { self.decks[d].loaded_id } else { 0 };
            let audible = self.on_air(d) && st.is_playing() && self.master_volume > 0.02;
            let pos = st.position_secs();
            let Some(at) = self.play_trackers[d].update(id, audible, pos, dt, crate::history::now_local) else { continue };
            let v = &self.decks[d];
            let Some(path) = v.path.clone() else { continue };
            let meta = v.meta.clone().unwrap_or_else(|| TrackMeta::from_filename(&path));
            let entry = Entry {
                size: std::fs::metadata(&path).ok().map(|m| m.len()),
                path,
                title: meta.title,
                artist: meta.artist,
                album: meta.album,
                duration_secs: meta.duration_secs,
                played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                deck: Some(d as u8),
            };
            log::info!("Historie: {} – {}", entry.artist, entry.title);
            self.history.record(entry);
        }
    }

    /// Entwicklerschalter `FW_HISTORY`: zwei Beispieltage aus den Demo-Titeln, der neuere ist geöffnet.
    pub(super) fn demo_history(&mut self, tracks: &[PathBuf]) {
        let today = crate::history::history_date(crate::history::now_local());
        // Ein Titel, dessen Datei fehlt (wird ausgegraut gezeigt)
        // Namen passend zur Sprache, damit englische Screenshots nichts Deutsches zeigen
        let en = i18n::lang() == i18n::Lang::En;
        let (artist, title, name) = if en { ("Unknown", "Lost Track", "Grandma's birthday") } else { ("Unbekannt", "Verschollen", "Omas Geburtstag") };
        let gone = PathBuf::from(format!("/gibt/es/nicht/{artist} - {title}.mp3"));
        let at = today.at(19, 56, 0, 0);
        self.history.record(Entry { path: gone, title: title.into(), artist: artist.into(), played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()), deck: Some(0), ..Default::default() });
        for (k, p) in tracks.iter().enumerate() {
            let meta = TrackMeta::from_filename(p);
            let day = if k < 3 { today.yesterday().unwrap_or(today) } else { today };
            let at = day.at(20, 0, 0, 0) + jiff::Span::new().minutes(4 * k as i64);
            self.history.record(Entry {
                path: p.clone(),
                title: meta.title,
                artist: meta.artist,
                duration_secs: 200.0,
                played_at: Some(at.strftime("%Y-%m-%dT%H:%M:%S").to_string()),
                deck: Some((k % 2) as u8),
                ..Default::default()
            });
        }
        self.history.rename(&today.to_string(), name);
        self.browser.open_history_day(today.to_string());
    }

    /// Füllt die Playlists wieder so, wie an diesem Tag gespielt wurde: Titel von Player A nach A, von B
    /// nach B, in der Reihenfolge von damals. Titel ohne Player-Angabe (importierte Playlists) werden
    /// abwechselnd verteilt, fehlende Dateien übersprungen. Ein laufender Titel bleibt, der Rest folgt dahinter.
    pub(super) fn restore_history(&mut self, date: &str, now: f64) {
        let Some(day) = self.history.day(date) else { return };
        let mut lists: [Vec<PathBuf>; 2] = Default::default();
        let mut next = 0;
        for e in day.entries.iter().filter(|e| e.path.is_file()) {
            let d = e.deck.map(usize::from).filter(|d| *d < 2).unwrap_or_else(|| {
                next += 1;
                (next - 1) % 2
            });
            lists[d].push(e.path.clone());
        }
        let counts = [lists[0].len(), lists[1].len()];
        if counts == [0, 0] {
            self.notify(tr("Keine der Dateien dieses Tages ist zu finden"), now);
            return;
        }
        for (d, paths) in lists.into_iter().enumerate() {
            let playing = self.has_track(d) && self.ctl.shared.decks[d].is_playing();
            self.playlists[d].clear_keep(playing);
            let first = self.playlists[d].entries.len();
            let any = !paths.is_empty();
            self.playlists[d].push(paths);
            // Steht der Player, gleich den ersten Titel bereitlegen
            if !playing && any {
                self.load_index(d, first, false, false);
            }
        }
        self.notify(trf("Wiederhergestellt: {} Titel in Playlist A, {} in B", &[&counts[0], &counts[1]]), now);
    }

    /// Speichert einen Tag in einem der Exportformate (Speicherort wählt der Nutzer).
    pub(super) fn export_history(&mut self, date: &str, format: ExportFormat, now: f64) {
        let Some(day) = self.history.day(date) else { return };
        let (filter, text) = match format {
            ExportFormat::M3u => (tr("Playlist"), crate::history::to_m3u(day)),
            ExportFormat::Csv => {
                let sep = if i18n::lang() == i18n::Lang::De { ';' } else { ',' };
                let headers = [tr("Nr."), tr("Uhrzeit"), tr("Interpret"), tr("Titel"), tr("Album"), tr("Länge"), tr("Player"), tr("Datei")];
                (tr("Tabelle"), crate::history::to_csv(day, sep, headers))
            }
            ExportFormat::Json => (tr("FadeWerk-DJ-Historie"), crate::history::to_json(day)),
        };
        let ext = format.extension();
        let Some(file) = rfd::FileDialog::new()
            .set_title(tr("Historie exportieren"))
            .set_file_name(format!("{}.{ext}", day.file_stem()))
            .add_filter(filter, &[ext])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&file, text) {
            Ok(()) => self.notify(trf("Exportiert: {}", &[&file.file_name().unwrap_or_default().to_string_lossy()]), now),
            Err(e) => self.notify(trf("Export fehlgeschlagen: {}", &[&e]), now),
        }
    }

    /// Ordner, in denen beim Import fehlende Titel gesucht werden: eigene Ordner und der Musikordner.
    pub(super) fn music_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.settings.folders.clone();
        if let Some(m) = dirs::audio_dir()
            && !roots.iter().any(|r| m.starts_with(r))
        {
            roots.push(m);
        }
        roots
    }

    pub(super) fn poll_import(&mut self, now: f64) {
        let Some(rx) = &self.import_rx else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.import_rx = None;
        match result {
            Ok((crate::history::Imported::Day(day), found)) => {
                let date = day.date.clone();
                let n = day.entries.len();
                let added = self.history.merge(day);
                self.browser.open_history_day(date.clone());
                let mut text = if added == n { trf("Historie {} importiert: {} Titel", &[&date, &n]) } else { trf("Historie {}: {} neue Titel", &[&date, &added]) };
                if found > 0 {
                    text.push_str(&trf(" ({} wiedergefunden)", &[&found]));
                }
                self.notify(text, now);
            }
            Ok((crate::history::Imported::List { name, entries }, found)) => {
                let date = crate::history::history_date(crate::history::now_local()).to_string();
                self.history_dialog = Some(HistoryDialog::Import { entries, found, date, name });
            }
            Err(e) => self.notify(trf("Import fehlgeschlagen: {}", &[&e]), now),
        }
    }
}
