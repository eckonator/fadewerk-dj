// SPDX-License-Identifier: GPL-3.0-or-later
//! Statuszeile und Befehlssuche: Arbeitsweise, Modell der Statuszeile, Einträge und Ausführung der Befehle.

use super::*;
use crate::statusbar::SysLine;

impl FadeWerkDjApp {
    /// Befehlssuche zeigen, solange sie offen ist, und den gewählten Befehl ausführen.
    pub(super) fn show_palette(&mut self, ctx: &egui::Context, now: f64) {
        if !self.palette.open {
            return;
        }
        let mut cmds = self.palette_commands();
        let items: Vec<crate::palette::Item> = cmds.iter().map(|(it, _)| crate::palette::Item { label: it.0.clone(), group: it.1.clone(), keys: it.2 }).collect();
        if let Some(i) = self.palette.show(ctx, &items) {
            let (_, cmd) = cmds.swap_remove(i);
            self.run_command(cmd, ctx, now);
        }
    }

    /// Arbeitsweise: gewählt oder nach Bildschirm (kleine Bildschirme Automatik).
    pub(super) fn view(&self) -> ViewMode {
        self.settings.view.unwrap_or(if self.small_screen { ViewMode::Auto } else { ViewMode::Live })
    }

    /// Was die Statuszeile zeigt.
    pub(super) fn status_model(&self, t: &mixer_ui::TransitionView, ctx: &egui::Context) -> StatusModel {
        let air = [self.autofade.active, 1 - self.autofade.active].into_iter().find(|&d| self.on_air(d));
        let on_air = air.map(|d| {
            let st = &self.ctl.shared.decks[d];
            let left = st.duration_secs() - st.position_secs();
            let title = match self.decks[d].meta.as_ref().filter(|m| !m.title.is_empty()) {
                Some(m) => m.title.clone(),
                None => self.decks[d].path.as_deref().map(|p| self.title_of(p)).unwrap_or_default(),
            };
            OnAir { deck: d, title, left, warn: st.is_playing() && left > 0.0 && left <= END_WARNING_SECS }
        });
        let to = 1 - t.from;
        let next = match (t.fading, t.until) {
            (Some(_), _) => Some((to, t.next.clone(), None)),
            (None, Some(u)) => Some((to, t.next.clone(), Some(u))),
            _ => None,
        };
        let st = &self.settings;
        let mask = [(st.status_cpu, crate::sysmon::CPU), (st.status_ram, crate::sysmon::RAM), (st.status_temp, crate::sysmon::TEMP), (st.status_battery, crate::sysmon::BATTERY)]
            .iter()
            .filter(|(on, _)| *on)
            .fold(0, |m, (_, bit)| m | bit);
        self.sysmon.set_wanted(mask);
        let stats = self.sysmon.stats();
        let now = ctx.input(|i| i.time);
        let sys = SysLine {
            clock: st.status_clock.then(|| jiff::Zoned::now().strftime("%H:%M").to_string()),
            battery: dev_battery().or(stats.battery).filter(|_| st.status_battery),
            cpu: stats.cpu.filter(|_| st.status_cpu).map(|c| (c, self.audio_load, now < self.dropout_until)),
            ram: stats.ram.filter(|_| st.status_ram),
            temp: stats.temp.filter(|_| st.status_temp),
        };
        StatusModel {
            sys,
            on_air,
            next,
            notice: self.notice.as_ref().map(|(text, _)| text.clone()),
            view: self.view(),
            recording: self.recording.as_ref().map(|r| r.secs()),
            fullscreen: ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)),
        }
    }

    pub(super) fn status_action(&mut self, a: StatusAction, now: f64) {
        match a {
            StatusAction::About => self.settings_ui.open_at("ueber"),
            StatusAction::View(v) => {
                self.settings.view = Some(v);
                self.save_settings();
            }
            StatusAction::Palette => self.palette.open(),
            StatusAction::StopRecording => self.mixer_action(MixerAction::Record, now),
            StatusAction::Fullscreen => self.toggle_fullscreen = true,
            StatusAction::Settings => self.settings_ui.toggle(),
            StatusAction::Lock => self.lock_ui(now),
        }
    }

    /// Einträge der Befehlssuche: (Name, Bereich, Kürzel) und was der Eintrag tut. Schalter zeigen, was ein Klick
    /// bewirkt („einschalten“ bzw. „ausschalten“).
    fn palette_commands(&self) -> Vec<((String, String, &'static str), PaletteCmd)> {
        use theme::Lite;
        let mac = cfg!(target_os = "macos");
        let mut v: Vec<((String, String, &'static str), PaletteCmd)> = Vec::new();
        let mut add = |label: String, group: &str, keys: &'static str, cmd: PaletteCmd| v.push(((label, group.to_owned(), keys), cmd));
        let onoff = |on: bool, label: &str| if on { trf("{} ausschalten", &[&label]) } else { trf("{} einschalten", &[&label]) };

        let view = tr("Ansicht");
        add(tr("Live-Ansicht").into(), view, "", PaletteCmd::View(ViewMode::Live));
        add(tr("Automatik-Ansicht").into(), view, "", PaletteCmd::View(ViewMode::Auto));
        add(tr("Vollbild").into(), view, if mac { "⌃⌘F" } else { "F11" }, PaletteCmd::Fullscreen);
        add(tr("Oberfläche sperren").into(), view, if mac { "⌘L" } else { "Strg+L" }, PaletteCmd::Lock);
        add(tr("Größer zoomen").into(), view, if mac { "⌘+" } else { "Strg++" }, PaletteCmd::Zoom(self.settings.ui_zoom + ZOOM_STEP));
        add(tr("Kleiner zoomen").into(), view, if mac { "⌘−" } else { "Strg+−" }, PaletteCmd::Zoom(self.settings.ui_zoom - ZOOM_STEP));
        add(tr("Zoom 100 %").into(), view, if mac { "⌘0" } else { "Strg+0" }, PaletteCmd::Zoom(1.0));
        if self.compact() && !self.settings.browser_detached {
            add(tr("Ordner einblenden oder ausblenden").into(), view, if mac { "⌘B" } else { "Strg+B" }, PaletteCmd::Slide);
        }

        let fade = tr("Auto-Fade");
        add(onoff(self.autofade.enabled, tr("Auto-Fade")), fade, "", PaletteCmd::Fade(FadeAction::Auto(!self.autofade.enabled)));
        add(tr("Jetzt überblenden (FADE)").into(), fade, "", PaletteCmd::Fade(FadeAction::FadeNow));
        add(onoff(self.bass_swap, "Bass-Swap"), fade, "", PaletteCmd::Fade(FadeAction::BassSwap(!self.bass_swap)));
        add(onoff(self.auto_sync, "Auto-Sync"), fade, "", PaletteCmd::Fade(FadeAction::AutoSync(!self.auto_sync)));
        for (c, name) in [(Curve::Soft, tr("Weich")), (Curve::Linear, tr("Linear")), (Curve::Cut, tr("Hart"))] {
            add(trf("Blendkurve: {}", &[&name]), fade, "", PaletteCmd::Fade(FadeAction::Curve(c)));
        }

        let rec = if self.recording.is_some() { tr("Aufnahme beenden") } else { tr("Mix aufnehmen") };
        add(rec.into(), tr("Aufnahme"), "", PaletteCmd::Mixer(MixerAction::Record));
        let mic = if self.mic.is_some() { tr("Mikrofon ausschalten") } else { tr("Mikrofon einschalten") };
        add(mic.into(), tr("Mikrofon"), "", PaletteCmd::Mixer(MixerAction::Mic));

        let lists = tr("Playlists");
        for d in 0..2 {
            let l = theme::deck_letter(d);
            add(trf("Playlist {} harmonisch sortieren", &[&l]), lists, "", PaletteCmd::Playlist(d, PlaylistAction::Harmonic));
            add(trf("Playlist {} zufällig mischen", &[&l]), lists, "", PaletteCmd::Playlist(d, PlaylistAction::Shuffle));
            add(trf("Playlist {} leeren", &[&l]), lists, "", PaletteCmd::Playlist(d, PlaylistAction::Clear));
        }
        add(tr("Alles leeren (beide Playlists und Player) …").into(), lists, "", PaletteCmd::ClearAll);
        add(onoff(self.fadewerk, tr("FadeWerk-Modus")), lists, "", PaletteCmd::Playlist(0, PlaylistAction::FadeWerk(!self.fadewerk)));

        let folders = tr("Ordner");
        add(tr("Ordner hinzufügen …").into(), folders, "", PaletteCmd::Browser(BrowserAction::AddRoot));
        add(tr("Ordner neu einlesen").into(), folders, if mac { "⌘R" } else { "F5" }, PaletteCmd::Refresh);
        let detach = if self.settings.browser_detached { tr("Ordnerfenster zurückholen") } else { tr("Ordner in eigenem Fenster") };
        add(detach.into(), folders, "", PaletteCmd::Browser(BrowserAction::Detach(!self.settings.browser_detached)));
        if !self.settings.browser_detached && !self.compact() {
            let side = if self.settings.browser_side { tr("Ordner unten zeigen") } else { tr("Ordner rechts zeigen") };
            add(side.into(), folders, "", PaletteCmd::Browser(BrowserAction::ToggleSide));
        }

        let settings = tr("Einstellungen");
        for (key, name) in [
            ("allgemein", tr("Allgemein")),
            ("audio", tr("Audio")),
            ("musik", tr("Musik")),
            ("effekte", tr("Effekte")),
            ("aussehen", tr("Aussehen")),
            ("leistung", tr("Leistung")),
            ("ueber", tr("Über")),
        ] {
            add(trf("Einstellungen: {}", &[&name]), settings, if key == "allgemein" { if mac { "⌘," } else { "Strg+," } } else { "" }, PaletteCmd::Page(key));
        }
        add(tr("Jetzt nach Updates suchen").into(), settings, "", PaletteCmd::Setting(SettingsAction::CheckUpdateNow));
        add(tr("Caches leeren …").into(), settings, "", PaletteCmd::Page("musik"));
        add(tr("Logdatei zeigen").into(), settings, "", PaletteCmd::Setting(SettingsAction::RevealLog));

        let perf = tr("Einstellungen › Leistung");
        let lite = self.settings.lite;
        add(tr("Volle Optik").into(), perf, "", PaletteCmd::FullLook);
        add(tr("Sparsam").into(), perf, "", PaletteCmd::Lean);
        type Pick = fn(&mut Lite) -> &mut bool;
        let parts: [(&str, Pick); 8] = [
            (tr("Einfache Formen"), |l| &mut l.shapes),
            (tr("Kantenglättung aus"), |l| &mut l.no_feathering),
            (tr("Ohne Schatten und Leuchten"), |l| &mut l.flat),
            (tr("Grobe Wellenformen"), |l| &mut l.coarse_waves),
            (tr("Wellenform aus"), |l| &mut l.no_waves),
            (tr("Pads ohne Symbole"), |l| &mut l.simple_pads),
            (tr("Frequenzbänder aus"), |l| &mut l.no_bands),
            (tr("Bildrate begrenzen"), |l| &mut l.frame_limit),
        ];
        for (name, pick) in parts {
            let mut next = lite;
            let on = *pick(&mut next);
            *pick(&mut next) = !on;
            add(onoff(on, name), perf, "", PaletteCmd::Setting(SettingsAction::Lite(next)));
        }
        add(onoff(!self.settings.platters, tr("Plattenspieler ausblenden")), perf, "", PaletteCmd::Setting(SettingsAction::Platters(!self.settings.platters)));
        add(onoff(self.settings.opengl, tr("Grafik über OpenGL")), perf, "", PaletteCmd::Setting(SettingsAction::OpenGl(!self.settings.opengl)));

        let look = tr("Einstellungen › Aussehen");
        for skin in theme::SKINS.iter() {
            add(trf("Skin: {}", &[&skin.name()]), look, "", PaletteCmd::Setting(SettingsAction::Skin(skin.id)));
        }
        add(onoff(self.settings.brushed_steel, tr("Gebürsteter Stahl")), look, "", PaletteCmd::Setting(SettingsAction::Brushed(!self.settings.brushed_steel)));
        for (value, name) in [(None, tr("Automatisch")), (Some(true), tr("An")), (Some(false), tr("Aus"))] {
            add(trf("Kompakte Ansicht: {}", &[&name]), look, "", PaletteCmd::Setting(SettingsAction::Compact(value)));
        }
        for l in i18n::Lang::ALL {
            add(trf("Sprache: {}", &[&l.native_name()]), tr("Einstellungen › Allgemein"), "", PaletteCmd::Setting(SettingsAction::Language(l)));
        }
        v
    }

    fn run_command(&mut self, cmd: PaletteCmd, ctx: &egui::Context, now: f64) {
        match cmd {
            PaletteCmd::View(v) => self.status_action(StatusAction::View(v), now),
            PaletteCmd::Page(page) => self.settings_ui.open_at(page),
            PaletteCmd::Fade(a) => self.fade_action(a, now),
            PaletteCmd::Mixer(a) => self.mixer_action(a, now),
            PaletteCmd::Playlist(d, a) => self.playlist_action(d, a, now),
            PaletteCmd::Setting(a) => self.settings_action(a, ctx, now),
            PaletteCmd::Browser(a) => self.browser_action(a, now),
            PaletteCmd::Fullscreen => self.toggle_fullscreen = true,
            PaletteCmd::Zoom(z) => self.set_zoom(ctx, z),
            PaletteCmd::Slide => self.browser_slid = !self.browser_slid,
            PaletteCmd::Refresh => self.browser.refresh(),
            PaletteCmd::ClearAll => self.confirm_clear_all = true,
            PaletteCmd::Lean => {
                self.settings_action(SettingsAction::Lite(theme::Lite::ALL), ctx, now);
                self.settings_action(SettingsAction::Platters(false), ctx, now);
            }
            PaletteCmd::FullLook => {
                self.settings_action(SettingsAction::Lite(theme::Lite::NONE), ctx, now);
                self.settings_action(SettingsAction::Platters(true), ctx, now);
            }
            PaletteCmd::Lock => self.lock_ui(now),
        }
    }

    /// Oberfläche sperren (Taste in der Statuszeile, ⌘L, Befehlssuche). Offene Suche und Lernmodus enden.
    pub(super) fn lock_ui(&mut self, now: f64) {
        self.palette.open = false;
        self.midi_learn = None;
        self.lock.begin(self.settings.lock_pin.is_some());
        // Ohne Auto-Fade bliebe die Musik nach dem Titel stehen, solange niemand bedienen kann
        if self.autofade.enabled {
            self.notify(tr("Oberfläche gesperrt. Die Musik läuft weiter."), now);
        } else {
            self.fade_action(FadeAction::Auto(true), now);
            self.auto_for_lock = true;
            self.notify(tr("Oberfläche gesperrt. Auto-Fade ist an, bis du entsperrst."), now);
        }
    }

    /// Titel auf Sendung für den Sperrbildschirm (wie in der Statuszeile, dazu Interpret und Fortschritt).
    pub(super) fn lock_now_playing(&self) -> Option<crate::lock::NowPlaying> {
        let d = [self.autofade.active, 1 - self.autofade.active].into_iter().find(|&d| self.on_air(d))?;
        let st = &self.ctl.shared.decks[d];
        let meta = self.decks[d].meta.as_ref();
        let title = match meta.filter(|m| !m.title.is_empty()) {
            Some(m) => m.title.clone(),
            None => self.decks[d].path.as_deref().map(|p| self.title_of(p)).unwrap_or_default(),
        };
        let (position, duration) = (st.position_secs(), st.duration_secs());
        let left = duration - position;
        Some(crate::lock::NowPlaying {
            deck: d,
            title,
            artist: meta.map(|m| m.artist.clone()).unwrap_or_default(),
            position,
            duration,
            warn: st.is_playing() && left > 0.0 && left <= END_WARNING_SECS,
        })
    }

    /// Nach dem Entsperren (oder abgebrochenen Sperren): Auto-Fade wieder aus, wenn es nur für die Sperre an war
    /// (eine laufende Überblendung läuft zu Ende).
    pub(super) fn after_unlock(&mut self, msg: &str, now: f64) {
        if std::mem::take(&mut self.auto_for_lock) {
            self.fade_action(FadeAction::Auto(false), now);
            self.notify(trf("{} Auto-Fade ist wieder aus.", &[&msg]), now);
        } else {
            self.notify(msg.to_owned(), now);
        }
    }
}

/// Was ein Eintrag der Befehlssuche tut.
enum PaletteCmd {
    View(ViewMode),
    /// Einstellungen auf dieser Seite öffnen (Schlüssel wie bei `SettingsUi::open_at`).
    Page(&'static str),
    Fade(FadeAction),
    Mixer(MixerAction),
    Playlist(usize, PlaylistAction),
    Setting(SettingsAction),
    Browser(BrowserAction),
    Fullscreen,
    Zoom(f32),
    /// Ordnerbereich der kompakten Ansicht hereinschieben oder zurück.
    Slide,
    Refresh,
    /// Beide Playlists und Player leeren (mit Rückfrage).
    ClearAll,
    /// Sparsam: alle Sparmaßnahmen an und Plattenspieler aus.
    Lean,
    /// Volle Optik: alle Sparmaßnahmen aus und Plattenspieler an.
    FullLook,
    Lock,
}

/// `FW_BATTERY=<Prozent>[,netz]` (nur Debug): Akkustand vortäuschen, um die Farben zu prüfen.
fn dev_battery() -> Option<crate::sysmon::Battery> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("FW_BATTERY").ok()?;
    let mut parts = v.split(',');
    let percent = parts.next()?.trim().parse().ok()?;
    Some(crate::sysmon::Battery { percent, plugged: parts.next().is_some_and(|p| p.trim() == "netz") })
}
