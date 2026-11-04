// SPDX-License-Identifier: GPL-3.0-or-later
//! Einstellungen übernehmen und speichern, Fenstergröße und Zoom, Update-Leiste, Zurücksetzen.

use super::*;

impl FadeWerkDjApp {
    pub(super) fn settings_action(&mut self, a: SettingsAction, ctx: &egui::Context, now: f64) {
        match a {
            SettingsAction::Device(id) => {
                if self.settings.output_device != id {
                    self.settings.output_device = id;
                    self.restart_audio(now, None);
                }
            }
            SettingsAction::HeadDevice(id) => {
                // Dasselbe Gerät erneut wählen schaltet es ein, wenn es nicht läuft
                if self.settings.head_device != id || self.head_audio.is_none() {
                    self.settings.head_device = id;
                    self.start_head_audio(now, true);
                }
            }
            SettingsAction::TestTone(to_master) => self.test_tone(to_master, now),
            SettingsAction::Buffer(b) => {
                if self.settings.buffer_frames != b {
                    self.settings.buffer_frames = b;
                    self.restart_audio(now, None);
                }
            }
            SettingsAction::Language(l) => {
                self.settings.language = l;
                i18n::set_lang(l);
            }
            SettingsAction::Skin(s) => {
                self.settings.skin = s;
                theme::set_skin(s);
                theme::apply(ctx);
            }
            SettingsAction::Brushed(on) => {
                self.settings.brushed_steel = on;
                theme::set_brushed(on);
                theme::apply(ctx);
            }
            SettingsAction::Lite(parts) => {
                self.settings.lite = parts;
                theme::set_lite(parts);
                theme::apply(ctx);
            }
            SettingsAction::Platters(on) => self.settings.platters = on,
            SettingsAction::Compact(c) => self.settings.compact = c,
            SettingsAction::Zoom(z) => self.set_zoom(ctx, z),
            // Wirkt erst beim nächsten Start
            SettingsAction::OpenGl(on) => self.settings.opengl = on,
            SettingsAction::RecordHistory(on) => self.settings.record_history = on,
            SettingsAction::KeepAwake(on) => self.settings.keep_awake = on,
            SettingsAction::StatusItem(i, on) => {
                let s = &mut self.settings;
                *[&mut s.status_clock, &mut s.status_battery, &mut s.status_cpu, &mut s.status_ram, &mut s.status_temp][i.min(4)] = on;
            }
            SettingsAction::MicDevice(id) => {
                self.settings.mic_device = id;
                // Läuft das Mikrofon, gleich auf das neue Gerät wechseln
                if self.mic.take().is_some() {
                    self.ctl.send(Command::SetMic(None));
                    self.start_mic(now);
                }
            }
            SettingsAction::MicGain(g) => {
                self.settings.mic_gain = g;
                self.push_mic_settings();
            }
            SettingsAction::Talkover(on) => {
                self.settings.talkover = on;
                self.push_mic_settings();
            }
            SettingsAction::TalkoverDb(db) => {
                self.settings.talkover_db = db;
                self.push_mic_settings();
            }
            SettingsAction::Limiter(on) => {
                self.settings.limiter = on;
                self.ctl.send(Command::SetLimiter(on));
            }
            SettingsAction::RecordFormat(f) => self.settings.record_format = f,
            SettingsAction::RecordPickDir => {
                let start = self.settings.record_dir.clone().unwrap_or_else(crate::recorder::default_dir);
                let mut dialog = rfd::FileDialog::new().set_title(tr("Ordner für Aufnahmen"));
                if start.is_dir() {
                    dialog = dialog.set_directory(&start);
                }
                if let Some(dir) = dialog.pick_folder() {
                    self.settings.record_dir = Some(dir);
                }
            }
            SettingsAction::RecordDirDefault => self.settings.record_dir = None,
            SettingsAction::RecordRevealDir => {
                let dir = self.settings.record_dir.clone().unwrap_or_else(crate::recorder::default_dir);
                if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| reveal(&dir)) {
                    self.notify(trf("Ordner lässt sich nicht öffnen: {}", &[&e]), now);
                }
            }
            SettingsAction::MuteOthers(on) => {
                self.settings.mute_others = on;
                if let Some(q) = &self.quiet {
                    q.set(on);
                }
            }
            SettingsAction::Exclusive(on) => {
                self.settings.exclusive_output = on;
                if self.quiet.is_some() {
                    // Belegen und Freigeben bringen bei manchen Geräten den laufenden Stream aus dem Tritt
                    // (danach starteten die Player nicht mehr): Ausgabe dabei neu öffnen
                    let done = if on { tr("Lautsprecher exklusiv belegt") } else { tr("Lautsprecher wieder für alle frei") };
                    self.restart_audio(now, Some(done.to_owned()));
                }
            }
            SettingsAction::FxName(slot, name) => {
                self.settings.fx_slot_mut(slot).name = name;
                self.save_due = Some(now + 1.0);
            }
            SettingsAction::FxPickFile(slot) => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title(trf("Effekt für Pad {}", &[&(slot + 1)]))
                    .add_filter(tr("Audiodateien"), &["mp3", "wav", "flac", "ogg", "m4a", "aac", "aif", "aiff"]);
                if let Some(dir) = self.settings.fx_slot(slot).file.as_ref().and_then(|f| f.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(file) = dialog.pick_file() {
                    self.settings.fx_slot_mut(slot).file = Some(file.clone());
                    self.fx.load(slot, Some(file));
                    self.save_settings();
                }
            }
            SettingsAction::FxReset(slot) => {
                *self.settings.fx_slot_mut(slot) = Default::default();
                self.fx.load(slot, None);
                self.save_settings();
            }
            SettingsAction::FxPreview(slot) => self.trigger_fx(slot, false, now),
            SettingsAction::MeasureCaches => {
                if self.caches.sizes_rx.is_none() {
                    self.caches.sizes_rx = Some(crate::caches::measure());
                }
            }
            SettingsAction::ClearCaches { bpm } => {
                self.caches.tag = Default::default();
                self.caches.folder = Default::default();
                if bpm {
                    self.caches.bpm = Default::default();
                    self.bpms.clear();
                    self.caches.key = Default::default();
                    self.keys.clear();
                    self.data_gen += 1;
                }
                if self.persist {
                    crate::caches::clear(bpm);
                }
                // Der Wellenform-Ordner wird noch im Hintergrund geleert: gleich als leer anzeigen
                let kept_bpm = if bpm { 0 } else { self.caches.sizes.map_or(0, |s| s.bpm) };
                self.caches.sizes = Some(crate::caches::CacheSizes { bpm: kept_bpm, ..Default::default() });
                self.caches.sizes_rx = None;
                self.notify(tr("Caches geleert."), now);
            }
            SettingsAction::ResetAll => self.reset_all(ctx, now),
            SettingsAction::FxResetAll => {
                self.settings.fx_slots.clear();
                for slot in 0..fw_engine::FX_SLOTS {
                    self.fx.load(slot, None);
                }
                self.save_settings();
            }
            SettingsAction::FxIcon(slot, key) => {
                self.settings.fx_slot_mut(slot).icon = Some(key);
                self.save_settings();
            }
            SettingsAction::OnlineLookup(on) => {
                self.settings.online_lookup = on;
                self.save_settings();
            }
            SettingsAction::UpdateCheck(on) => {
                self.settings.update_check = Some(on);
                self.update.ask = false;
            }
            SettingsAction::CheckUpdateNow => {
                if self.update.rx.is_none() && self.update.installing.is_none() {
                    self.update.rx = Some(crate::update::check_in_background(ctx.clone()));
                    self.update.manual = true;
                }
            }
            SettingsAction::RevealLog => {
                if let Some(path) = crate::diagnose::log_path()
                    && let Err(e) = reveal(&path)
                {
                    self.notify(trf("Logdatei nicht gefunden: {}", &[&e]), now);
                }
            }
        }
        self.save_settings();
    }

    /// Übernimmt geänderte Werte in die Einstellungen und speichert verzögert (Schieberegler ändern sich laufend).
    pub(super) fn track_settings(&mut self, now: f64) {
        let mut s = self.settings.clone();
        s.spin_up_secs = self.motor.spin_up_secs;
        s.spin_down_secs = self.motor.spin_down_secs;
        s.fade_secs = self.autofade.fade_secs;
        s.auto_fade = self.autofade.enabled;
        s.curve = match self.curve {
            Curve::Soft => 0,
            Curve::Linear => 1,
            Curve::Cut => 2,
            Curve::Custom(pts) => {
                s.curve_points = pts;
                3
            }
        };
        s.master_volume = self.master_volume;
        s.fx_volume = self.fx_volume;
        s.head_volume = self.head_volume;
        s.head_mix = self.head_mix;
        s.eq_db = self.eq_db;
        s.bass_swap = self.bass_swap;
        s.auto_sync = self.auto_sync;
        if s != self.settings {
            self.settings = s;
            self.save_due = Some(now + 1.0);
        }
        if self.save_due.is_some_and(|t| now >= t) {
            self.save_settings();
            self.save_due = None;
        }
    }

    /// Mindestfenster an den Bildschirm anpassen (z. B. Netbook mit 1024×600). Ist er kleiner als das
    /// Layout, wird der Zoom einmalig so gesetzt, dass alles hineinpasst; danach entscheidet der Nutzer.
    pub(super) fn fit_screen(&mut self, ctx: &egui::Context) {
        let zoom = ctx.zoom_factor();
        // Bildschirm in logischen Pixeln (ohne Zoom), abzüglich Fenster- und Taskleiste
        let Some(screen) = crate::devtools::fake_screen().or_else(|| ctx.input(|i| i.viewport().monitor_size).map(|m| m * zoom)) else {
            return;
        };
        if screen.x < 1.0 || screen.y <= SCREEN_RESERVE_H {
            return;
        }
        self.small_screen = screen.y < COMPACT_AUTO_H;
        if !self.screen_known {
            self.screen_known = true;
            for (d, i, autoplay, keep_playing) in std::mem::take(&mut self.deferred_loads) {
                self.load_index(d, i, autoplay, keep_playing);
            }
        }
        let screen = screen - egui::vec2(0.0, SCREEN_RESERVE_H);
        // Kleiner als das normale Layout (Netbook): einmal auf Vollbild hinweisen, das spart Fenster- und Taskleiste
        let small = screen.x < MIN_WINDOW.x || screen.y < MIN_WINDOW.y;
        if small && !self.settings.fullscreen_hinted && !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
            self.settings.fullscreen_hinted = true;
            let now = ctx.input(|i| i.time);
            let text = if cfg!(target_os = "macos") {
                tr("Kleiner Bildschirm: Vollbild (Ctrl+⌘+F) schafft mehr Platz. Die Ordner gleiten mit ⌘B von links herein.")
            } else {
                tr("Kleiner Bildschirm: Vollbild (F11) schafft mehr Platz. Die Ordner gleiten mit Strg+B von links herein.")
            };
            self.notice = Some((text.to_owned(), now + 12.0));
            self.save_due = Some(now + 1.0);
        }
        // Das kleinere Mindestfenster nur, wenn die kompakte Ansicht auch wirklich an ist
        let min_window = if self.compact() { MIN_WINDOW_COMPACT } else { MIN_WINDOW };
        if !self.settings.zoom_fitted && (screen.x < min_window.x || screen.y < min_window.y) {
            self.settings.zoom_fitted = true;
            let fit = (screen.x / min_window.x).min(screen.y / min_window.y);
            let fit = ((fit * 20.0).floor() / 20.0).max(ZOOM_MIN);
            if fit < zoom {
                log::info!("Kleiner Bildschirm ({:.0}×{:.0}): Zoom {:.0} %", screen.x, screen.y + SCREEN_RESERVE_H, fit * 100.0);
                self.set_zoom(ctx, fit);
                return;
            }
            self.save_due = Some(ctx.input(|i| i.time) + 1.0);
        }
        let min = min_window.min(screen) / zoom;
        if self.min_window != Some(min) {
            self.min_window = Some(min);
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
        }
    }

    /// Kompakte Ansicht: fest eingestellt oder (automatisch) auf kleinen Bildschirmen.
    pub(super) fn compact(&self) -> bool {
        self.settings.compact.unwrap_or(self.small_screen)
    }

    /// Zoom der ganzen Oberfläche setzen (begrenzt, auf 5 % gerundet) und merken.
    pub(super) fn set_zoom(&mut self, ctx: &egui::Context, z: f32) {
        let z = ((z * 20.0).round() / 20.0).clamp(ZOOM_MIN, ZOOM_MAX);
        ctx.set_zoom_factor(z);
        self.settings.ui_zoom = z;
        self.save_due = Some(ctx.input(|i| i.time) + 1.0);
    }

    /// Schwebende Leiste oben in der Mitte: einmalige Frage, ob nach Updates gesucht werden darf,
    /// bzw. Hinweis auf eine neue Version samt Update per Knopf. Schwebt über der Oberfläche, damit sich
    /// nichts verschiebt.
    pub(super) fn update_ui(&mut self, ctx: &egui::Context, now: f64) {
        use crate::self_update::Progress;
        if let Some(rx) = &self.update.rx
            && let Ok(result) = rx.try_recv()
        {
            self.update.rx = None;
            let manual = std::mem::take(&mut self.update.manual);
            match result {
                Ok(Some(v)) => {
                    self.update.new_version = Some(v);
                    self.update.progress = None;
                }
                Ok(None) if manual => self.notify(trf("FadeWerk DJ {} ist die neueste Version.", &[&env!("CARGO_PKG_VERSION")]), now),
                // Ohne curl scheitert jede Prüfung still: auch bei der automatischen darauf hinweisen
                Err(crate::update::CheckError::NoCurl) => {
                    let text = tr("Für die Update-Prüfung fehlt das Programm curl (unter Debian/Ubuntu: sudo apt install curl).");
                    self.notice = Some((text.to_owned(), now + 12.0));
                }
                // Falsche Uhr: Auch die automatische Prüfung meldet es, sonst bleibt unerklärlich, warum nie ein Update kommt
                Err(crate::update::CheckError::ClockWrong) => {
                    let date = jiff::Zoned::now().strftime("%d.%m.%Y").to_string();
                    let text = trf("Datum und Uhrzeit automatisch einstellen: Die Uhr steht auf dem {}, so scheitern Updates.", &[&date]);
                    self.notice = Some((text, now + 20.0));
                }
                Err(_) if manual => self.notify(tr("GitHub ist nicht erreichbar, bitte später noch einmal versuchen."), now),
                _ => {}
            }
        }
        if let Some(rx) = &self.update.installing {
            while let Ok(p) = rx.try_recv() {
                if p == Progress::Restart {
                    // Beim Schließen speichert `Drop` Einstellungen und Playlists; die neue Version startet danach
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                self.update.progress = Some(p);
            }
            if matches!(self.update.progress, Some(Progress::Restart | Progress::Failed(_))) {
                self.update.installing = None;
            }
        }
        if !self.update.ask && self.update.new_version.is_none() {
            return;
        }
        let font = crate::fonts::bold(13.0);
        let playing = self.ctl.shared.decks.iter().any(|s| s.is_playing());
        let mut answer = None;
        let mut close = false;
        let mut install = None;
        egui::Area::new(egui::Id::new("update")).anchor(Align2::CENTER_TOP, vec2(0.0, 12.0)).show(ctx, |ui| {
            Frame::new()
                .fill(theme::panel2())
                .stroke(Stroke::new(1.0, theme::line()))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if self.update.ask {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(tr("Beim Start nach neuen Versionen suchen?")).strong().color(theme::ink()));
                                ui.label(RichText::new(tr("Dabei fragt FadeWerk DJ nur bei GitHub nach der neuesten Version – ohne Daten über deine Musik.")).size(11.5).color(theme::dim()));
                            });
                            ui.add_space(10.0);
                            if crate::hw::button(ui, vec2(100.0, 28.0), tr("Ja, prüfen"), font.clone(), crate::hw::Look::Color(theme::play()), false, true).clicked() {
                                answer = Some(true);
                            }
                            if crate::hw::button(ui, vec2(70.0, 28.0), tr("Nein"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                                answer = Some(false);
                            }
                        } else if let Some(v) = &self.update.new_version {
                            match &self.update.progress {
                                Some(Progress::Downloading) => {
                                    ui.spinner();
                                    ui.label(RichText::new(trf("Lade FadeWerk DJ {} …", &[&v.version])).strong().color(theme::ink()));
                                }
                                Some(Progress::Installing | Progress::Restart) => {
                                    ui.spinner();
                                    ui.label(RichText::new(trf("Installiere FadeWerk DJ {} – die App startet gleich neu …", &[&v.version])).strong().color(theme::ink()));
                                }
                                progress => {
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(trf("FadeWerk DJ {} ist verfügbar (installiert: {}).", &[&v.version, &env!("CARGO_PKG_VERSION")])).strong().color(theme::ink()));
                                        if let Some(Progress::Failed(e)) = progress {
                                            ui.label(RichText::new(trf("Update fehlgeschlagen: {}", &[e])).size(11.5).color(theme::cue()));
                                        } else if v.asset.is_some() && playing {
                                            ui.label(RichText::new(tr("Die App startet dabei neu. Erst, wenn keine Musik mehr läuft.")).size(11.5).color(theme::dim()));
                                        }
                                    });
                                    ui.add_space(10.0);
                                    let failed = matches!(progress, Some(Progress::Failed(_)));
                                    if let Some(asset) = &v.asset
                                        && !failed
                                        && crate::hw::button(ui, vec2(170.0, 28.0), tr("Jetzt aktualisieren"), font.clone(), crate::hw::Look::Color(theme::play()), false, !playing).clicked()
                                    {
                                        install = Some(asset.clone());
                                    }
                                    let look = if v.asset.is_some() && !failed { crate::hw::Look::Plain } else { crate::hw::Look::Color(theme::play()) };
                                    if crate::hw::button(ui, vec2(190.0, 28.0), tr("Download-Seite öffnen"), font.clone(), look, false, true).clicked() {
                                        ctx.open_url(egui::OpenUrl::new_tab(&v.url));
                                        close = true;
                                    }
                                    if crate::hw::button(ui, vec2(80.0, 28.0), tr("Später"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                                        close = true;
                                    }
                                }
                            }
                        }
                    });
                });
        });
        if let Some(on) = answer {
            self.settings.update_check = Some(on);
            self.update.ask = false;
            self.save_settings();
            if on {
                self.update.rx = Some(crate::update::check_in_background(ctx.clone()));
            }
        }
        if let Some(url) = install {
            self.update.progress = Some(Progress::Downloading);
            self.update.installing = Some(crate::self_update::start(url, ctx.clone()));
        }
        if close {
            self.update.new_version = None;
            self.update.progress = None;
        }
    }

    pub(super) fn save_settings(&self) {
        if self.persist {
            self.settings.save();
        }
    }

    /// „FadeWerk DJ zurücksetzen“: Einstellungsordner leeren und neu starten. Danach wird nichts mehr
    /// gespeichert, sonst schriebe das Beenden die alten Einstellungen gleich wieder hin.
    pub(super) fn reset_all(&mut self, ctx: &egui::Context, now: f64) {
        if self.ctl.shared.decks.iter().any(|s| s.is_playing()) {
            return;
        }
        if !self.persist {
            self.notify(tr("Entwicklerlauf: es wird nichts gelöscht."), now);
            return;
        }
        self.persist = false;
        self.save_due = None;
        self.lib.set_sketch_cache(None);
        if let Err(e) = crate::caches::reset_all() {
            log::warn!("Zurücksetzen unvollständig: {e}");
        }
        match crate::self_update::restart_after_exit(crate::self_update::AFTER_RESET_FLAG) {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => {
                log::warn!("Neustart nach dem Zurücksetzen nicht möglich: {e}");
                self.notify(tr("Zurückgesetzt. Bitte FadeWerk DJ neu starten."), now);
            }
        }
    }

    /// Speichert die Playlists, wenn sie sich seit dem letzten Mal geändert haben.
    pub(super) fn save_session_if_changed(&mut self) {
        if !self.persist {
            return;
        }
        let fp = crate::session::fingerprint(&self.playlists);
        if fp != self.session_saved {
            crate::session::save(&self.playlists);
            self.session_saved = fp;
        }
    }

    /// Merkt die Stellen der geladenen Titel in `player.json` (auf ganze Sekunden), für einen Neustart nach Absturz.
    pub(super) fn save_places_if_changed(&mut self) {
        if !self.persist {
            return;
        }
        let places: [Option<crate::session::Place>; 2] = std::array::from_fn(|d| {
            // Noch nicht angewandte Wiederherstellung bleibt stehen (der Titel lädt noch)
            if let Some(r) = &self.resume[d] {
                return Some(r.clone());
            }
            let v = &self.decks[d];
            let path = v.path.clone()?;
            let secs = self.ctl.shared.decks[d].position_secs().floor();
            Some(crate::session::Place { path, secs, pitch_pct: v.pitch_pct })
        });
        if places != self.places_saved {
            crate::session::save_places(&places);
            self.places_saved = places;
        }
    }
}
