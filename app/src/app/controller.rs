// SPDX-License-Identifier: GPL-3.0-or-later
//! DJ-Controller: MIDI-Nachrichten über dieselben Aktionen ausführen wie die Maus, Lernen (in den Einstellungen
//! und als Lernmodus in der Oberfläche), Soft-Takeover für Fader und Drehregler.

use super::*;
use crate::midi::{Control, Event, Learn};

impl FadeWerkDjApp {
    /// Geräte suchen, Nachrichten ausführen bzw. lernen. Läuft in `logic`, also auch bei verdecktem Fenster.
    pub(super) fn step_midi(&mut self, ctx: &egui::Context, now: f64) {
        self.midi.scan(now, ctx);
        let msgs = self.midi.poll(now);
        // Gesperrte Oberfläche: auch der Controller bewirkt nichts (Nachrichten werden verworfen)
        if self.lock.active() {
            return;
        }
        if let Some(learn) = &mut self.midi_learn {
            for (dev, m) in &msgs {
                learn.feed(dev, *m, now);
            }
            if learn.got.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(crate::midi::LEARN_SETTLE_SECS));
            }
            if let Some(b) = learn.done(now) {
                let in_ui = learn.in_ui;
                self.midi_learn = in_ui.then_some(Learn { control: None, got: None, in_ui: true });
                let (name, input) = (b.control.full_name(), b.input.label());
                let lost = crate::midi::bind(&mut self.settings.midi_bindings, b);
                self.save_settings();
                let msg = match lost.first() {
                    Some(old) => trf("{} ← {} (vorher {})", &[&name, &input, &old.full_name()]),
                    None => trf("{} ← {}", &[&name, &input]),
                };
                self.notify(msg, now);
            }
            // Beim Lernen nichts auslösen
            return;
        }
        if msgs.is_empty() {
            return;
        }
        let bindings = self.settings.midi_bindings.clone();
        for (dev, m) in msgs {
            for b in &bindings {
                if let Some(ev) = self.midi.resolve(b, &dev, m) {
                    self.apply_control(b.control, ev, now);
                }
            }
        }
    }

    /// Aktueller Wert einer Funktion als Anteil 0 bis 1 (für Soft-Takeover), `None` für Tasten.
    fn control_value(&self, c: Control) -> Option<f32> {
        Some(match c {
            Control::Pitch(d) => (self.decks[d].pitch_pct + PITCH_RANGE) / (2.0 * PITCH_RANGE),
            Control::Filter(d) => (self.filter[d] + 1.0) / 2.0,
            Control::Volume(d) => self.decks[d].volume,
            Control::Eq(d, b) => crate::mixer_ui::eq_to_norm(self.eq_db[d][b]),
            Control::Crossfader => (self.crossfader + 1.0) / 2.0,
            Control::Master => self.master_volume,
            Control::HeadMix => self.head_mix,
            Control::HeadVolume => self.head_volume,
            _ => return None,
        })
    }

    fn apply_control(&mut self, c: Control, ev: Event, now: f64) {
        let v = match ev {
            Event::Value(v) => {
                let Some(sw) = self.control_value(c) else { return };
                // Hand gewinnt: Bewegt sich der Crossfader am Gerät, endet eine laufende Automatik-Überblendung,
                // auch wenn er erst noch übernehmen muss
                if c == Control::Crossfader && self.autofade.fade().is_some() {
                    self.autofade.cancel();
                }
                if !self.midi.take_over(c, v, sw) {
                    return;
                }
                v
            }
            Event::Press => {
                self.press_control(c, now);
                return;
            }
        };
        match c {
            Control::Pitch(d) => self.deck_action(d, DeckAction::Pitch((((v * 2.0 - 1.0) * PITCH_RANGE) * 10.0).round() / 10.0), now),
            Control::Filter(d) => {
                let f = v * 2.0 - 1.0;
                self.deck_action(d, DeckAction::Filter(if f.abs() < 0.04 { 0.0 } else { f }), now);
            }
            Control::Volume(d) => self.mixer_action(MixerAction::Volume(d, v), now),
            Control::Eq(deck, band) => self.mixer_action(MixerAction::Eq { deck, band, db: crate::mixer_ui::eq_from_norm(v) }, now),
            Control::Crossfader => self.fade_action(FadeAction::Crossfader(v * 2.0 - 1.0), now),
            Control::Master => self.mixer_action(MixerAction::MasterVolume(v), now),
            Control::HeadMix => self.mixer_action(MixerAction::HeadMix(v), now),
            Control::HeadVolume => self.mixer_action(MixerAction::HeadVolume(v), now),
            _ => {}
        }
    }

    fn press_control(&mut self, c: Control, now: f64) {
        match c {
            Control::Play(d) => self.deck_action(d, DeckAction::TogglePlay, now),
            Control::Cue(d) => self.deck_action(d, DeckAction::Cue, now),
            Control::Stop(d) => self.deck_action(d, DeckAction::Stop, now),
            Control::Prev(d) => self.deck_action(d, DeckAction::Prev, now),
            Control::Next(d) => self.deck_action(d, DeckAction::Next, now),
            Control::Sync(d) => self.deck_action(d, DeckAction::Sync, now),
            Control::Pfl(d) => self.deck_action(d, DeckAction::Pfl(!self.decks[d].pfl), now),
            Control::Reverse(d) => self.deck_action(d, DeckAction::Reverse(!self.decks[d].reverse), now),
            Control::Repeat(d) => self.deck_action(d, DeckAction::CycleRepeat, now),
            Control::Loop(d, beats) => self.deck_action(d, DeckAction::Loop(beats), now),
            Control::Kill(deck, band) => self.mixer_action(MixerAction::EqKill { deck, band, on: !self.eq_kill[deck][band] }, now),
            Control::Auto => self.fade_action(FadeAction::Auto(!self.autofade.enabled), now),
            Control::Fade => self.fade_action(FadeAction::FadeNow, now),
            Control::BassSwap => self.fade_action(FadeAction::BassSwap(!self.bass_swap), now),
            Control::AutoSync => self.fade_action(FadeAction::AutoSync(!self.auto_sync), now),
            Control::Pad(slot) => self.mixer_action(MixerAction::Fx { slot, to_master: true }, now),
            Control::Mic => self.mixer_action(MixerAction::Mic, now),
            Control::Record => self.mixer_action(MixerAction::Record, now),
            _ => {}
        }
    }

    /// Lernmodus in der Oberfläche: Was das angeklickte Bedienelement auslösen würde, wird zum Lernziel statt
    /// ausgeführt. Liefert `true`, wenn die Aktion damit erledigt ist.
    pub(super) fn learn_deck(&mut self, d: usize, a: &DeckAction) -> bool {
        let c = match a {
            DeckAction::TogglePlay => Control::Play(d),
            DeckAction::Cue => Control::Cue(d),
            DeckAction::Stop => Control::Stop(d),
            DeckAction::Prev => Control::Prev(d),
            DeckAction::Next => Control::Next(d),
            DeckAction::Sync => Control::Sync(d),
            DeckAction::Pfl(_) => Control::Pfl(d),
            DeckAction::Reverse(_) => Control::Reverse(d),
            DeckAction::CycleRepeat => Control::Repeat(d),
            DeckAction::Loop(b) => Control::Loop(d, *b),
            DeckAction::Pitch(_) => Control::Pitch(d),
            DeckAction::Filter(_) => Control::Filter(d),
            _ => return self.learning_in_ui(),
        };
        self.learn_pick(c)
    }

    pub(super) fn learn_mixer(&mut self, a: &MixerAction) -> bool {
        let c = match a {
            MixerAction::Volume(d, _) => Control::Volume(*d),
            MixerAction::Eq { deck, band, .. } => Control::Eq(*deck, *band),
            MixerAction::EqKill { deck, band, .. } => Control::Kill(*deck, *band),
            MixerAction::MasterVolume(_) => Control::Master,
            MixerAction::HeadMix(_) => Control::HeadMix,
            MixerAction::HeadVolume(_) => Control::HeadVolume,
            MixerAction::Mic => Control::Mic,
            MixerAction::Record => Control::Record,
            MixerAction::Fx { slot, .. } => Control::Pad(*slot),
            _ => return self.learning_in_ui(),
        };
        self.learn_pick(c)
    }

    pub(super) fn learn_fade(&mut self, a: &FadeAction) -> bool {
        let c = match a {
            FadeAction::Crossfader(_) => Control::Crossfader,
            FadeAction::Auto(_) => Control::Auto,
            FadeAction::FadeNow => Control::Fade,
            FadeAction::BassSwap(_) => Control::BassSwap,
            FadeAction::AutoSync(_) => Control::AutoSync,
            _ => return self.learning_in_ui(),
        };
        self.learn_pick(c)
    }

    /// Im Lernmodus tut die Oberfläche nichts anderes (auch Bedienelemente ohne Controller-Funktion).
    fn learning_in_ui(&self) -> bool {
        self.midi_learn.as_ref().is_some_and(|l| l.in_ui)
    }

    fn learn_pick(&mut self, c: Control) -> bool {
        match &mut self.midi_learn {
            Some(l) if l.in_ui => {
                l.control = Some(c);
                l.got = None;
                true
            }
            _ => false,
        }
    }

    /// Lernen starten: für eine Funktion (Einstellungen) oder als Lernmodus in der Oberfläche (`None`).
    pub(super) fn start_learn(&mut self, c: Option<Control>, now: f64) {
        self.midi_learn = Some(Learn { control: c, got: None, in_ui: c.is_none() });
        if c.is_none() {
            self.settings_ui.open = false;
            self.notify(tr("Bedienelement anklicken, dann am Controller bewegen. Esc beendet."), now);
        }
    }

    /// Balken oben in der Mitte, solange in der Oberfläche zugewiesen wird: was gewählt ist und wie es weitergeht.
    pub(super) fn learn_banner(&mut self, ctx: &egui::Context, now: f64) {
        let Some(learn) = self.midi_learn.as_ref().filter(|l| l.in_ui) else { return };
        let text = match learn.control {
            None => tr("Controller zuweisen: Bedienelement anklicken").to_owned(),
            Some(c) => trf("{}: jetzt am Controller drücken oder bewegen", &[&c.full_name()]),
        };
        let mut done = false;
        egui::Area::new(egui::Id::new("midi_learn")).anchor(Align2::CENTER_TOP, vec2(0.0, 52.0)).show(ctx, |ui| {
            Frame::new()
                .fill(theme::panel2())
                .stroke(Stroke::new(1.5, theme::select_accent()))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Blinkender Punkt: Lernmodus ist an
                        let on = (now * 2.0).fract() < 0.6;
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.5, if on { theme::select_accent() } else { theme::faint() });
                        ui.label(RichText::new(text).strong().color(theme::ink()));
                        ui.add_space(8.0);
                        if crate::hw::button(ui, vec2(110.0, 26.0), tr("Fertig (Esc)"), crate::fonts::bold(12.5), crate::hw::Look::Plain, false, true).clicked() {
                            done = true;
                        }
                    });
                });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        if done {
            self.midi_learn = None;
        }
    }
}
