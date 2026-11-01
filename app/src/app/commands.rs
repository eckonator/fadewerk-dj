// SPDX-License-Identifier: GPL-3.0-or-later
//! Statuszeile und Befehlssuche: Arbeitsweise, Modell der Statuszeile, Einträge und Ausführung der Befehle.

use super::*;

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
        StatusModel {
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
        let parts: [(&str, Pick); 7] = [
            (tr("Einfache Formen"), |l| &mut l.shapes),
            (tr("Kantenglättung aus"), |l| &mut l.no_feathering),
            (tr("Ohne Schatten und Leuchten"), |l| &mut l.flat),
            (tr("Grobe Wellenformen"), |l| &mut l.coarse_waves),
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
}
