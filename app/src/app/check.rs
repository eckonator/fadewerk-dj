// SPDX-License-Identifier: GPL-3.0-or-later
//! Playlists prüfen: fehlende Dateien wiederfinden (auch beim Start), unlesbare und nicht analysierte Titel zeigen.

use std::collections::HashSet;

use super::*;
use crate::relocate::Checked;

/// Wofür eine laufende Prüfung (`relocate::check_in_background`) gedacht ist.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum CheckFor {
    /// Beim Start: Verschobene übernehmen, was fehlt, fliegt aus den Playlists.
    Startup,
    /// Fenster „Playlist prüfen“: nur anzeigen, ändern erst per Taste.
    Dialog,
}

/// Fenster „Playlist prüfen“ für Player `deck`.
pub(super) struct CheckDialog {
    deck: usize,
    /// `None`, solange die Prüfung läuft.
    result: Option<Checked>,
}

impl FadeWerkDjApp {
    /// Prüft die Dateien von `paths` im Hintergrund.
    pub(super) fn start_check(&mut self, paths: Vec<PathBuf>, what: CheckFor) {
        let ctx = self.repaint_ctx.clone();
        // Eine neue Prüfung im Fenster ersetzt die alte; die beim Start läuft weiter
        self.check_rx.retain(|(w, _)| *w != what);
        self.check_rx.push((what, crate::relocate::check_in_background(paths, self.music_roots(), move || ctx.request_repaint())));
    }

    /// „Playlist prüfen …“: Fenster öffnen und die Prüfung starten. Tags aller Titel werden dafür gelesen.
    pub(super) fn open_check(&mut self, d: usize) {
        let paths = self.playlists[d].entries.clone();
        for p in &paths {
            self.request_meta(p.clone());
        }
        self.check_dialog = Some(CheckDialog { deck: d, result: None });
        self.start_check(paths, CheckFor::Dialog);
    }

    pub(super) fn poll_check(&mut self, now: f64) {
        let mut done = Vec::new();
        self.check_rx.retain(|(what, rx)| match rx.try_recv() {
            Ok(checked) => {
                done.push((*what, checked));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for (what, checked) in done {
            match what {
                CheckFor::Startup => self.apply_startup_check(checked, now),
                CheckFor::Dialog => {
                    if let Some(dlg) = &mut self.check_dialog {
                        dlg.result = Some(checked);
                    }
                }
            }
        }
    }

    /// Ersetzt verschobene Titel in beiden Playlists durch ihren neuen Ort; liefert die Zahl der Einträge.
    fn take_moved(&mut self, moved: &std::collections::HashMap<PathBuf, PathBuf>) -> usize {
        let mut n = 0;
        for pl in &mut self.playlists {
            for e in &mut pl.entries {
                if let Some(p) = moved.get(e) {
                    *e = p.clone();
                    n += 1;
                }
            }
        }
        n
    }

    /// Entfernt die Einträge mit diesen Pfaden aus Playlist `d` (ein geladener Titel bleibt).
    fn remove_paths(&mut self, d: usize, gone: &HashSet<PathBuf>) -> usize {
        let keep = self.has_track(d).then_some(self.playlists[d].current).flatten();
        let pl = &self.playlists[d];
        let idx: Vec<usize> = (0..pl.entries.len()).filter(|&i| Some(i) != keep && gone.contains(&pl.entries[i])).collect();
        self.playlists[d].remove_many(&idx);
        idx.len()
    }

    fn apply_startup_check(&mut self, c: Checked, now: f64) {
        let found = self.take_moved(&c.moved);
        // Inzwischen wieder da (Laufwerk angesteckt)? Dann bleibt der Titel.
        let gone: HashSet<PathBuf> = c.missing.into_iter().filter(|p| !c.moved.contains_key(p) && !p.is_file()).collect();
        let removed = self.remove_paths(0, &gone) + self.remove_paths(1, &gone);
        match (found, removed) {
            (0, 0) => {}
            (f, 0) => self.notify(trf("{} verschobene Titel wiedergefunden", &[&f]), now),
            (0, r) => self.notify(trf("{} Titel aus den Playlists gibt es nicht mehr", &[&r]), now),
            (f, r) => self.notify(trf("{} verschobene Titel wiedergefunden, {} gibt es nicht mehr", &[&f, &r]), now),
        }
        // Player, deren Titel beim Start fehlte, bekommen jetzt einen
        for d in 0..2 {
            let start = self.playlists[d].current.or((!self.playlists[d].entries.is_empty()).then_some(0));
            if let Some(i) = start {
                self.fill_if_empty(d, i);
            }
        }
    }

    /// Fenster „Playlist prüfen“.
    pub(super) fn check_dialog(&mut self, ctx: &egui::Context, now: f64) {
        let Some(dlg) = &self.check_dialog else { return };
        let d = dlg.deck;
        let pl = &self.playlists[d];
        let total = pl.entries.len();
        // Was zu tun ist, wird nach dem Fenster ausgeführt
        enum Do {
            Close,
            Again,
            TakeMoved,
            RemoveMissing,
            RemoveUnreadable,
            Analyze(Vec<PathBuf>),
        }
        let mut todo = None;
        let font = egui::FontId::proportional(13.0);
        let checked = dlg.result.as_ref();
        let missing: Vec<&PathBuf> = checked.map(|c| pl.entries.iter().filter(|p| c.missing.contains(p)).collect()).unwrap_or_default();
        let moved = checked.map_or(0, |c| missing.iter().filter(|p| c.moved.contains_key(**p)).count());
        let unreadable: Vec<&PathBuf> = pl.entries.iter().filter(|p| self.unreadable.contains(*p) && !missing.contains(p)).collect();
        let bad: HashSet<&PathBuf> = missing.iter().chain(&unreadable).copied().collect();
        let reading = pl.entries.iter().filter(|p| !self.metas.contains_key(*p) && !bad.contains(p)).count();
        let unanalyzed: Vec<PathBuf> = pl.entries.iter().filter(|p| !bad.contains(p) && self.metas.contains_key(*p) && self.needs_analysis(p)).cloned().collect();
        let analyzing = !self.bpm_pending.is_empty();
        let modal = egui::Modal::new(egui::Id::new("playlist_pruefen"))
            .frame(egui::Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(egui::Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new(trf("Playlist {} prüfen", &[&theme::deck_letter(d)])).font(crate::fonts::bold(17.0)));
                ui.add_space(8.0);
                let row = |ui: &mut egui::Ui, ok: bool, text: String, buttons: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(30.0);
                        let color = if ok { theme::ink() } else { theme::cue() };
                        ui.label(RichText::new(text).color(color));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui));
                    });
                };
                if checked.is_none() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(trf("Prüft {} Titel und sucht fehlende in deinen Musikordnern …", &[&total]));
                    });
                } else {
                    row(ui, missing.is_empty(), trf("Fehlende Dateien: {}", &[&missing.len()]), &mut |ui| {
                        if !missing.is_empty() && crate::hw::button(ui, vec2(90.0, 26.0), tr("Entfernen"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                            todo = Some(Do::RemoveMissing);
                        }
                        if moved > 0 && crate::hw::button(ui, vec2(150.0, 26.0), &trf("{} wiedergefunden", &[&moved]), font.clone(), crate::hw::Look::Color(theme::play()), true, true)
                            .on_hover_text(tr("An anderer Stelle in deinen Musikordnern gefunden (gleicher Dateiname). Klick übernimmt den neuen Ort."))
                            .clicked()
                        {
                            todo = Some(Do::TakeMoved);
                        }
                    });
                }
                row(ui, unreadable.is_empty(), trf("Nicht abspielbar: {}", &[&unreadable.len()]), &mut |ui| {
                    if !unreadable.is_empty() && crate::hw::button(ui, vec2(90.0, 26.0), tr("Entfernen"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                        todo = Some(Do::RemoveUnreadable);
                    }
                });
                let text = if reading > 0 { trf("Ohne BPM oder Tonart: {} (liest noch {} Tags …)", &[&unanalyzed.len(), &reading]) } else { trf("Ohne BPM oder Tonart: {}", &[&unanalyzed.len()]) };
                row(ui, unanalyzed.is_empty(), text, &mut |ui| {
                    if !unanalyzed.is_empty() {
                        let label = if analyzing { tr("Analysiert …") } else { tr("Jetzt analysieren") };
                        if crate::hw::button(ui, vec2(150.0, 26.0), label, font.clone(), crate::hw::Look::Plain, false, !analyzing).clicked() {
                            todo = Some(Do::Analyze(unanalyzed.clone()));
                        }
                    }
                });
                // Namen der Problemfälle
                if !bad.is_empty() {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                        for p in missing.iter().chain(&unreadable) {
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let note = match checked.and_then(|c| c.moved.get(*p)) {
                                Some(to) => trf("→ {}", &[&to.parent().map(|f| f.display().to_string()).unwrap_or_default()]),
                                None => p.parent().map(|f| f.display().to_string()).unwrap_or_default(),
                            };
                            ui.label(RichText::new(name).color(theme::ink())).on_hover_text(p.display().to_string());
                            ui.label(RichText::new(note).small().color(theme::dim()));
                        }
                    });
                }
                if checked.is_some() && bad.is_empty() && unanalyzed.is_empty() && reading == 0 {
                    ui.add_space(4.0);
                    ui.label(RichText::new(trf("Alles in Ordnung: alle {} Titel sind da und analysiert.", &[&total])).color(theme::play()));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if crate::hw::button(ui, vec2(120.0, 28.0), tr("Erneut prüfen"), font.clone(), crate::hw::Look::Plain, false, checked.is_some()).clicked() {
                        todo = Some(Do::Again);
                    }
                    if crate::hw::button(ui, vec2(100.0, 28.0), tr("Schließen"), font.clone(), crate::hw::Look::Plain, false, true).clicked() {
                        todo = Some(Do::Close);
                    }
                });
            });
        if todo.is_none() && modal.should_close() {
            todo = Some(Do::Close);
        }
        let gone = |list: &[&PathBuf]| list.iter().map(|p| (*p).clone()).collect::<HashSet<PathBuf>>();
        match todo {
            None => {}
            Some(Do::Close) => {
                self.check_dialog = None;
                self.check_rx.retain(|(w, _)| *w != CheckFor::Dialog);
            }
            Some(Do::Again) => self.open_check(d),
            Some(Do::TakeMoved) => {
                let moved = self.check_dialog.as_mut().and_then(|c| c.result.as_mut()).map(|r| std::mem::take(&mut r.moved)).unwrap_or_default();
                let n = self.take_moved(&moved);
                if let Some(r) = self.check_dialog.as_mut().and_then(|c| c.result.as_mut()) {
                    r.missing.retain(|p| !moved.contains_key(p));
                }
                self.notify(trf("{} Titel an ihren neuen Ort übernommen", &[&n]), now);
            }
            Some(Do::RemoveMissing) => {
                let set = gone(&missing);
                let n = self.remove_paths(d, &set);
                self.notify(trf("{} fehlende Titel entfernt", &[&n]), now);
            }
            Some(Do::RemoveUnreadable) => {
                let set = gone(&unreadable);
                let n = self.remove_paths(d, &set);
                self.notify(trf("{} nicht abspielbare Titel entfernt", &[&n]), now);
            }
            Some(Do::Analyze(paths)) => self.browser_action(BrowserAction::Analyze { paths, force: false }, now),
        }
    }
}
