// SPDX-License-Identifier: GPL-3.0-or-later
//! Pfeiltasten und Eingabetaste in den Titellisten (Ordnerbrowser, Playlist A und B).
//! Die Tasten gehen an die Liste, in der zuletzt ein Titel angeklickt wurde. Eingesammelt wird am Ende eines
//! Bildes, damit Regler unter der Maus (siehe `hw::nudge_steps`) die Pfeiltasten vorher bekommen; die Liste
//! wendet sie im nächsten Bild an.

use eframe::egui::{self, Context, Id};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyList {
    Browser,
    Playlist(usize),
}

/// Angefallene Tasten für eine Liste: Schritte (↓ = +1, ↑ = −1), mit ⇧ (Auswahl erweitern), Eingabetaste.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Keys {
    pub steps: i32,
    pub extend: bool,
    pub enter: bool,
}

fn target_id() -> Id {
    Id::new("keynav_target")
}

fn pending_id() -> Id {
    Id::new("keynav_pending")
}

/// Klick in eine Liste: sie bekommt ab jetzt die Tasten.
pub fn focus(ctx: &Context, list: KeyList) {
    ctx.data_mut(|d| d.insert_temp(target_id(), list));
}

pub fn focused(ctx: &Context) -> Option<KeyList> {
    ctx.data(|d| d.get_temp(target_id()))
}

/// Am Ende des Bildes: übrige Pfeil- und Eingabetasten für die aktive Liste aufheben.
/// Nicht beim Tippen, nicht bei offenem Menü und nicht mit ⌘/Strg/⌥.
pub fn collect(ctx: &Context) {
    let Some(list) = focused(ctx) else { return };
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
        return;
    }
    let mut keys = Keys::default();
    ctx.input_mut(|i| {
        i.events.retain(|e| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { return true };
            if modifiers.command || modifiers.ctrl || modifiers.alt {
                return true;
            }
            match key {
                egui::Key::ArrowDown => keys.steps += 1,
                egui::Key::ArrowUp => keys.steps -= 1,
                egui::Key::Enter if !modifiers.shift => keys.enter = true,
                _ => return true,
            }
            keys.extend |= modifiers.shift;
            false
        });
    });
    if keys != Keys::default() {
        ctx.data_mut(|d| d.insert_temp(pending_id(), (list, keys)));
        ctx.request_repaint();
    }
}

/// Anstehende Tasten für `list`, falls sie die aktive Liste ist.
pub fn take(ctx: &Context, list: KeyList) -> Option<Keys> {
    let (target, keys) = ctx.data(|d| d.get_temp::<(KeyList, Keys)>(pending_id()))?;
    if target != list {
        return None;
    }
    ctx.data_mut(|d| d.remove::<(KeyList, Keys)>(pending_id()));
    Some(keys)
}

/// Neue Stelle nach `steps` Schritten ab `from` in einer Liste mit `len` Einträgen (am Rand bleibt sie stehen).
pub fn step(from: Option<usize>, steps: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(from) = from.filter(|&f| f < len) else { return Some(0) };
    Some((from as i64 + steps as i64).clamp(0, len as i64 - 1) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(step(None, 1, 0), None);
        assert_eq!(step(None, 3, 5), Some(0));
        assert_eq!(step(Some(2), 1, 5), Some(3));
        assert_eq!(step(Some(0), -1, 5), Some(0));
        assert_eq!(step(Some(4), 2, 5), Some(4));
        assert_eq!(step(Some(9), -1, 5), Some(0));
    }
}
