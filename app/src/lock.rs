// SPDX-License-Identifier: GPL-3.0-or-later
//! Oberfläche sperren, solange der DJ kurz weg ist: Musik, Auto-Fade und alles in `logic` laufen weiter, aber
//! Maus, Tastatur, Mausrad, abgelegte Dateien und das Schließen des Fensters wirken nicht mehr. Entsperrt wird
//! über die Plakette in der Mitte: ohne PIN durch 2 s Drücken auf das Schloss, mit PIN über das Ziffernfeld oder
//! die Zifferntasten. Die Sperre gilt nicht über einen Neustart hinaus; wer den PIN vergessen hat, beendet die
//! App hart und startet sie neu (steht in den Einstellungen, nicht auf der Plakette).
//!
//! Die Eingaben filtert ein egui-Plugin (`LockFilter`, wie `ctrl_click`), bevor irgendein Bedienelement sie
//! sieht; so muss nichts einzeln gesperrt werden, und es greift auch im ausgeklinkten Ordnerfenster.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, Color32, Event, Key, Pos2, Rect, Sense, Stroke, ViewportId, pos2, vec2};

use crate::i18n::{tr, trf};
use crate::{fonts, hw, theme};

/// Die App schließt sich selbst (Neustart nach Update/Zurücksetzen, Bildschirmfoto der Entwicklerhilfen): Das
/// Schließen gilt dann auch bei gesperrter Oberfläche.
static CLOSE_ALLOWED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Vor einem `ViewportCommand::Close`, das die App selbst schickt.
pub fn allow_close() {
    CLOSE_ALLOWED.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn close_allowed() -> bool {
    CLOSE_ALLOWED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Ziffern eines PIN.
pub const PIN_LEN: usize = 4;
/// So lange auf das Schloss drücken, wenn kein PIN festgelegt ist.
const HOLD_SECS: f64 = 2.0;
/// Nach so vielen falschen PINs hintereinander …
const MAX_TRIES: u32 = 3;
/// … so lange warten, bis die nächste Eingabe geht.
const WAIT_SECS: f64 = 30.0;
/// So lange wackelt die Anzeige nach einem falschen PIN.
const SHAKE_SECS: f64 = 0.4;
/// Tasten des Ziffernfelds.
const KEY: egui::Vec2 = vec2(52.0, 40.0);
const KEY_GAP: f32 = 8.0;

/// Getippte Taste (Ziffer oder Löschen), vom Plugin eingesammelt.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PinKey {
    Digit(char),
    Back,
}

#[derive(Default)]
struct Shared {
    locked: bool,
    /// Fläche der Plakette im Hauptfenster (letztes Bild): Nur dort kommen Mausklicks an.
    pad: Option<Rect>,
    /// Zifferntasten, die das Plugin abgefangen hat.
    typed: Vec<PinKey>,
}

/// egui-Plugin: verwirft bei gesperrter Oberfläche alle Eingaben außer Mausbewegung und Klicks auf die Plakette.
pub struct LockFilter(Arc<Mutex<Shared>>);

impl egui::Plugin for LockFilter {
    fn debug_name(&self) -> &'static str {
        "Oberfläche sperren"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        let Ok(mut s) = self.0.lock() else { return };
        if !s.locked {
            return;
        }
        let main = input.viewport_id == ViewportId::ROOT;
        let pad = if main { s.pad } else { None };
        let mut typed = Vec::new();
        filter(&mut input.events, pad, main.then_some(&mut typed));
        s.typed.extend(typed);
        input.dropped_files.clear();
        input.hovered_files.clear();
    }
}

/// Lässt nur durch, was die gesperrte Oberfläche braucht: Mausbewegung, Loslassen (harmlos, beendet ein Ziehen
/// von vor der Sperre) und Drücken innerhalb von `pad`. Zifferntasten und Rücktaste landen in `typed`.
fn filter(events: &mut Vec<Event>, pad: Option<Rect>, mut typed: Option<&mut Vec<PinKey>>) {
    events.retain(|e| match e {
        Event::PointerMoved(_) | Event::PointerGone | Event::WindowFocused(_) | Event::Screenshot { .. } => true,
        Event::PointerButton { pressed: false, .. } => true,
        Event::PointerButton { pos, pressed: true, button, .. } => *button == egui::PointerButton::Primary && pad.is_some_and(|r| r.contains(*pos)),
        Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
            if let Some(typed) = typed.as_deref_mut()
                && !modifiers.command
                && !modifiers.ctrl
                && !modifiers.alt
            {
                if let Some(d) = digit(*key) {
                    typed.push(PinKey::Digit(d));
                } else if *key == Key::Backspace {
                    typed.push(PinKey::Back);
                }
            }
            false
        }
        _ => false,
    });
}

fn digit(key: Key) -> Option<char> {
    const KEYS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    KEYS.iter().position(|k| *k == key).map(|i| char::from(b'0' + i as u8))
}

/// Gespeicherte Form eines PIN. Nur damit er nicht lesbar in den Einstellungen steht; bei vier Ziffern schützt
/// ein Hash ohnehin nicht vor jemandem, der an die Datei kommt.
pub fn pin_hash(pin: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in "FadeWerk DJ PIN ".bytes().chain(pin.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Gültiger PIN: genau `PIN_LEN` Ziffern.
pub fn pin_valid(pin: &str) -> bool {
    pin.len() == PIN_LEN && pin.bytes().all(|b| b.is_ascii_digit())
}

pub struct Lock {
    shared: Arc<Mutex<Shared>>,
    /// Bisher getippte Ziffern.
    entered: String,
    /// Seit wann das Schloss gedrückt wird (ohne PIN).
    hold_since: Option<f64>,
    wrong: u32,
    wait_until: f64,
    shake_until: f64,
}

impl Lock {
    /// Sperre samt Plugin, das in `ctx` eingehängt wird.
    pub fn install(ctx: &egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        ctx.add_plugin(LockFilter(shared.clone()));
        Self { shared, entered: String::new(), hold_since: None, wrong: 0, wait_until: 0.0, shake_until: 0.0 }
    }

    pub fn locked(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.locked)
    }

    pub fn set_locked(&mut self, on: bool) {
        if let Ok(mut s) = self.shared.lock() {
            s.locked = on;
            s.pad = None;
            s.typed.clear();
        }
        self.entered.clear();
        self.hold_since = None;
        self.wrong = 0;
        set_quit_key(!on);
    }

    /// Zeichnet die abgedunkelte Oberfläche mit der Plakette. `pin` = gespeicherter Hash (`None` = kein PIN).
    /// Liefert `true`, wenn entsperrt wurde.
    pub fn show(&mut self, ctx: &egui::Context, pin: Option<&str>, now: f64) -> bool {
        let typed = self.shared.lock().map(|mut s| std::mem::take(&mut s.typed)).unwrap_or_default();
        let screen = ctx.content_rect();
        let mut unlocked = false;
        let mut pad_rect = None;
        let area = egui::Area::new(egui::Id::new("sperre")).order(egui::Order::Foreground).fixed_pos(screen.min).constrain(false);
        let layer = area.show(ctx, |ui| {
            // Fängt Mauszeiger und Klicks ab, damit darunter nichts aufleuchtet oder Tooltips zeigt
            ui.allocate_rect(screen, Sense::click_and_drag());
            ui.painter().rect_filled(screen, 0.0, theme::ground().gamma_multiply(0.72));
            let waiting = now < self.wait_until;
            let size = if pin.is_some() { vec2(3.0 * KEY.x + 2.0 * KEY_GAP + 48.0, 360.0) } else { vec2(380.0, 230.0) };
            let shake = if now < self.shake_until {
                ctx.request_repaint();
                ((self.shake_until - now) * 60.0).sin() as f32 * 8.0 * ((self.shake_until - now) / SHAKE_SECS) as f32
            } else {
                0.0
            };
            let rect = Rect::from_center_size(screen.center() + vec2(shake, 0.0), size);
            pad_rect = Some(rect);
            hw::panel(ui.painter(), rect);
            let mut y = rect.top() + 34.0;
            paint_icon(ui.painter(), pos2(rect.center().x, y), 30.0, theme::select_accent(), false);
            y += 32.0;
            ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_CENTER, tr("GESPERRT"), fonts::bold(20.0), theme::ink());
            y += 24.0;
            let Some(hash) = pin else {
                // Ohne PIN: großes Schloss gedrückt halten, ein Ring füllt sich
                let hint = tr("Zum Entsperren 2 Sekunden auf das Schloss drücken");
                ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_CENTER, hint, egui::FontId::proportional(12.5), theme::dim());
                let c = pos2(rect.center().x, rect.bottom() - 62.0);
                let r = 36.0;
                let resp = ui.interact(Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)), egui::Id::new("sperre_schloss"), Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::PointingHand);
                let down = resp.is_pointer_button_down_on();
                let p = ui.painter();
                p.circle_filled(c, r, theme::well());
                p.circle_stroke(c, r, Stroke::new(1.0, theme::line()));
                let t = match (down, self.hold_since) {
                    (true, Some(t0)) => ((now - t0) / HOLD_SECS).min(1.0) as f32,
                    (true, None) => {
                        self.hold_since = Some(now);
                        0.0
                    }
                    (false, _) => {
                        self.hold_since = None;
                        0.0
                    }
                };
                if t > 0.0 {
                    let n = 48;
                    let pts: Vec<Pos2> = (0..=(n as f32 * t) as usize)
                        .map(|i| {
                            let a = -std::f32::consts::FRAC_PI_2 + i as f32 / n as f32 * std::f32::consts::TAU;
                            c + vec2(a.cos(), a.sin()) * (r - 3.0)
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, Stroke::new(4.0, theme::play())));
                }
                if down {
                    ctx.request_repaint();
                }
                paint_icon(p, c, 28.0, if down { theme::play() } else { theme::well_ink() }, t >= 1.0);
                if t >= 1.0 {
                    unlocked = true;
                }
                return;
            };
            // Mit PIN: Anzeige der getippten Ziffern als Punkte, darunter das Ziffernfeld
            let msg = if waiting {
                trf("Zu viele Versuche. Noch {} s warten.", &[&format!("{:.0}", (self.wait_until - now).ceil())])
            } else if self.wrong > 0 {
                tr("Falscher PIN").to_owned()
            } else {
                tr("PIN eingeben").to_owned()
            };
            let color = if self.wrong > 0 || waiting { theme::cue() } else { theme::dim() };
            ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_CENTER, msg, egui::FontId::proportional(12.5), color);
            if waiting {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            y += 22.0;
            let well = Rect::from_center_size(pos2(rect.center().x, y + 14.0), vec2(3.0 * KEY.x + 2.0 * KEY_GAP, 30.0));
            hw::well(ui.painter(), well, 5.0);
            for i in 0..PIN_LEN {
                let x = well.left() + well.width() * (i as f32 + 0.5) / PIN_LEN as f32;
                let c = pos2(x, well.center().y);
                if i < self.entered.len() {
                    ui.painter().circle_filled(c, 5.5, theme::well_ink());
                } else {
                    ui.painter().circle_stroke(c, 5.5, Stroke::new(1.0, theme::well_ink().gamma_multiply(0.3)));
                }
            }
            y = well.bottom() + 14.0;
            let mut keys = typed;
            let left = rect.center().x - (3.0 * KEY.x + 2.0 * KEY_GAP) / 2.0;
            const LABELS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "", "0", "⌫"];
            for (i, label) in LABELS.iter().enumerate() {
                if label.is_empty() {
                    continue;
                }
                let (row, col) = (i / 3, i % 3);
                let at = Rect::from_min_size(pos2(left + col as f32 * (KEY.x + KEY_GAP), y + row as f32 * (KEY.y + KEY_GAP)), KEY);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at).id_salt(("sperre_taste", i)));
                let resp = if *label == "⌫" {
                    hw::custom_button(&mut child, KEY, hw::Look::Plain, false, !waiting, paint_backspace).on_hover_text(tr("Löschen"))
                } else {
                    hw::button(&mut child, KEY, label, fonts::bold(18.0), hw::Look::Plain, false, !waiting)
                };
                if resp.clicked() {
                    keys.push(if *label == "⌫" { PinKey::Back } else { PinKey::Digit(label.chars().next().unwrap_or('0')) });
                }
            }
            if waiting {
                return;
            }
            for k in keys {
                match k {
                    PinKey::Back => {
                        self.entered.pop();
                    }
                    PinKey::Digit(d) if self.entered.len() < PIN_LEN => self.entered.push(d),
                    PinKey::Digit(_) => {}
                }
                if self.entered.len() == PIN_LEN {
                    if pin_hash(&self.entered) == hash {
                        unlocked = true;
                    } else {
                        self.wrong += 1;
                        self.shake_until = now + SHAKE_SECS;
                        if self.wrong >= MAX_TRIES {
                            self.wait_until = now + WAIT_SECS;
                            self.wrong = 0;
                        }
                    }
                    self.entered.clear();
                    break;
                }
            }
        });
        ctx.move_to_top(layer.response.layer_id);
        if let Ok(mut s) = self.shared.lock() {
            s.pad = pad_rect;
        }
        if unlocked {
            self.set_locked(false);
        }
        unlocked
    }
}

/// Abgedunkelt mit kleinem Schloss, für das ausgeklinkte Ordnerfenster (Eingaben verwirft dort das Plugin).
pub fn shade(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("sperre_fenster")).order(egui::Order::Foreground).fixed_pos(screen.min).constrain(false).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(screen, 0.0, theme::ground().gamma_multiply(0.72));
        paint_icon(p, screen.center() - vec2(0.0, 18.0), 26.0, theme::select_accent(), false);
        p.text(screen.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, tr("GESPERRT"), fonts::bold(16.0), theme::ink());
    });
}

/// Vorhängeschloss, `h` hoch, mittig auf `c`; `open` = Bügel auf einer Seite angehoben.
pub fn paint_icon(p: &egui::Painter, c: Pos2, h: f32, color: Color32, open: bool) {
    let body = Rect::from_center_size(c + vec2(0.0, h * 0.18), vec2(h * 0.78, h * 0.6));
    let w = (h * 0.09).max(1.4);
    let r = body.width() * 0.3;
    // Bügel: zwei Beine und ein Halbkreis; offen ist er angehoben und das rechte Bein steht frei
    let arc_y = body.top() - h * 0.12 - if open { h * 0.16 } else { 0.0 };
    let n = 14;
    let mut pts = vec![pos2(c.x - r, body.top() + w)];
    pts.extend((0..=n).map(|i| {
        let a = std::f32::consts::PI + i as f32 / n as f32 * std::f32::consts::PI;
        pos2(c.x + a.cos() * r, arc_y + a.sin() * r)
    }));
    pts.push(pos2(c.x + r, if open { arc_y + h * 0.06 } else { body.top() + w }));
    p.add(egui::Shape::line(pts, Stroke::new(w, color)));
    p.rect_filled(body, h * 0.08, color);
    // Schlüsselloch
    let hole = theme::panel();
    p.circle_filled(body.center() - vec2(0.0, h * 0.05), h * 0.07, hole);
    p.rect_filled(Rect::from_center_size(body.center() + vec2(0.0, h * 0.06), vec2(h * 0.06, h * 0.16)), 0.0, hole);
}

/// Rücktaste: Pfeil nach links mit Kreuz.
fn paint_backspace(p: &egui::Painter, r: Rect, color: Color32) {
    let c = r.center();
    let (w, h) = (22.0, 14.0);
    let st = Stroke::new(1.6, color);
    let l = c.x - w / 2.0;
    let pts = vec![pos2(l, c.y), pos2(l + 6.0, c.y - h / 2.0), pos2(c.x + w / 2.0, c.y - h / 2.0), pos2(c.x + w / 2.0, c.y + h / 2.0), pos2(l + 6.0, c.y + h / 2.0), pos2(l, c.y)];
    p.add(egui::Shape::line(pts, st));
    let x = c + vec2(3.0, 0.0);
    p.line_segment([x + vec2(-3.5, -3.5), x + vec2(3.5, 3.5)], st);
    p.line_segment([x + vec2(-3.5, 3.5), x + vec2(3.5, -3.5)], st);
}

/// macOS: ⌘Q beendet über das App-Menü, bevor egui die Taste sieht. Während der Sperre bekommt der Menüeintrag
/// „Beenden“ kein Tastenkürzel (dann landet ⌘Q beim Plugin und wird verworfen), danach wieder „q“.
#[cfg(target_os = "macos")]
fn set_quit_key(enabled: bool) {
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{class, msg_send, sel};
    // SAFETY: nur aus dem Hauptthread (Oberfläche); Nachrichten an AppKit-Objekte mit passenden Typen,
    // Null-Zeiger werden vor jeder Verwendung geprüft
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let menu: *mut AnyObject = msg_send![app, mainMenu];
        if menu.is_null() {
            return;
        }
        let key = if enabled { c"q" } else { c"" };
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: key.as_ptr()];
        let top: isize = msg_send![menu, numberOfItems];
        for i in 0..top {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: i];
            let sub: *mut AnyObject = if item.is_null() { std::ptr::null_mut() } else { msg_send![item, submenu] };
            if sub.is_null() {
                continue;
            }
            let n: isize = msg_send![sub, numberOfItems];
            for j in 0..n {
                let entry: *mut AnyObject = msg_send![sub, itemAtIndex: j];
                if entry.is_null() {
                    continue;
                }
                let action: Option<Sel> = msg_send![entry, action];
                if action == Some(sel!(terminate:)) {
                    let _: () = msg_send![entry, setKeyEquivalent: key];
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_quit_key(_enabled: bool) {}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    fn key(k: Key, modifiers: Modifiers) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn press(x: f32) -> Event {
        Event::PointerButton { pos: pos2(x, 10.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }
    }

    #[test]
    fn locked_input_only_reaches_the_pad() {
        let pad = Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 50.0));
        let mut events = vec![
            press(10.0),
            press(100.0),
            Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, 1.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move },
            key(Key::Num4, Modifiers::NONE),
            key(Key::Q, Modifiers::COMMAND),
            key(Key::Backspace, Modifiers::NONE),
            key(Key::Num2, Modifiers::COMMAND),
            Event::Text("4".into()),
            Event::PointerMoved(pos2(1.0, 1.0)),
        ];
        let mut typed = Vec::new();
        filter(&mut events, Some(pad), Some(&mut typed));
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], Event::PointerButton { pressed: true, .. }));
        assert!(matches!(events[1], Event::PointerMoved(_)));
        assert_eq!(typed, vec![PinKey::Digit('4'), PinKey::Back]);
    }

    #[test]
    fn other_windows_get_no_clicks() {
        let mut events = vec![press(10.0)];
        filter(&mut events, None, None);
        assert!(events.is_empty());
    }

    #[test]
    fn pin_rules() {
        assert!(pin_valid("0815"));
        assert!(!pin_valid("081"));
        assert!(!pin_valid("08a5"));
        assert_ne!(pin_hash("1234"), pin_hash("1235"));
        assert_eq!(pin_hash("1234").len(), 16);
    }
}
