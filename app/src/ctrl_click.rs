// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS: Ctrl+Klick wirkt wie die rechte Maustaste (Kontextmenüs, Rechtsklick-Funktionen).
//!
//! Andere Mac-Programme bekommen das von AppKit übersetzt; winit meldet aber einen Linksklick mit gedrückter
//! Ctrl-Taste, und egui öffnet Menüs nur bei der zweiten Taste. Auf Trackpads ohne eingerichteten Sekundärklick
//! (etwa einem MacBook Pro von 2008) gab es so keinen Weg in die Kontextmenüs. Als egui-Plugin greift es in jedem
//! Fenster, auch im ausgeklinkten Ordnerfenster (dort ruft eframe `App::raw_input_hook` nicht auf).

use std::collections::HashSet;

use eframe::egui::{self, Event, PointerButton, ViewportId};

#[derive(Default)]
pub struct CtrlClick {
    /// Fenster, in denen gerade ein umgeschriebener Klick gedrückt ist (das Loslassen gehört dann auch dazu).
    held: HashSet<ViewportId>,
}

impl egui::Plugin for CtrlClick {
    fn debug_name(&self) -> &'static str {
        "Ctrl+Klick als Rechtsklick"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let mut held = self.held.contains(&input.viewport_id);
        convert(&mut input.events, &mut held);
        if held {
            self.held.insert(input.viewport_id);
        } else {
            self.held.remove(&input.viewport_id);
        }
    }
}

/// Schreibt Ctrl+Linksklick in einen Rechtsklick um. `held`: ein umgeschriebener Klick ist gedrückt.
fn convert(events: &mut [Event], held: &mut bool) {
    for event in events {
        match event {
            Event::PointerButton { button, pressed: true, modifiers, .. }
                if *button == PointerButton::Primary && modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.alt =>
            {
                *button = PointerButton::Secondary;
                // Ctrl war hier nur der Weg zur rechten Taste, kein Zusatz zum Klick
                modifiers.ctrl = false;
                *held = true;
            }
            // Loslassen auch dann als rechte Taste, wenn Ctrl schon vorher losgelassen wurde
            Event::PointerButton { button, pressed: false, modifiers, .. } if *held && *button == PointerButton::Primary => {
                *button = PointerButton::Secondary;
                modifiers.ctrl = false;
                *held = false;
            }
            Event::PointerGone => *held = false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Modifiers, pos2};

    fn click(pressed: bool, button: PointerButton, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos: pos2(10.0, 10.0), button, pressed, modifiers }
    }

    fn buttons(events: &[Event]) -> Vec<(PointerButton, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::PointerButton { button, pressed, .. } => Some((*button, *pressed)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ctrl_click_becomes_secondary() {
        let mut held = false;
        let mut events = vec![click(true, PointerButton::Primary, Modifiers::CTRL), click(false, PointerButton::Primary, Modifiers::CTRL)];
        convert(&mut events, &mut held);
        assert_eq!(buttons(&events), vec![(PointerButton::Secondary, true), (PointerButton::Secondary, false)]);
        assert!(!held);
    }

    #[test]
    fn release_after_letting_go_of_ctrl_still_secondary() {
        let mut held = false;
        let mut first = vec![click(true, PointerButton::Primary, Modifiers::CTRL)];
        convert(&mut first, &mut held);
        assert!(held);
        // Nächstes Bild: Ctrl ist schon los, die Taste kommt erst jetzt hoch
        let mut second = vec![click(false, PointerButton::Primary, Modifiers::NONE)];
        convert(&mut second, &mut held);
        assert_eq!(buttons(&second), vec![(PointerButton::Secondary, false)]);
        assert!(!held);
    }

    #[test]
    fn plain_and_command_clicks_stay_primary() {
        let mut held = false;
        let mut events = vec![
            click(true, PointerButton::Primary, Modifiers::NONE),
            click(false, PointerButton::Primary, Modifiers::NONE),
            click(true, PointerButton::Primary, Modifiers::MAC_CMD | Modifiers::CTRL),
            click(false, PointerButton::Primary, Modifiers::MAC_CMD | Modifiers::CTRL),
        ];
        convert(&mut events, &mut held);
        assert!(buttons(&events).iter().all(|(b, _)| *b == PointerButton::Primary));
    }
}
