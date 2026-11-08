// SPDX-License-Identifier: GPL-3.0-or-later
//! DJ-Controller über MIDI: Eingänge öffnen, Nachrichten zerlegen, Zuordnungen (gelernt oder aus einem Profil)
//! und Soft-Takeover für Fader und Drehregler. Ausgeführt wird in `app/controller.rs` über dieselben Aktionen wie
//! mit der Maus. Bauplan: `.claude/plans/midi-controller.md`.
//!
//! midir meldet nicht, wenn ein Gerät kommt oder geht; [`Midi::scan`] vergleicht darum alle 2 s die Portliste.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use midir::{MidiInput, MidiInputConnection};
use serde::{Deserialize, Serialize};

/// So oft wird nach neuen oder verschwundenen Geräten gesehen.
const SCAN_SECS: f64 = 2.0;
/// So lange leuchtet die Aktivitäts-LED eines Geräts nach einer Nachricht.
pub const ACTIVITY_SECS: f64 = 0.15;
/// Abstand zum Wert in FadeWerk, ab dem ein Hardware-Regler sofort übernimmt (Anteil des Wegs).
const TAKEOVER_NEAR: f32 = 0.04;

/// Eine zerlegte MIDI-Nachricht. `value`: Noten-Anschlag bzw. Reglerwert 0–127, Pitch Bend 0–16383.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Msg {
    pub kind: Kind,
    /// Kanal 1–16.
    pub ch: u8,
    /// Noten- bzw. Reglernummer (bei Pitch Bend 0).
    pub num: u8,
    pub value: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Note an (Note aus kommt als Anschlag 0).
    Note,
    Cc,
    PitchBend,
}

/// Zerlegt eine Kanalnachricht; System-Nachrichten (Takt, Active Sensing, SysEx) liefern `None`.
pub fn parse(bytes: &[u8]) -> Option<Msg> {
    let (&status, data) = bytes.split_first()?;
    let ch = (status & 0x0F) + 1;
    let d = |i: usize| data.get(i).copied().map(|b| b & 0x7F);
    match status & 0xF0 {
        0x80 => Some(Msg { kind: Kind::Note, ch, num: d(0)?, value: 0 }),
        0x90 => Some(Msg { kind: Kind::Note, ch, num: d(0)?, value: d(1)? as u16 }),
        0xB0 => Some(Msg { kind: Kind::Cc, ch, num: d(0)?, value: d(1)? as u16 }),
        0xE0 => Some(Msg { kind: Kind::PitchBend, ch, num: 0, value: d(0)? as u16 | (d(1)? as u16) << 7 }),
        _ => None,
    }
}

/// Woher eine Zuordnung ihren Wert bekommt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "in", rename_all = "lowercase")]
pub enum Input {
    Note { ch: u8, num: u8 },
    Cc { ch: u8, num: u8 },
    /// Regler mit 14 Bit: grob unter `num`, fein unter `num + 32`.
    Cc14 { ch: u8, num: u8 },
    PitchBend { ch: u8 },
}

impl Input {
    /// Kurz für die Anzeige, z. B. „Note 11 · K1“.
    pub fn label(self) -> String {
        match self {
            Input::Note { ch, num } => format!("Note {num} · K{ch}"),
            Input::Cc { ch, num } => format!("CC {num} · K{ch}"),
            Input::Cc14 { ch, num } => format!("CC {num}/{} · K{ch}", num + 32),
            Input::PitchBend { ch } => format!("Pitch Bend · K{ch}"),
        }
    }
}

/// Was ein Bedienelement am Controller auslöst. `usize` = Player (0 = A, 1 = B).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Play(usize),
    Cue(usize),
    Stop(usize),
    Prev(usize),
    Next(usize),
    Sync(usize),
    Pfl(usize),
    Reverse(usize),
    Repeat(usize),
    Loop(usize, u32),
    Pitch(usize),
    Filter(usize),
    Volume(usize),
    /// Band 0 = Höhen, 1 = Mitten, 2 = Bässe.
    Eq(usize, usize),
    Kill(usize, usize),
    Crossfader,
    Master,
    HeadMix,
    HeadVolume,
    Auto,
    Fade,
    BassSwap,
    AutoSync,
    Pad(usize),
    Mic,
    Record,
}

/// Gruppe in der Liste der Einstellungen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Deck(usize),
    Mixer,
    AutoFade,
    Pads,
}

impl Control {
    /// Alle Funktionen in der Reihenfolge der Einstellungsseite.
    pub fn all() -> Vec<Control> {
        let mut v = Vec::new();
        for d in 0..2 {
            v.extend([
                Control::Play(d),
                Control::Cue(d),
                Control::Stop(d),
                Control::Prev(d),
                Control::Next(d),
                Control::Sync(d),
                Control::Pfl(d),
                Control::Reverse(d),
                Control::Repeat(d),
                Control::Loop(d, 4),
                Control::Loop(d, 8),
                Control::Loop(d, 16),
                Control::Pitch(d),
                Control::Filter(d),
            ]);
        }
        for d in 0..2 {
            v.push(Control::Volume(d));
            for band in 0..3 {
                v.push(Control::Eq(d, band));
            }
            for band in 0..3 {
                v.push(Control::Kill(d, band));
            }
        }
        v.extend([Control::Master, Control::HeadMix, Control::HeadVolume, Control::Mic, Control::Record]);
        v.extend([Control::Crossfader, Control::Auto, Control::Fade, Control::BassSwap, Control::AutoSync]);
        v.extend((0..fw_engine::FX_SLOTS).map(Control::Pad));
        v
    }

    pub fn group(self) -> Group {
        match self {
            Control::Play(d) | Control::Cue(d) | Control::Stop(d) | Control::Prev(d) | Control::Next(d) | Control::Sync(d)
            | Control::Pfl(d) | Control::Reverse(d) | Control::Repeat(d) | Control::Loop(d, _) | Control::Pitch(d) | Control::Filter(d) => {
                Group::Deck(d)
            }
            Control::Volume(_) | Control::Eq(..) | Control::Kill(..) | Control::Master | Control::HeadMix | Control::HeadVolume | Control::Mic | Control::Record => {
                Group::Mixer
            }
            Control::Crossfader | Control::Auto | Control::Fade | Control::BassSwap | Control::AutoSync => Group::AutoFade,
            Control::Pad(_) => Group::Pads,
        }
    }

    /// Fader oder Drehregler (Wert) statt Taste (Druck)?
    pub fn is_fader(self) -> bool {
        matches!(
            self,
            Control::Pitch(_) | Control::Filter(_) | Control::Volume(_) | Control::Eq(..) | Control::Crossfader | Control::Master | Control::HeadMix | Control::HeadVolume
        )
    }

    /// Name ohne Player (für die Liste unter „Player A/B“), z. B. „Loop 8“, „EQ Mitten“.
    pub fn name(self) -> String {
        use crate::i18n::{tr, trf};
        let band = |b: usize| [tr("Höhen"), tr("Mitten"), tr("Bässe")][b.min(2)];
        match self {
            Control::Play(_) => tr("Play/Pause").into(),
            Control::Cue(_) => tr("Cue").into(),
            Control::Stop(_) => tr("Stopp").into(),
            Control::Prev(_) => tr("Zurück").into(),
            Control::Next(_) => tr("Vor").into(),
            Control::Sync(_) => tr("Sync").into(),
            Control::Pfl(_) => tr("Vorhören").into(),
            Control::Reverse(_) => tr("Rückwärts").into(),
            Control::Repeat(_) => tr("Wiederholen").into(),
            Control::Loop(_, n) => trf("Loop {}", &[&n]),
            Control::Pitch(_) => tr("Pitch").into(),
            Control::Filter(_) => tr("Filter").into(),
            Control::Volume(d) => trf("Pegel {}", &[&crate::theme::deck_letter(d)]),
            Control::Eq(d, b) => trf("EQ {} {}", &[&crate::theme::deck_letter(d), &band(b)]),
            Control::Kill(d, b) => trf("Kill {} {}", &[&crate::theme::deck_letter(d), &band(b)]),
            Control::Crossfader => tr("Crossfader").into(),
            Control::Master => tr("Master").into(),
            Control::HeadMix => tr("Kopfhörer CUE/MST").into(),
            Control::HeadVolume => tr("Kopfhörer Pegel").into(),
            Control::Auto => tr("AUTO").into(),
            Control::Fade => tr("FADE").into(),
            Control::BassSwap => tr("Bass-Swap").into(),
            Control::AutoSync => tr("Auto-Sync").into(),
            Control::Pad(i) => trf("Pad {}", &[&(i + 1)]),
            Control::Mic => tr("Mikrofon").into(),
            Control::Record => tr("Aufnahme").into(),
        }
    }

    /// Name samt Player, z. B. „Play/Pause A“ (für Hinweise).
    pub fn full_name(self) -> String {
        match self.group() {
            Group::Deck(d) => format!("{} {}", self.name(), crate::theme::deck_letter(d)),
            _ => self.name(),
        }
    }
}

/// Eine Zuordnung: Nachricht von diesem Gerät → Funktion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Name des MIDI-Eingangs (so wie das System ihn meldet).
    pub device: String,
    #[serde(flatten)]
    pub input: Input,
    #[serde(rename = "do")]
    pub control: Control,
    /// Regler andersherum (etwa Pitch-Fader: oben = langsamer).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
}

/// Was eine Nachricht für eine Zuordnung bedeutet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Taste gedrückt.
    Press,
    /// Regler steht auf diesem Anteil (0 bis 1, schon umgekehrt, falls gewünscht).
    Value(f32),
}

/// Offene Eingänge, eingehende Nachrichten, Zustand für Tasten (CC) und Soft-Takeover.
#[derive(Default)]
pub struct Midi {
    conns: Vec<(String, MidiInputConnection<()>)>,
    tx: Option<Sender<(String, Vec<u8>)>>,
    rx: Option<Receiver<(String, Vec<u8>)>>,
    last_scan: Option<f64>,
    /// Gefundene Eingänge und wann zuletzt etwas von ihnen kam.
    pub ports: Vec<String>,
    pub activity: HashMap<String, f64>,
    /// Fehler beim Öffnen (für die Einstellungsseite).
    pub error: Option<String>,
    /// Feinanteil von 14-Bit-Reglern: (Gerät, Kanal, Nummer des Grobanteils) → Wert.
    lsb: HashMap<(String, u8, u8), u8>,
    /// Zuletzt gesehener Wert je Taste über CC (gedrückt = ab 64; ausgelöst wird beim Wechsel).
    cc_down: HashMap<(String, Input), bool>,
    /// Soft-Takeover: zuletzt gesehener Hardware-Wert je Funktion.
    hw: HashMap<Control, f32>,
}

impl Midi {
    /// Eingänge suchen (höchstens alle 2 s), neue öffnen, verschwundene schließen. `ctx` wird bei jeder Nachricht
    /// geweckt, sonst warteten Tasten im Ruhezustand auf das nächste Bild.
    pub fn scan(&mut self, now: f64, ctx: &egui::Context) {
        if self.last_scan.is_some_and(|t| now - t < SCAN_SECS) {
            return;
        }
        self.last_scan = Some(now);
        if self.tx.is_none() {
            let (tx, rx) = channel();
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        let probe = match MidiInput::new("FadeWerk DJ") {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let names: Vec<String> = probe.ports().iter().filter_map(|p| probe.port_name(p).ok()).collect();
        self.conns.retain(|(n, _)| names.contains(n));
        for name in &names {
            if self.conns.iter().any(|(n, _)| n == name) {
                continue;
            }
            let Ok(input) = MidiInput::new("FadeWerk DJ") else { continue };
            let Some(port) = input.ports().into_iter().find(|p| input.port_name(p).ok().as_deref() == Some(name)) else { continue };
            let (tx, ctx, tag) = (self.tx.clone().expect("oben angelegt"), ctx.clone(), name.clone());
            match input.connect(&port, "fadewerk-dj-in", move |_, bytes, _| {
                let _ = tx.send((tag.clone(), bytes.to_vec()));
                ctx.request_repaint();
            }, ()) {
                Ok(conn) => {
                    log::info!("MIDI-Eingang geöffnet: {name}");
                    self.conns.push((name.clone(), conn));
                }
                Err(e) => log::warn!("MIDI-Eingang {name} lässt sich nicht öffnen: {e}"),
            }
        }
        if names != self.ports {
            self.ports = names;
        }
        self.error = None;
    }

    /// Eingegangene Nachrichten seit dem letzten Aufruf, 14-Bit-Feinanteile schon verrechnet (siehe [`Self::resolve`]).
    pub fn poll(&mut self, now: f64) -> Vec<(String, Msg)> {
        let Some(rx) = &self.rx else { return Vec::new() };
        let mut out = Vec::new();
        while let Ok((dev, bytes)) = rx.try_recv() {
            let Some(m) = parse(&bytes) else { continue };
            self.activity.insert(dev.clone(), now);
            out.push((dev, m));
        }
        out
    }

    /// Bedeutung der Nachricht für diese Zuordnung (oder `None`, wenn sie nicht passt bzw. nichts auslöst).
    pub fn resolve(&mut self, b: &Binding, dev: &str, m: Msg) -> Option<Event> {
        if b.device != dev {
            return None;
        }
        let raw = match (b.input, m.kind) {
            (Input::Note { ch, num }, Kind::Note) if ch == m.ch && num == m.num => {
                return (m.value > 0 && !b.control.is_fader()).then_some(Event::Press);
            }
            (Input::Cc { ch, num }, Kind::Cc) if ch == m.ch && num == m.num => {
                if !b.control.is_fader() {
                    let down = m.value >= 64;
                    let was = self.cc_down.insert((dev.to_owned(), b.input), down).unwrap_or(false);
                    return (down && !was).then_some(Event::Press);
                }
                m.value as f32 / 127.0
            }
            (Input::Cc14 { ch, num }, Kind::Cc) if ch == m.ch && num == m.num => {
                let lsb = self.lsb.get(&(dev.to_owned(), ch, num)).copied().unwrap_or(0);
                ((m.value << 7) | lsb as u16) as f32 / 16383.0
            }
            (Input::Cc14 { ch, num }, Kind::Cc) if ch == m.ch && num + 32 == m.num => {
                self.lsb.insert((dev.to_owned(), ch, num), m.value as u8);
                return None;
            }
            (Input::PitchBend { ch }, Kind::PitchBend) if ch == m.ch => m.value as f32 / 16383.0,
            _ => return None,
        };
        let v = raw.clamp(0.0, 1.0);
        Some(Event::Value(if b.invert { 1.0 - v } else { v }))
    }

    /// Soft-Takeover: Darf der Hardware-Wert `hw` die Funktion übernehmen, deren Wert in FadeWerk `sw` ist? Ja, wenn
    /// er nah dran ist oder seit der letzten Bewegung über `sw` hinweggefahren ist. Sonst springt nichts.
    pub fn take_over(&mut self, c: Control, hw: f32, sw: f32) -> bool {
        let last = self.hw.insert(c, hw);
        (hw - sw).abs() <= TAKEOVER_NEAR || last.is_some_and(|l| (l - sw).signum() != (hw - sw).signum())
    }
}

/// Laufendes Lernen: Für welche Funktion wird die nächste Nachricht genommen, und was kam schon?
#[derive(Clone, Debug, PartialEq)]
pub struct Learn {
    /// `None` = Lernmodus in der Oberfläche, es ist noch kein Bedienelement gewählt.
    pub control: Option<Control>,
    /// Erste passende Nachricht (Gerät, Eingang) und Zeitpunkt; danach kann noch der Feinanteil eines
    /// 14-Bit-Reglers kommen.
    pub got: Option<(String, Input, f64)>,
    /// Lernmodus in der Oberfläche (bleibt nach einer Zuordnung an, bis Esc).
    pub in_ui: bool,
}

/// So lange nach der ersten Nachricht wird auf den Feinanteil eines 14-Bit-Reglers gewartet.
pub const LEARN_SETTLE_SECS: f64 = 0.4;

impl Learn {
    /// Nachricht während des Lernens: Merkt die erste brauchbare und erkennt danach 14-Bit-Regler.
    /// Tasten nehmen nur Note an bzw. CC ab 64 (das Loslassen zählt nicht).
    pub fn feed(&mut self, dev: &str, m: Msg, now: f64) {
        let Some(control) = self.control else { return };
        match &mut self.got {
            None => {
                let input = match m.kind {
                    Kind::Note if m.value > 0 => Input::Note { ch: m.ch, num: m.num },
                    Kind::Cc if control.is_fader() || m.value >= 64 => Input::Cc { ch: m.ch, num: m.num },
                    Kind::PitchBend => Input::PitchBend { ch: m.ch },
                    _ => return,
                };
                self.got = Some((dev.to_owned(), input, now));
            }
            Some((d, input, _)) if d == dev => {
                if let Input::Cc { ch, num } = *input
                    && m.kind == Kind::Cc
                    && m.ch == ch
                    && m.num == num + 32
                    && num < 32
                    && control.is_fader()
                {
                    *input = Input::Cc14 { ch, num };
                }
            }
            _ => {}
        }
    }

    /// Fertige Zuordnung, sobald nach der ersten Nachricht genug Zeit verstrichen ist.
    pub fn done(&self, now: f64) -> Option<Binding> {
        let control = self.control?;
        let (dev, input, at) = self.got.clone()?;
        (now - at >= LEARN_SETTLE_SECS).then_some(Binding { device: dev, input, control, invert: false })
    }
}

/// Neue Zuordnung übernehmen: Was dieselbe Nachricht vom selben Gerät schon auslöste und was diese Funktion vom
/// selben Gerät schon belegte, fällt weg. Liefert die verdrängten Funktionen (für einen Hinweis).
pub fn bind(bindings: &mut Vec<Binding>, new: Binding) -> Vec<Control> {
    let mut lost = Vec::new();
    bindings.retain(|b| {
        let same_input = b.device == new.device && b.input == new.input;
        let same_control = b.device == new.device && b.control == new.control;
        if same_input && b.control != new.control {
            lost.push(b.control);
        }
        !(same_input || same_control)
    });
    bindings.push(new);
    lost
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(input: Input, control: Control) -> Binding {
        Binding { device: "Pult".into(), input, control, invert: false }
    }

    #[test]
    fn parses_channel_messages_only() {
        assert_eq!(parse(&[0x90, 11, 127]), Some(Msg { kind: Kind::Note, ch: 1, num: 11, value: 127 }));
        assert_eq!(parse(&[0x81, 11, 64]), Some(Msg { kind: Kind::Note, ch: 2, num: 11, value: 0 }));
        assert_eq!(parse(&[0xB0, 19, 87]), Some(Msg { kind: Kind::Cc, ch: 1, num: 19, value: 87 }));
        assert_eq!(parse(&[0xE3, 0x00, 0x40]), Some(Msg { kind: Kind::PitchBend, ch: 4, num: 0, value: 8192 }));
        assert_eq!(parse(&[0xF8]), None);
        assert_eq!(parse(&[0x90, 11]), None);
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn buttons_fire_once_per_press() {
        let mut midi = Midi::default();
        let note = binding(Input::Note { ch: 1, num: 11 }, Control::Play(0));
        let msg = |v| Msg { kind: Kind::Note, ch: 1, num: 11, value: v };
        assert_eq!(midi.resolve(&note, "Pult", msg(127)), Some(Event::Press));
        assert_eq!(midi.resolve(&note, "Pult", msg(0)), None);
        assert_eq!(midi.resolve(&note, "Anderes", msg(127)), None);
        let cc = binding(Input::Cc { ch: 1, num: 5 }, Control::Cue(0));
        let cc_msg = |v| Msg { kind: Kind::Cc, ch: 1, num: 5, value: v };
        assert_eq!(midi.resolve(&cc, "Pult", cc_msg(127)), Some(Event::Press));
        assert_eq!(midi.resolve(&cc, "Pult", cc_msg(127)), None, "gehalten");
        assert_eq!(midi.resolve(&cc, "Pult", cc_msg(0)), None);
        assert_eq!(midi.resolve(&cc, "Pult", cc_msg(100)), Some(Event::Press));
    }

    #[test]
    fn faders_in_7_and_14_bit() {
        let mut midi = Midi::default();
        let cc = binding(Input::Cc { ch: 1, num: 19 }, Control::Volume(0));
        assert_eq!(midi.resolve(&cc, "Pult", Msg { kind: Kind::Cc, ch: 1, num: 19, value: 127 }), Some(Event::Value(1.0)));
        let fine = binding(Input::Cc14 { ch: 1, num: 0 }, Control::Pitch(0));
        assert_eq!(midi.resolve(&fine, "Pult", Msg { kind: Kind::Cc, ch: 1, num: 32, value: 127 }), None);
        let Some(Event::Value(v)) = midi.resolve(&fine, "Pult", Msg { kind: Kind::Cc, ch: 1, num: 0, value: 127 }) else { panic!() };
        assert!((v - 1.0).abs() < 1e-6);
        let inverted = Binding { invert: true, ..binding(Input::PitchBend { ch: 1 }, Control::Pitch(1)) };
        assert_eq!(midi.resolve(&inverted, "Pult", Msg { kind: Kind::PitchBend, ch: 1, num: 0, value: 16383 }), Some(Event::Value(0.0)));
    }

    #[test]
    fn soft_takeover_waits_until_the_fader_crosses() {
        let mut midi = Midi::default();
        let c = Control::Volume(0);
        // FadeWerk steht auf 0,5, der Fader am Gerät unten: nichts springt
        assert!(!midi.take_over(c, 0.1, 0.5));
        assert!(!midi.take_over(c, 0.3, 0.5));
        // Über den Wert hinweg: übernimmt
        assert!(midi.take_over(c, 0.6, 0.5));
        // Nah dran: sofort
        assert!(midi.take_over(Control::Master, 0.52, 0.5));
    }

    #[test]
    fn learning_detects_14_bit_and_ignores_releases() {
        let mut l = Learn { control: Some(Control::Pitch(0)), got: None, in_ui: false };
        l.feed("Pult", Msg { kind: Kind::Cc, ch: 1, num: 0, value: 64 }, 1.0);
        l.feed("Pult", Msg { kind: Kind::Cc, ch: 1, num: 32, value: 3 }, 1.01);
        assert_eq!(l.done(1.2), None, "wartet noch auf den Feinanteil");
        assert_eq!(l.done(1.5).map(|b| b.input), Some(Input::Cc14 { ch: 1, num: 0 }));
        let mut l = Learn { control: Some(Control::Play(1)), got: None, in_ui: false };
        l.feed("Pult", Msg { kind: Kind::Note, ch: 2, num: 11, value: 0 }, 1.0);
        assert_eq!(l.got, None, "Loslassen zählt nicht");
        l.feed("Pult", Msg { kind: Kind::Note, ch: 2, num: 11, value: 127 }, 1.1);
        assert_eq!(l.done(2.0).map(|b| b.input), Some(Input::Note { ch: 2, num: 11 }));
    }

    #[test]
    fn binding_replaces_doubles() {
        let mut list = vec![binding(Input::Note { ch: 1, num: 11 }, Control::Play(0)), binding(Input::Note { ch: 1, num: 12 }, Control::Cue(0))];
        let lost = bind(&mut list, binding(Input::Note { ch: 1, num: 12 }, Control::Play(0)));
        assert_eq!(lost, vec![Control::Cue(0)]);
        assert_eq!(list, vec![binding(Input::Note { ch: 1, num: 12 }, Control::Play(0))]);
    }

    #[test]
    fn binding_file_format() {
        let b = binding(Input::Cc14 { ch: 1, num: 0 }, Control::Eq(1, 2));
        assert_eq!(serde_json::to_string(&b).unwrap(), r#"{"device":"Pult","in":"cc14","ch":1,"num":0,"do":{"eq":[1,2]}}"#);
        let back: Binding = serde_json::from_str(r#"{"device":"Pult","in":"note","ch":1,"num":11,"do":{"play":0}}"#).unwrap();
        assert_eq!(back.control, Control::Play(0));
    }
}
