// SPDX-License-Identifier: GPL-3.0-or-later
//! Oberfläche sperren, solange der DJ kurz weg ist: Musik, Auto-Fade und alles in `logic` laufen weiter, aber
//! Maus, Tastatur, Mausrad, abgelegte Dateien und das Schließen des Fensters wirken nicht mehr. Gesperrt zeigt ein
//! eigenes Display Schloss, GESPERRT, Uhr, Pegel und den Titel auf Sendung – und **nirgends, wie man entsperrt**.
//!
//! Ohne PIN wählt der DJ beim Sperren eines von vier Bildschirmvierteln (nur dort wird es erklärt); entsperrt wird
//! durch 3 s Gedrückthalten in diesem Viertel (Maus oder Touch). Eine kleine Zählung am Finger erscheint erst nach
//! einer halben Sekunde, ein falsches Viertel kostet wachsend Wartezeit. Mit PIN: Ziffernfeld oder Zifferntasten.
//! Die Sperre gilt nicht über einen Neustart hinaus; wer PIN oder Viertel vergessen hat, beendet die App hart und
//! startet sie neu (steht in den Einstellungen, nicht auf dem Sperrbildschirm).
//!
//! Die Eingaben filtert ein egui-Plugin (`LockFilter`, wie `ctrl_click`), bevor irgendein Bedienelement sie
//! sieht; so muss nichts einzeln gesperrt werden, und es greift auch im ausgeklinkten Ordnerfenster.

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, Color32, Event, Key, Pos2, Rect, Sense, Stroke, ViewportId, pos2, vec2};

use crate::i18n::{tr, trf};
use crate::meter::{Bands, Meter};
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
/// So lange im gewählten Viertel gedrückt halten (ohne PIN).
const HOLD_SECS: f64 = 3.0;
/// Erst nach so langem Drücken erscheint die Zählung am Finger.
const HOLD_SHOW_SECS: f64 = 0.5;
/// Wartezeit nach dem 1., 2. und jedem weiteren falschen Viertel.
const QUARTER_WAITS: [f64; 3] = [5.0, 15.0, 60.0];
/// Nach so vielen falschen PINs hintereinander …
const MAX_TRIES: u32 = 3;
/// … so lange warten, bis die nächste Eingabe geht.
const WAIT_SECS: f64 = 30.0;
/// So lange wackelt die Anzeige nach einem falschen PIN.
const SHAKE_SECS: f64 = 0.4;
/// Tasten des Ziffernfelds.
const KEY: egui::Vec2 = vec2(52.0, 40.0);
const KEY_GAP: f32 = 8.0;
/// Breite der Mittelachse des Sperrbildschirms (Karte „Jetzt läuft“, Pegel und Übergang).
const STAGE_W: f32 = 720.0;
/// So hoch ist der ganze Sperrbildschirm in voller Größe; niedrigere Fenster verkleinern alles gleichmäßig.
const STAGE_H: f32 = 640.0;
/// Ziffernfeld neben der Mittelachse.
const KEYPAD_W: f32 = 3.0 * KEY.x + 2.0 * KEY_GAP + 32.0;
const KEYPAD_H: f32 = 270.0;

/// Getippte Taste, vom Plugin eingesammelt.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PinKey {
    Digit(char),
    Back,
    Escape,
}

#[derive(Default)]
struct Shared {
    locked: bool,
    /// Fläche im Hauptfenster (letztes Bild), in der Mausklicks ankommen.
    pad: Option<Rect>,
    /// Tasten, die das Plugin abgefangen hat.
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
/// von vor der Sperre) und Drücken innerhalb von `pad`. Ziffern, Rücktaste und Esc landen in `typed`.
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
                } else if *key == Key::Escape {
                    typed.push(PinKey::Escape);
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

/// Viertel des Bildschirms (0 links oben, 1 rechts oben, 2 links unten, 3 rechts unten).
fn quarter_of(screen: Rect, p: Pos2) -> usize {
    usize::from(p.x >= screen.center().x) + 2 * usize::from(p.y >= screen.center().y)
}

fn quarter_rect(screen: Rect, q: usize) -> Rect {
    let c = screen.center();
    let x = if q.is_multiple_of(2) { screen.left()..=c.x } else { c.x..=screen.right() };
    let y = if q < 2 { screen.top()..=c.y } else { c.y..=screen.bottom() };
    Rect::from_x_y_ranges(x, y)
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Open,
    /// Ohne PIN: Viertel zum Entsperren wählen (noch nicht gesperrt, Eingaben aber schon gefiltert).
    Choosing,
    /// Gesperrt; `None` = mit PIN, sonst das gewählte Viertel.
    Locked(Option<usize>),
}

/// Was ein Bild des Sperrbildschirms ergeben hat.
#[derive(PartialEq, Debug)]
pub enum Outcome {
    None,
    /// Viertel gewählt, jetzt gesperrt.
    Locked,
    /// Auswahl abgebrochen (Esc, „Abbrechen“).
    Canceled,
    Unlocked,
}

/// Titel auf Sendung für den Sperrbildschirm.
pub struct NowPlaying {
    pub deck: usize,
    pub title: String,
    pub artist: String,
    pub position: f64,
    pub duration: f64,
    /// Titel endet bald: Restzeit blinkt rot (wie in der Statuszeile).
    pub warn: bool,
}

/// Was der Sperrbildschirm zeigt: Uhr und Datum, Titel auf Sendung, Pegel (A, B, MAIN) und den Übergang.
pub struct LockView<'a> {
    pub clock: String,
    pub date: String,
    pub meters: &'a [Meter; 3],
    pub bands: &'a Bands,
    pub now: Option<NowPlaying>,
    pub transition: &'a crate::mixer_ui::TransitionView,
}

/// Datum für den Sperrbildschirm, etwa „Sonntag, 8. November“.
pub fn date_text() -> String {
    let z = jiff::Zoned::now();
    let days = [tr("Montag"), tr("Dienstag"), tr("Mittwoch"), tr("Donnerstag"), tr("Freitag"), tr("Samstag"), tr("Sonntag")];
    let months = [
        tr("Januar"), tr("Februar"), tr("März"), tr("April"), tr("Mai"), tr("Juni"),
        tr("Juli"), tr("August"), tr("September"), tr("Oktober"), tr("November"), tr("Dezember"),
    ];
    let day = days[z.weekday().to_monday_zero_offset().clamp(0, 6) as usize];
    let month = months[(z.month() - 1).clamp(0, 11) as usize];
    trf("{}, {}. {}", &[&day, &z.day(), &month])
}

pub struct Lock {
    shared: Arc<Mutex<Shared>>,
    phase: Phase,
    /// Bisher getippte Ziffern.
    entered: String,
    /// Gedrückt seit (Zeit, Viertel), ohne PIN.
    hold: Option<(f64, usize)>,
    wrong: u32,
    wait_until: f64,
    shake_until: f64,
}

impl Lock {
    /// Sperre samt Plugin, das in `ctx` eingehängt wird.
    pub fn install(ctx: &egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        ctx.add_plugin(LockFilter(shared.clone()));
        Self { shared, phase: Phase::Open, entered: String::new(), hold: None, wrong: 0, wait_until: 0.0, shake_until: 0.0 }
    }

    /// Eingaben gesperrt (auch schon während der Wahl des Viertels).
    pub fn active(&self) -> bool {
        self.phase != Phase::Open
    }

    /// Sperren beginnen: mit PIN sofort gesperrt, sonst erst das Viertel wählen.
    pub fn begin(&mut self, with_pin: bool) {
        self.set_phase(if with_pin { Phase::Locked(None) } else { Phase::Choosing });
    }

    /// Entwicklerhilfe: gleich gesperrt mit diesem Viertel.
    pub fn lock_quarter(&mut self, q: usize) {
        self.set_phase(Phase::Locked(Some(q.min(3))));
    }

    fn set_phase(&mut self, phase: Phase) {
        let on = phase != Phase::Open;
        if let Ok(mut s) = self.shared.lock() {
            s.locked = on;
            s.pad = None;
            s.typed.clear();
        }
        if matches!(phase, Phase::Open | Phase::Choosing) {
            self.wrong = 0;
            self.wait_until = 0.0;
        }
        self.phase = phase;
        self.entered.clear();
        self.hold = None;
        set_quit_key(!on);
    }

    /// Zeichnet die Wahl des Viertels bzw. den Sperrbildschirm. `pin` = gespeicherter Hash (`None` = kein PIN).
    pub fn show(&mut self, ctx: &egui::Context, pin: Option<&str>, view: &LockView<'_>, now: f64) -> Outcome {
        let typed = self.shared.lock().map(|mut s| std::mem::take(&mut s.typed)).unwrap_or_default();
        let screen = ctx.content_rect();
        let mut outcome = Outcome::None;
        let mut pad = None;
        let area = egui::Area::new(egui::Id::new("sperre")).order(egui::Order::Foreground).fixed_pos(screen.min).constrain(false);
        let layer = area.show(ctx, |ui| {
            // Fängt Mauszeiger und Klicks ab, damit darunter nichts aufleuchtet oder Tooltips zeigt
            let bg = ui.allocate_rect(screen, Sense::click_and_drag());
            match self.phase {
                Phase::Open => {}
                Phase::Choosing => {
                    pad = Some(screen);
                    outcome = self.choose(ui, &bg, screen, &typed);
                }
                Phase::Locked(quarter) => {
                    let with_pin = quarter.is_none() && pin.is_some();
                    let stage = stage(ui, screen, view, with_pin, now);
                    if let (Some(hash), true) = (pin, with_pin) {
                        let keypad = Rect::from_min_size(pos2(stage.block.right() + 28.0, stage.block.center().y - KEYPAD_H / 2.0), vec2(KEYPAD_W, KEYPAD_H));
                        let shake = if now < self.shake_until {
                            ctx.request_repaint();
                            ((self.shake_until - now) * 60.0).sin() as f32 * 8.0 * ((self.shake_until - now) / SHAKE_SECS) as f32
                        } else {
                            0.0
                        };
                        let keypad = keypad.translate(vec2(shake, 0.0));
                        pad = Some(keypad);
                        if self.keypad(ui, keypad, hash, typed, now) {
                            outcome = Outcome::Unlocked;
                        }
                    } else {
                        pad = Some(screen);
                        if self.hold_to_unlock(ui, screen, quarter.unwrap_or(2), stage.hint, now) {
                            outcome = Outcome::Unlocked;
                        }
                    }
                }
            }
        });
        ctx.move_to_top(layer.response.layer_id);
        if let Ok(mut s) = self.shared.lock() {
            s.pad = pad;
        }
        match outcome {
            Outcome::Unlocked | Outcome::Canceled => self.set_phase(Phase::Open),
            _ => {}
        }
        // Die Uhr geht weiter, auch wenn sich sonst nichts bewegt
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        outcome
    }

    /// Wahl des Viertels: abgedunkelt, vier Felder mit Namen, in der Mitte die Erklärung und „Abbrechen“.
    fn choose(&mut self, ui: &mut egui::Ui, bg: &egui::Response, screen: Rect, typed: &[PinKey]) -> Outcome {
        if typed.contains(&PinKey::Escape) {
            return Outcome::Canceled;
        }
        let p = ui.painter().clone();
        p.rect_filled(screen, 0.0, theme::ground().gamma_multiply(0.82));
        let hover = ui.input(|i| i.pointer.hover_pos());
        let accent = theme::select_accent();
        const NAMES: [&str; 4] = ["LINKS OBEN", "RECHTS OBEN", "LINKS UNTEN", "RECHTS UNTEN"];
        // Erklärung in der Mitte; ein Klick dort wählt kein Viertel
        let info = Rect::from_center_size(screen.center(), vec2(420.0, 200.0));
        let over_info = hover.is_some_and(|h| info.contains(h));
        for (q, name) in NAMES.iter().enumerate() {
            let r = quarter_rect(screen, q).shrink(12.0);
            let hot = !over_info && hover.is_some_and(|h| r.contains(h));
            if hot {
                p.rect_filled(r, 10.0, accent.gamma_multiply(0.10));
            }
            p.rect_stroke(r, 10.0, Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { accent } else { theme::line() }), egui::StrokeKind::Inside);
            // Namen zur Außenecke hin, damit die Mitte frei bleibt
            let c = r.center() + (r.center() - screen.center()) * 0.25;
            p.text(c, Align2::CENTER_CENTER, tr(name), fonts::bold(22.0), if hot { accent } else { theme::dim() });
        }
        hw::panel(&p, info);
        paint_icon(&p, pos2(info.center().x, info.top() + 30.0), 26.0, accent, false);
        p.text(pos2(info.center().x, info.top() + 62.0), Align2::CENTER_CENTER, tr("FELD ZUM ENTSPERREN WÄHLEN"), fonts::bold(15.0), theme::ink());
        let text = tr("Klicke auf eines der vier Felder. Zum Entsperren hältst du später 3 Sekunden in diesem Feld gedrückt. Merke es dir – angezeigt wird es danach nirgends.");
        let g = p.layout(text.to_owned(), egui::FontId::proportional(12.5), theme::dim(), info.width() - 40.0);
        let gy = info.top() + 80.0;
        p.galley(pos2(info.center().x - g.size().x / 2.0, gy), g.clone(), theme::dim());
        let btn = Rect::from_center_size(pos2(info.center().x, info.bottom() - 26.0), vec2(120.0, 28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn));
        if hw::button(&mut child, btn.size(), tr("Abbrechen"), egui::FontId::proportional(13.0), hw::Look::Plain, false, true).clicked() {
            return Outcome::Canceled;
        }
        if bg.clicked()
            && let Some(at) = bg.interact_pointer_pos()
            && !info.contains(at)
        {
            self.set_phase(Phase::Locked(Some(quarter_of(screen, at))));
            return Outcome::Locked;
        }
        Outcome::None
    }

    /// Ohne PIN: 3 s im richtigen Viertel gedrückt halten. Liefert `true` beim Entsperren.
    fn hold_to_unlock(&mut self, ui: &mut egui::Ui, screen: Rect, quarter: usize, hint: Pos2, now: f64) -> bool {
        let (down, pos) = ui.input(|i| (i.pointer.primary_down(), i.pointer.interact_pos()));
        let waiting = now < self.wait_until;
        let p = ui.painter();
        if waiting {
            // Hinweis unter dem Display, ohne zu verraten, worauf es ankommt
            let secs = format!("{:.0}", (self.wait_until - now).ceil());
            p.text(hint, Align2::CENTER_CENTER, trf("Falsch. Erneut versuchen in {} s", &[&secs]), fonts::bold(14.0), theme::cue());
            self.hold = None;
            return false;
        }
        let (Some(pos), true) = (pos, down) else {
            self.hold = None;
            return false;
        };
        let q = quarter_of(screen, pos);
        let since = match self.hold {
            Some((t, hq)) if hq == q => t,
            _ => {
                self.hold = Some((now, q));
                now
            }
        };
        let held = now - since;
        ui.ctx().request_repaint();
        if held >= HOLD_SHOW_SECS {
            // Kleine Zählung am Finger: Ring füllt sich, darin 3-2-1
            let t = ((held - HOLD_SHOW_SECS) / (HOLD_SECS - HOLD_SHOW_SECS)).min(1.0) as f32;
            let c = pos + vec2(0.0, -44.0);
            let r = 16.0;
            p.circle_filled(c, r + 3.0, theme::well());
            let n = 40;
            let pts: Vec<Pos2> = (0..=(n as f32 * t) as usize)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 / n as f32 * std::f32::consts::TAU;
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            p.add(egui::Shape::line(pts, Stroke::new(3.0, theme::well_ink())));
            let left = (HOLD_SECS - held).ceil().max(1.0);
            p.text(c, Align2::CENTER_CENTER, format!("{left:.0}"), fonts::bold(14.0), theme::well_ink());
        }
        if held < HOLD_SECS {
            return false;
        }
        self.hold = None;
        if q == quarter {
            return true;
        }
        let wait = QUARTER_WAITS[(self.wrong as usize).min(QUARTER_WAITS.len() - 1)];
        self.wrong += 1;
        self.wait_until = now + wait;
        false
    }

    /// Mit PIN: Punkte der getippten Ziffern, Ziffernfeld. Liefert `true` beim Entsperren.
    fn keypad(&mut self, ui: &mut egui::Ui, rect: Rect, hash: &str, typed: Vec<PinKey>, now: f64) -> bool {
        hw::panel(ui.painter(), rect);
        let waiting = now < self.wait_until;
        let msg = if waiting {
            trf("Zu viele Versuche. Noch {} s warten.", &[&format!("{:.0}", (self.wait_until - now).ceil())])
        } else if self.wrong > 0 {
            tr("Falscher PIN").to_owned()
        } else {
            tr("PIN eingeben").to_owned()
        };
        let color = if self.wrong > 0 || waiting { theme::cue() } else { theme::dim() };
        let mut y = rect.top() + 18.0;
        ui.painter().text(pos2(rect.center().x, y), Align2::CENTER_CENTER, msg, egui::FontId::proportional(12.5), color);
        y += 14.0;
        let well = Rect::from_min_size(pos2(rect.center().x - (3.0 * KEY.x + 2.0 * KEY_GAP) / 2.0, y), vec2(3.0 * KEY.x + 2.0 * KEY_GAP, 26.0));
        hw::well(ui.painter(), well, 5.0);
        for i in 0..PIN_LEN {
            let x = well.left() + well.width() * (i as f32 + 0.5) / PIN_LEN as f32;
            let c = pos2(x, well.center().y);
            if i < self.entered.len() {
                ui.painter().circle_filled(c, 5.0, theme::well_ink());
            } else {
                ui.painter().circle_stroke(c, 5.0, Stroke::new(1.0, theme::well_ink().gamma_multiply(0.3)));
            }
        }
        y = well.bottom() + 12.0;
        let mut keys = typed;
        const LABELS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "", "0", "⌫"];
        for (i, label) in LABELS.iter().enumerate() {
            if label.is_empty() {
                continue;
            }
            let (row, col) = (i / 3, i % 3);
            let at = Rect::from_min_size(pos2(well.left() + col as f32 * (KEY.x + KEY_GAP), y + row as f32 * (KEY.y + KEY_GAP)), KEY);
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
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            return false;
        }
        for k in keys {
            match k {
                PinKey::Back => {
                    self.entered.pop();
                }
                PinKey::Digit(d) if self.entered.len() < PIN_LEN => self.entered.push(d),
                PinKey::Digit(_) | PinKey::Escape => {}
            }
            if self.entered.len() == PIN_LEN {
                let ok = pin_hash(&self.entered) == hash;
                self.entered.clear();
                if ok {
                    return true;
                }
                self.wrong += 1;
                self.shake_until = now + SHAKE_SECS;
                if self.wrong >= MAX_TRIES {
                    self.wait_until = now + WAIT_SECS;
                    self.wrong = 0;
                }
                break;
            }
        }
        false
    }
}

/// Lage der Teile, die `stage` gezeichnet hat: Karte und Gläser (daneben das Ziffernfeld) und die Stelle für Hinweise.
struct Stage {
    block: Rect,
    hint: Pos2,
}

/// Sperrbildschirm: oben die Marke, darunter auf einer Mittelachse GESPERRT, Uhr und Datum, die Karte „Jetzt läuft“
/// und nebeneinander Pegel und Übergang. Auf niedrigen Fenstern wird alles gleichmäßig kleiner (`k`).
fn stage(ui: &mut egui::Ui, screen: Rect, v: &LockView<'_>, with_pin: bool, now: f64) -> Stage {
    let p = ui.painter().clone();
    p.rect_filled(screen, 0.0, theme::ground());
    // Leichter Schein hinter der Uhr, damit die Mitte nicht flach wirkt
    if !theme::lite().flat {
        let glow = Rect::from_center_size(screen.center() - vec2(0.0, screen.height() * 0.18), vec2(screen.width() * 0.7, screen.height() * 0.5));
        for i in 0..6 {
            let t = i as f32 / 6.0;
            p.rect_filled(glow.shrink2(glow.size() * 0.5 * t), glow.height() * 0.5, theme::panel().gamma_multiply(0.05));
        }
    }
    let k = ((screen.height() - 24.0) / STAGE_H).clamp(0.6, 1.0);
    let pin_w = if with_pin { KEYPAD_W + 28.0 } else { 0.0 };
    let w = (STAGE_W * k).min(screen.width() - 64.0 - pin_w);
    let cx = screen.center().x - pin_w / 2.0;
    // Marke und Uhr auf der Bildschirmmitte; mit PIN steht die Gruppe aus Karte, Gläsern und Ziffernfeld als Ganzes mittig
    let mid = screen.center().x;
    let top = screen.center().y - STAGE_H * k / 2.0;

    // Marke oben, wie in der Statuszeile
    brand(&p, pos2(mid, top + 14.0 * k), k);

    // Abzeichen GESPERRT: Schloss und Schrift in einer Pille
    let accent = theme::select_accent();
    let label = p.layout_no_wrap(tr("GESPERRT").to_owned(), fonts::bold(13.0 * k), accent);
    let pill_w = label.size().x + 50.0 * k;
    let pill = Rect::from_center_size(pos2(mid, top + 62.0 * k), vec2(pill_w, 30.0 * k));
    p.rect_filled(pill, pill.height() / 2.0, accent.gamma_multiply(0.10));
    p.rect_stroke(pill, pill.height() / 2.0, Stroke::new(1.0, accent.gamma_multiply(0.6)), egui::StrokeKind::Inside);
    paint_icon(&p, pos2(pill.left() + 19.0 * k, pill.center().y), 15.0 * k, accent, false);
    p.galley(pos2(pill.left() + 33.0 * k, pill.center().y - label.size().y / 2.0), label, accent);

    // Uhr und Datum
    let clock_y = top + 150.0 * k;
    soft_lcd(&p, pos2(mid, clock_y), Align2::CENTER_CENTER, &v.clock, 104.0 * k, theme::ink());
    p.text(pos2(mid, clock_y + 76.0 * k), Align2::CENTER_CENTER, &v.date, egui::FontId::proportional(16.0 * k), theme::dim());

    // Karte „Jetzt läuft“
    let card = Rect::from_min_size(pos2(cx - w / 2.0, top + 262.0 * k), vec2(w, 130.0 * k));
    now_playing(&p, card, v.now.as_ref(), k, now);

    // Pegel und Übergang nebeneinander, gleich hoch
    let row_h = 128.0 * k;
    let gap = 14.0 * k;
    let meters = Rect::from_min_size(pos2(card.left(), card.bottom() + gap), vec2(140.0 * k, row_h));
    hw::well(&p, meters, 8.0);
    crate::mixer_ui::display_meters(&p, meters.shrink2(vec2(16.0 * k, 12.0 * k)), v.meters, v.bands);
    let trans = Rect::from_min_max(pos2(meters.right() + gap, meters.top()), pos2(card.right(), meters.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(trans), |ui| crate::mixer_ui::transition_bar(ui, v.transition, Some(row_h), false, true));

    let block = Rect::from_min_max(card.min, pos2(card.right(), meters.bottom()));
    Stage { block, hint: pos2(cx, block.bottom() + 34.0 * k) }
}

/// 7-Segment-Text wie `hw::lcd_text`, die unbeleuchteten Segmente aber nur ganz schwach (groß wirken sie sonst wie
/// eine zweite Zahl).
fn soft_lcd(p: &egui::Painter, pos: Pos2, align: Align2, text: &str, size: f32, color: Color32) {
    if !theme::lite().flat {
        let ghost: String = text.chars().map(|c| if c.is_ascii_digit() || c == '-' || c == ' ' { '8' } else { c }).collect();
        p.text(pos, align, ghost, fonts::lcd(size), color.gamma_multiply(0.035));
    }
    p.text(pos, align, text, fonts::lcd(size), color);
}

/// Marke wie in der Statuszeile: Platte in den Farben der Player und „FADEWERK DJ“, mittig auf `c`.
fn brand(p: &egui::Painter, c: Pos2, k: f32) {
    let font = fonts::bold(15.0 * k);
    let spacing = 3.0 * k;
    let glyphs: Vec<_> = "FADEWERK DJ".chars().map(|ch| p.layout_no_wrap(ch.to_string(), font.clone(), theme::dim())).collect();
    let name_w: f32 = glyphs.iter().map(|g| g.size().x).sum::<f32>() + spacing * (glyphs.len() as f32 - 1.0);
    let r = 9.0 * k;
    let total = 2.0 * r + 10.0 * k + name_w;
    let disc = pos2(c.x - total / 2.0 + r, c.y);
    let half = |from: f32, color: Color32| {
        let pts: Vec<Pos2> = (0..=16)
            .map(|i| {
                let a = from + i as f32 / 16.0 * std::f32::consts::PI;
                disc + vec2(a.cos(), a.sin()) * r
            })
            .collect();
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    };
    half(std::f32::consts::FRAC_PI_2, theme::deck_color(0));
    half(-std::f32::consts::FRAC_PI_2, theme::deck_color(1));
    p.circle_filled(disc, 2.8 * k, theme::ground());
    // Gesperrte Buchstaben mit Abstand, wie die Überschriften der Oberfläche
    let mut x = disc.x + r + 10.0 * k;
    for g in glyphs {
        let gw = g.size().x;
        p.galley(pos2(x, c.y - g.size().y / 2.0), g, theme::dim());
        x += gw + spacing;
    }
}

/// Glaskarte mit dem Titel auf Sendung: Player, großer Titel, Interpret, Restzeit und Fortschritt.
fn now_playing(p: &egui::Painter, card: Rect, np: Option<&NowPlaying>, k: f32, now: f64) {
    hw::well(p, card, 10.0);
    let ink = theme::well_ink();
    let inner = card.shrink2(vec2(24.0 * k, 18.0 * k));
    let Some(np) = np else {
        p.text(card.center(), Align2::CENTER_CENTER, tr("Nichts auf Sendung"), fonts::bold(18.0 * k), ink.gamma_multiply(0.45));
        return;
    };
    let color = theme::deck_color(np.deck);
    // Kopf: ON AIR · PLAYER A, rechts REST
    let head_y = inner.top() + 7.0 * k;
    let tag = p.layout_no_wrap("ON AIR".to_owned(), fonts::bold(10.5 * k), theme::cue());
    let tag_r = Rect::from_min_size(pos2(inner.left(), head_y - 9.0 * k), vec2(tag.size().x + 12.0 * k, 18.0 * k));
    p.rect_stroke(tag_r, 3.0, Stroke::new(1.0, theme::cue()), egui::StrokeKind::Inside);
    p.galley(tag_r.center() - tag.size() / 2.0, tag, theme::cue());
    p.text(pos2(tag_r.right() + 10.0 * k, head_y), Align2::LEFT_CENTER, trf("PLAYER {}", &[&theme::deck_letter(np.deck)]), fonts::bold(11.0 * k), color);
    p.text(pos2(inner.right(), head_y), Align2::RIGHT_CENTER, tr("REST"), fonts::bold(10.0 * k), ink.gamma_multiply(0.45));

    // Restzeit groß rechts, blinkt rot vor dem Titelende
    let left = (np.duration - np.position).max(0.0);
    let secs = left.ceil() as i64;
    let time = format!("-{}:{:02}", secs / 60, secs % 60);
    let time_color = if np.warn && now % 1.0 < 0.5 { theme::cue() } else { color };
    let time_size = 40.0 * k;
    let time_w = p.layout_no_wrap(time.clone(), fonts::lcd(time_size), color).size().x;
    soft_lcd(p, pos2(inner.right(), inner.top() + 44.0 * k), Align2::RIGHT_CENTER, &time, time_size, time_color);

    // Titel und Interpret links, gekürzt bis zur Restzeit
    let text_w = inner.width() - time_w - 24.0 * k;
    let title = elide(p, &np.title, fonts::bold(28.0 * k), text_w);
    p.text(pos2(inner.left(), inner.top() + 42.0 * k), Align2::LEFT_CENTER, title, fonts::bold(28.0 * k), ink);
    if !np.artist.is_empty() {
        let artist = elide(p, &np.artist, egui::FontId::proportional(16.0 * k), text_w);
        p.text(pos2(inner.left(), inner.top() + 72.0 * k), Align2::LEFT_CENTER, artist, egui::FontId::proportional(16.0 * k), ink.gamma_multiply(0.6));
    }

    // Fortschritt über die ganze Breite
    let bar = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 4.0 * k), pos2(inner.right(), inner.bottom()));
    p.rect_filled(bar, 2.0, ink.gamma_multiply(0.12));
    let t = if np.duration > 0.0 { (np.position / np.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
    p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * t, bar.height())), 2.0, color);
}

/// Text, mit „…“ gekürzt, bis er in `max` passt.
fn elide(p: &egui::Painter, text: &str, font: egui::FontId, max: f32) -> String {
    let fits = |t: &str| p.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE).size().x <= max;
    if fits(text) {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if fits(&t) {
            return t;
        }
    }
    String::new()
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
    fn quarters() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 80.0));
        assert_eq!(quarter_of(screen, pos2(10.0, 10.0)), 0);
        assert_eq!(quarter_of(screen, pos2(90.0, 10.0)), 1);
        assert_eq!(quarter_of(screen, pos2(10.0, 70.0)), 2);
        assert_eq!(quarter_of(screen, pos2(90.0, 70.0)), 3);
        for q in 0..4 {
            assert_eq!(quarter_of(screen, quarter_rect(screen, q).center()), q);
        }
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
