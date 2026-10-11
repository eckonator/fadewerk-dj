// SPDX-License-Identifier: GPL-3.0-or-later
//! Mixer (Pegel, Kanalfader, Kopfhörer) und Auto-Fade-Bereich.

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_engine::{CURVE_KNOTS, Curve, HeadMode, Motor, eq};

use crate::meter::Meter;
use crate::i18n::{tr, trf};
use crate::theme;
use crate::{fonts, hw, settings};

/// Schritt der Pegelregler je Pfeiltaste.
const VOLUME_STEP: f32 = 0.02;
/// Bereich der Überblendzeit in Sekunden.
const FADE_MIN_SECS: f32 = 1.0;
const FADE_MAX_SECS: f32 = 20.0;

pub struct MixerModel<'a> {
    pub meters: &'a [Meter; 3],
    /// Frequenzbänder des Masters (zwischen L und R von MAIN, wenn Platz ist).
    pub bands: &'a crate::meter::Bands,
    pub volume: [f32; 2],
    pub head_mode: HeadMode,
    pub head_mix: f32,
    pub head_volume: f32,
    pub master_volume: f32,
    pub audio_ok: bool,
    /// Ein eigenes Kopfhörergerät ist offen: dann gibt es „Gerät“ statt „Ausg. 3/4“.
    pub head_device: bool,
    /// EQ je Player in dB und Kill-Tasten, Reihenfolge Bässe, Mitten, Höhen.
    pub eq_db: [[f32; 3]; 2],
    pub eq_kill: [[bool; 3]; 2],
    /// Beschriftung und Symbol der Effekt-Pads.
    pub fx_labels: &'a [String; fw_engine::FX_SLOTS],
    pub fx_icons: [crate::fx_icons::FxIcon; fw_engine::FX_SLOTS],
    /// Bitmaske der Pads, die gerade klingen.
    pub fx_playing: u32,
    pub fx_volume: f32,
}

pub enum MixerAction {
    /// Einstellungen auf der Seite „Über“ öffnen (Klick auf die Versionsnummer).
    OpenAbout,
    Volume(usize, f32),
    HeadMode(HeadMode),
    HeadMix(f32),
    HeadVolume(f32),
    MasterVolume(f32),
    Eq { deck: usize, band: usize, db: f32 },
    EqKill { deck: usize, band: usize, on: bool },
    /// Effekt-Pad auslösen: über Master oder nur auf dem Kopfhörer (vorhören).
    Fx { slot: usize, to_master: bool },
    FxVolume(f32),
}

/// Versionsnummer klein rechts neben der Überschrift, ohne sie aus der Mitte zu schieben.
/// Klick öffnet Einstellungen → Über.
fn version_tag(ui: &mut egui::Ui, title: Rect, out: &mut Vec<MixerAction>) {
    let galley = ui.painter().layout_no_wrap(format!("v{}", env!("CARGO_PKG_VERSION")), fonts::list(10.5), theme::faint());
    // Grundlinie wie die Überschrift: unten bündig, etwas angehoben
    let rect = Rect::from_min_size(pos2(title.right() + 6.0, title.bottom() - galley.size().y - 1.5), galley.size());
    let resp = ui.interact(rect.expand(3.0), ui.id().with("version"), Sense::click()).on_hover_text(tr("Über FadeWerk DJ")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if resp.hovered() { theme::dim() } else { theme::faint() };
    ui.painter().galley_with_override_text_color(rect.min, galley, color);
    if resp.clicked() {
        out.push(MixerAction::OpenAbout);
    }
}

fn small(text: &'static str) -> RichText {
    RichText::new(tr(text)).font(fonts::bold(10.5)).color(theme::dim()).extra_letter_spacing(0.8)
}

/// Vergrößerung für Tasten und Schrift, wenn die Mittelspalte breiter wird.
fn scale(ui: &egui::Ui) -> f32 {
    (ui.available_width() / 230.0).clamp(1.0, 1.3)
}

pub fn show_mixer(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let title = ui.vertical_centered(|ui| ui.label(RichText::new(tr("MIXER")).font(fonts::bold(15.0)).extra_letter_spacing(2.5))).inner;
    version_tag(ui, title.rect, out);
    eq_strip(ui, m, out);

    // Pegelbrücke: LED A | Fader A | MAIN | Fader B | LED B
    // Maße wachsen mit der Breite der Mittelspalte
    let w = ui.available_width();
    // Etwas flacher seit der EQ darüber sitzt, damit die Spalte auf kleinen Fenstern nicht überläuft
    let h = (w * 0.4).clamp(110.0, 180.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h + 18.0), Sense::hover());
    let led_w = (w * 0.055).clamp(14.0, 24.0);
    let bar = Rect::from_min_size(rect.min, vec2(w, h));
    // Außenkante des Schachts (2 pt um die LEDs) bündig mit dem äußeren EQ-Knopf darüber
    let inset = eq_outer_inset(w) + 2.0;
    let led_a = Rect::from_min_size(pos2(bar.left() + inset, bar.top()), vec2(led_w, h));
    let led_b = Rect::from_min_size(pos2(bar.right() - inset - led_w, bar.top()), vec2(led_w, h));
    let fader_w = (w * 0.16).clamp(40.0, 60.0);
    // Jeder Fader hängt mit festem Abstand an seiner LED-Kette; auf breiten Fenstern wächst nur der Raum zur Mitte
    let gap = 10.0;
    // MAIN füllt den Platz zwischen den Fadern (gleicher Abstand wie zur LED-Kette, 2 pt Schacht),
    // höchstens dreimal so breit wie die Grundbreite
    let main_base = led_w * 1.7;
    let between = (led_b.left() - led_a.right()) - 2.0 * (gap + fader_w) - 2.0 * (gap + 2.0);
    let main_w = between.clamp(main_base, main_base * 3.0);
    let led_m = Rect::from_center_size(pos2(bar.center().x, bar.center().y), vec2(main_w, h));
    let fa_center = (led_a.right() + gap + fader_w / 2.0).min((led_a.right() + led_m.left()) / 2.0);
    let fb_center = (led_b.left() - gap - fader_w / 2.0).max((led_m.right() + led_b.left()) / 2.0);
    let fad_a = Rect::from_center_size(pos2(fa_center, bar.center().y), vec2(fader_w, h));
    let fad_b = Rect::from_center_size(pos2(fb_center, bar.center().y), vec2(fader_w, h));

    let p = ui.painter().clone();
    // Meter sitzen in eingelassenen Schächten
    for r in [led_a, led_m, led_b] {
        hw::well(&p, r.expand(2.0), 3.0);
    }
    m.meters[0].paint(&p, led_a);
    // Sparsame Darstellung: MAIN nur mit L/R, die Bänder werden dann auch nicht berechnet
    m.meters[2].paint_with_bands(&p, led_m, (!theme::lite()).then_some(m.bands), main_base);
    m.meters[1].paint(&p, led_b);
    for (deck, r) in [(0usize, fad_a), (1, fad_b)] {
        let (resp, new) = hw::vertical_fader(ui, r, ("chfader", deck), m.volume[deck], settings::DEFAULT_CHANNEL_VOLUME, false);
        if let Some(v) = hw::nudge(ui, &resp, m.volume[deck], VOLUME_STEP, 0.0, 1.0).or(new) {
            out.push(MixerAction::Volume(deck, v));
        }
    }
    let ly = rect.bottom() - 7.0;
    p.text(pos2(fa_center, ly), Align2::CENTER_CENTER, "A", fonts::bold(13.0), theme::deck_color(0));
    p.text(pos2(bar.center().x, ly), Align2::CENTER_CENTER, "MAIN", fonts::bold(11.0), theme::dim());
    p.text(pos2(fb_center, ly), Align2::CENTER_CENTER, "B", fonts::bold(13.0), theme::deck_color(1));

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(small("MASTER"));
        let (resp, new) = hw::mini_fader(ui, "master", m.master_volume, settings::DEFAULT_MASTER_VOLUME, 0.0, 1.0, ui.available_width() - 4.0);
        if let Some(v) = hw::nudge(ui, &resp, m.master_volume, VOLUME_STEP, 0.0, 1.0).or(new) {
            out.push(MixerAction::MasterVolume(v));
        }
    });

    // Das aktive Gerät steht in den Einstellungen; hier nur ein Hinweis, wenn es fehlt
    if !m.audio_ok {
        ui.colored_label(theme::cue(), tr("Keine Audio-Ausgabe gefunden"));
    }
}

/// Maße der EQ-Reihe bei Breite `w`: Knopfdurchmesser, Breite einer Gruppe (A oder B), Platz je Knopf.
fn eq_geometry(w: f32) -> (f32, f32, f32) {
    let d = (w * 0.12).clamp(28.0, 40.0);
    let group_w = (w - (w * 0.1).max(24.0)) / 2.0;
    (d, group_w, group_w / 3.0)
}

/// Abstand der äußeren EQ-Knöpfe vom Rand der Mittelspalte.
fn eq_outer_inset(w: f32) -> f32 {
    let (d, _, slot) = eq_geometry(w);
    (slot - d) / 2.0
}

/// EQ-Reihe über der Pegelbrücke: links A, rechts B, je Höhen, Mitten, Bässe mit Kill-Taste darunter.
fn eq_strip(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let w = ui.available_width();
    let (d, group_w, slot) = eq_geometry(w);
    let (label_h, kill_h) = (12.0, 15.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, label_h + d + 3.0 + kill_h), Sense::hover());
    // 0 dB steht auf 12 Uhr: die linke Hälfte des Wegs senkt bis −26 dB, die rechte hebt bis +6 dB
    let to_norm = |db: f32| if db <= 0.0 { 0.5 * (1.0 - db / settings::EQ_MIN_DB) } else { 0.5 + 0.5 * db / settings::EQ_MAX_DB };
    let to_db = |n: f32| if n <= 0.5 { settings::EQ_MIN_DB * (1.0 - 2.0 * n) } else { settings::EQ_MAX_DB * (2.0 * n - 1.0) };
    let knob_y = rect.top() + label_h + d / 2.0;
    let p = ui.painter().clone();
    p.text(pos2(rect.center().x, knob_y), Align2::CENTER_CENTER, "EQ", fonts::bold(11.0), theme::dim());
    for deck in 0..2 {
        let gx = if deck == 0 { rect.left() } else { rect.right() - group_w };
        for (i, (band, name)) in [(eq::HIGH, "HI"), (eq::MID, "MID"), (eq::LOW, "LOW")].into_iter().enumerate() {
            let cx = gx + slot * (i as f32 + 0.5);
            p.text(pos2(cx, rect.top() + label_h / 2.0), Align2::CENTER_CENTER, tr(name), fonts::bold(9.0), theme::dim());
            let db = m.eq_db[deck][band];
            let killed = m.eq_kill[deck][band];
            let kr = Rect::from_center_size(pos2(cx, knob_y), vec2(d, d));
            // Ein abgeschaltetes Band zeigt seinen Bogen in Rot
            let color = if killed { theme::cue() } else { theme::deck_color(deck) };
            let (resp, new) = hw::knob(ui, kr, ("eq", deck, band), to_norm(db), to_norm(0.0), color);
            // Pfeiltasten in ganzen dB (die Skala ist links und rechts von 0 dB verschieden gestaucht)
            let keyed = hw::nudge(ui, &resp, db.round(), 1.0, settings::EQ_MIN_DB, settings::EQ_MAX_DB);
            resp.on_hover_text(format!("{} {:+.1} dB", tr(name), db));
            if let Some(db) = keyed {
                out.push(MixerAction::Eq { deck, band, db });
            } else if let Some(n) = new {
                // Nahe 0 dB einrasten, damit die Mitte leicht zu treffen ist
                let v = to_db(n);
                out.push(MixerAction::Eq { deck, band, db: if v.abs() < 0.6 { 0.0 } else { v } });
            }
            let br = Rect::from_center_size(pos2(cx, kr.bottom() + 3.0 + kill_h / 2.0), vec2((d + 6.0).min(slot - 3.0), kill_h));
            let b = ui.put(br, |ui: &mut egui::Ui| hw::button(ui, br.size(), "KILL", fonts::bold(8.5), hw::Look::Color(theme::cue()), killed, true));
            if b.on_hover_text(tr("Band ganz herausnehmen")).clicked() {
                out.push(MixerAction::EqKill { deck, band, on: !killed });
            }
        }
    }
    ui.add_space(2.0);
}

pub struct FadeModel<'a> {
    pub auto: bool,
    pub fade_secs: f32,
    pub curve: Curve,
    pub crossfader: f32,
    pub status: &'a str,
    pub motor: Motor,
    pub bass_swap: bool,
    pub auto_sync: bool,
}

pub enum FadeAction {
    Auto(bool),
    FadeSecs(f32),
    Curve(Curve),
    Crossfader(f32),
    FadeNow,
    Motor(Motor),
    BassSwap(bool),
    AutoSync(bool),
}

/// Feste Auto-Fade-Reihe: AUTO, FADE, BASS SWAP, AUTO SYNC, Statuszeile und Crossfader.
pub fn show_fade(ui: &mut egui::Ui, m: &FadeModel<'_>, out: &mut Vec<FadeAction>) {
    let k = scale(ui);
    // Die Tasten stehen oben, damit sie sichtbar bleiben, wenn der Ordnerbrowser die Mittelspalte kürzt;
    // Überblendzeit und Kurve werden im Betrieb seltener gebraucht
    ui.horizontal(|ui| {
        let avail = ui.available_width() - 18.0;
        let bw = avail * 0.3;
        let small_w = (avail - 2.0 * bw) / 2.0;
        if hw::button(ui, vec2(bw, 34.0 * k), "AUTO", fonts::bold(14.0 * k), hw::Look::Color(theme::play()), m.auto, true).clicked() {
            out.push(FadeAction::Auto(!m.auto));
        }
        if hw::button(ui, vec2(bw, 34.0 * k), tr("FADE"), fonts::bold(14.0 * k), hw::Look::Plain, false, true).clicked() {
            out.push(FadeAction::FadeNow);
        }
        let tip = tr("Beim Überblenden kommt der neue Titel ohne Bässe herein, in der Mitte tauschen die Bässe. Klingt sauberer als ein reiner Lautstärke-Fade.");
        let swap = hw::button(ui, vec2(small_w, 34.0 * k), "BASS\nSWAP", fonts::bold(10.0 * k), hw::Look::Color(theme::yellow()), m.bass_swap, true);
        if swap.on_hover_text(tip).clicked() {
            out.push(FadeAction::BassSwap(!m.bass_swap));
        }
        let tip = tr("Beim Titelwechsel bekommt der neue Titel das Tempo des laufenden. Danach läuft sein Pitch langsam auf 0 % zurück (0,1 % pro Sekunde).");
        let sync = hw::button(ui, vec2(small_w, 34.0 * k), "AUTO\nSYNC", fonts::bold(10.0 * k), hw::Look::Color(theme::yellow()), m.auto_sync, true);
        if sync.on_hover_text(tip).clicked() {
            out.push(FadeAction::AutoSync(!m.auto_sync));
        }
    });
    ui.vertical_centered(|ui| ui.label(RichText::new(m.status).size(12.0).color(theme::dim())));
    ui.add_space(4.0);
    // Crossfader über der Kurve, die weiße Linie darin zeigt seine Stellung
    // A und B direkt auf die Höhe der Fadermitte gezeichnet, damit beide gleich sitzen
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let label_w = 16.0;
    let rect = Rect::from_min_max(pos2(row.left() + label_w, row.top()), pos2(row.right() - label_w, row.bottom()));
    let (resp, new) = hw::horizontal_fader(ui, rect, "crossfader", m.crossfader, 0.0);
    let new = hw::nudge(ui, &resp, m.crossfader, 0.05, -1.0, 1.0).or(new);
    resp.on_hover_text(tr("Crossfader"));
    if let Some(x) = new {
        out.push(FadeAction::Crossfader(x));
    }
    let p = ui.painter();
    p.text(pos2(row.left() + label_w / 2.0 - 2.0, row.center().y), Align2::CENTER_CENTER, "A", fonts::bold(14.0), theme::deck_color(0));
    p.text(pos2(row.right() - label_w / 2.0 + 2.0, row.center().y), Align2::CENTER_CENTER, "B", fonts::bold(14.0), theme::deck_color(1));
}

/// Höhe der Kurvenbox bei Breite `cw`: flaches Breitformat, nach oben begrenzt.
fn curve_height(cw: f32) -> f32 {
    (cw * 0.33).clamp(84.0, 124.0)
}

/// Reiter KURVE: Blendkurve und Überblendzeit.
fn fade_settings(ui: &mut egui::Ui, m: &FadeModel<'_>, out: &mut Vec<FadeAction>) {
    let k = scale(ui);
    // Blendkurve: Linien lassen sich ziehen, Rechtsklick setzt auf „Weich“ zurück
    // Flaches Breitformat, nach oben begrenzt, damit die Box auf großen Monitoren nicht wuchtig wird
    let cw = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(cw, curve_height(cw)), Sense::click_and_drag());
    if let Some(c) = curve_editor(ui, &resp, rect, m.curve) {
        out.push(FadeAction::Curve(c));
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(tr("Linien ziehen formt die Blendkurve, Rechtsklick setzt sie zurück"));
    if resp.secondary_clicked() {
        out.push(FadeAction::Curve(Curve::Soft));
    }
    let p = ui.painter();
    hw::well(p, rect, 4.0);
    for (deck, color) in [(0usize, theme::deck_color(0)), (1, theme::deck_color(1))] {
        let pts: Vec<_> = (0..=48)
            .map(|i| {
                let x = i as f32 / 48.0 * 2.0 - 1.0;
                let (a, b) = m.curve.gains(x);
                let g = if deck == 0 { a } else { b };
                pos2(rect.left() + i as f32 / 48.0 * rect.width(), rect.bottom() - 5.0 - g * (rect.height() - 10.0))
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    }
    // Stützpunkte als kleine Griffe, bei eigener Kurve deutlicher
    let handle = if matches!(m.curve, Curve::Custom(_)) { 3.5 } else { 2.5 };
    for (k, g) in CURVE_KNOTS.iter().zip(m.curve.points()) {
        // B ist das Spiegelbild: gleicher Pegel an der gespiegelten Stelle
        let y = rect.bottom() - 5.0 - g * (rect.height() - 10.0);
        p.circle_filled(pos2(rect.left() + k * rect.width(), y), handle, theme::deck_color(0));
        p.circle_filled(pos2(rect.left() + (1.0 - k) * rect.width(), y), handle, theme::deck_color(1));
    }
    let xm = rect.left() + (m.crossfader + 1.0) / 2.0 * rect.width();
    p.line_segment([pos2(xm, rect.top()), pos2(xm, rect.bottom())], Stroke::new(1.5, Color32::WHITE));

    // Kurventasten unten mittig in der Box: dort verläuft bei keiner Kurve eine Linie
    let bw = rect.width() * 0.25;
    let bh = 20.0 * k;
    let left = rect.center().x - 1.5 * bw - 4.0;
    for (i, (c, label)) in [(Curve::Soft, "Weich"), (Curve::Linear, "Linear"), (Curve::Cut, "Hart")].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(left + i as f32 * (bw + 4.0), rect.bottom() - bh - 5.0), vec2(bw, bh));
        let on = m.curve == c;
        if ui.put(r, |ui: &mut egui::Ui| hw::button(ui, r.size(), tr(label), fonts::bold(10.5 * k), hw::Look::Color(theme::yellow()), on, true)).clicked() {
            out.push(FadeAction::Curve(c));
        }
    }

    // Überblendzeit unter der Kurve als kleines LCD und Regler
    ui.add_space(6.0);
    if let Some(v) = secs_row(ui, m.fade_secs, &FADE_TIME) {
        out.push(FadeAction::FadeSecs(v));
    }
}

/// Eine Zeit in Sekunden als LCD (anklickbar zum Eintippen), „SEK.“ und Hardware-Fader.
struct SecsField {
    id: &'static str,
    min: f32,
    max: f32,
    default: f32,
    /// Raster beim Ziehen und für Pfeiltasten/Mausrad.
    step: f32,
    /// Raster beim Eintippen.
    typed_step: f32,
    decimals: usize,
    fader_tip: &'static str,
}

const FADE_TIME: SecsField = SecsField {
    id: "fadesecs",
    min: FADE_MIN_SECS,
    max: FADE_MAX_SECS,
    default: settings::DEFAULT_FADE_SECS,
    step: 0.5,
    typed_step: 0.1,
    decimals: 1,
    fader_tip: "Überblendzeit",
};

/// Zeile aus LCD, „SEK.“ und Fader. Liefert den neuen Wert, wenn er geändert wurde.
fn secs_row(ui: &mut egui::Ui, value: f32, f: &SecsField) -> Option<f32> {
    ui.horizontal(|ui| {
        let scale = (ui.available_width() / 230.0).clamp(1.0, 1.5);
        let (lcd, lcd_resp) = ui.allocate_exact_size(vec2(52.0, 22.0) * scale, Sense::click());
        hw::well(ui.painter(), lcd, 4.0);
        let typed = secs_lcd(ui, lcd, lcd_resp, value, 13.0 * scale, f);
        ui.label(small("SEK."));
        let snap = |v: f32| ((v / f.step).round() * f.step).clamp(f.min, f.max);
        let (resp, new) = hw::mini_fader(ui, f.id, value, f.default, f.min, f.max, ui.available_width() - 4.0);
        let keyed = hw::nudge(ui, &resp, snap(value), f.step, f.min, f.max);
        resp.on_hover_text(tr(f.fader_tip));
        typed.or(keyed).or(new.map(snap))
    })
    .inner
}

/// Reiter MOTOR: Anlauf und Auslauf der Plattenteller.
fn motor_settings(ui: &mut egui::Ui, m: &FadeModel<'_>, out: &mut Vec<FadeAction>) {
    const SPIN_UP: SecsField = SecsField {
        id: "spinup",
        min: 0.1,
        max: 2.0,
        default: 0.5,
        step: 0.05,
        typed_step: 0.05,
        decimals: 2,
        fader_tip: "Anlaufzeit bei Play",
    };
    const SPIN_DOWN: SecsField = SecsField {
        id: "spindown",
        min: 0.2,
        max: 4.0,
        default: 1.6,
        step: 0.1,
        typed_step: 0.1,
        decimals: 1,
        fader_tip: "Auslaufzeit bei Pause und Stopp",
    };
    debug_assert_eq!((SPIN_UP.default, SPIN_DOWN.default), (Motor::default().spin_up_secs, Motor::default().spin_down_secs));
    ui.label(small("ANLAUF"));
    if let Some(v) = secs_row(ui, m.motor.spin_up_secs, &SPIN_UP) {
        out.push(FadeAction::Motor(Motor { spin_up_secs: v, ..m.motor }));
    }
    ui.add_space(2.0);
    ui.label(small("AUSLAUF"));
    if let Some(v) = secs_row(ui, m.motor.spin_down_secs, &SPIN_DOWN) {
        out.push(FadeAction::Motor(Motor { spin_down_secs: v, ..m.motor }));
    }
    ui.add_space(2.0);
    ui.label(RichText::new(tr("Wie schnell die Teller bei Play anlaufen und bei Pause oder Stopp auslaufen. Im Kopfhörer starten und halten sie sofort.")).size(11.5).color(theme::faint()));
}

/// Reiter PADS: acht Effekt-Tasten in zwei Reihen und die Lautstärke der Effekte.
fn pads(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let k = scale(ui);
    let gap = ui.spacing().item_spacing.x;
    let cols = 4;
    let bw = (ui.available_width() - (cols as f32 - 1.0) * gap) / cols as f32;
    for row in 0..2 {
        ui.horizontal(|ui| {
            for col in 0..cols {
                let slot = row * cols + col;
                let lit = m.fx_playing & (1 << slot) != 0;
                let label = &m.fx_labels[slot];
                let icon = m.fx_icons[slot];
                // Symbol oben, Name klein darunter
                let b = hw::custom_button(ui, vec2(bw, 46.0 * k), hw::Look::Color(theme::select_accent()), lit, true, |p, r, color| {
                    let icon_rect = Rect::from_center_size(pos2(r.center().x, r.top() + r.height() * 0.38), vec2(r.height() * 0.42, r.height() * 0.42));
                    icon.paint(p, icon_rect, color);
                    let font = fonts::bold(if label.chars().count() > 8 { 8.0 } else { 9.0 } * k);
                    let galley = p.layout(label.clone(), font, color, r.width() - 6.0);
                    let pos = pos2(r.center().x - galley.size().x / 2.0, r.bottom() - galley.size().y - 4.0 * k);
                    p.with_clip_rect(r.shrink(2.0)).galley(pos, galley, color);
                });
                let b = b.on_hover_text(trf("Taste {} · Rechtsklick: im Kopfhörer vorhören", &[&(slot + 1)]));
                if b.clicked() {
                    out.push(MixerAction::Fx { slot, to_master: true });
                }
                if b.secondary_clicked() {
                    out.push(MixerAction::Fx { slot, to_master: false });
                }
            }
        });
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(small("FX"));
        let (resp, new) = hw::mini_fader(ui, "fxvol", m.fx_volume, settings::DEFAULT_FX_VOLUME, 0.0, 1.0, ui.available_width() - 4.0);
        let new = hw::nudge(ui, &resp, m.fx_volume, VOLUME_STEP, 0.0, 1.0).or(new);
        resp.on_hover_text(tr("Lautstärke der Effekte"));
        if let Some(v) = new {
            out.push(MixerAction::FxVolume(v));
        }
    });
}

/// Reiter KOPFHÖRER: Mischung, Pegel und Betriebsart.
fn headphones(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let k = scale(ui);
    ui.horizontal(|ui| {
        ui.label(small("CUE"));
        let (resp, new) = hw::mini_fader(ui, "headmix", m.head_mix, settings::DEFAULT_HEAD_MIX, 0.0, 1.0, ui.available_width() - 34.0);
        let new = hw::nudge(ui, &resp, m.head_mix, VOLUME_STEP, 0.0, 1.0).or(new);
        resp.on_hover_text(tr("Links (CUE): nur was du vorhörst · rechts (MST): die Musik der Lautsprecher"));
        if let Some(v) = new {
            out.push(MixerAction::HeadMix(v));
        }
        ui.label(small("MST"));
    });
    ui.horizontal(|ui| {
        ui.label(small("PEGEL"));
        let (resp, new) = hw::mini_fader(ui, "headvol", m.head_volume, settings::DEFAULT_HEAD_VOLUME, 0.0, 1.0, ui.available_width() - 4.0);
        if let Some(v) = hw::nudge(ui, &resp, m.head_volume, VOLUME_STEP, 0.0, 1.0).or(new) {
            out.push(MixerAction::HeadVolume(v));
        }
    });
    // Betriebsart unter den Reglern
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let bw = (ui.available_width() - 12.0) / 3.0;
        let third = if m.head_device {
            (HeadMode::Device, "Gerät", "Eigenes Kopfhörergerät aus den Einstellungen")
        } else {
            (HeadMode::Outputs34, "Ausg. 3/4", "Mehrkanal-Interface: Kopfhörer auf Ausgang 3/4")
        };
        for (mode, label, tip) in [
            (HeadMode::Off, "Aus", "Kein Kopfhörersignal"),
            (HeadMode::Split, "Split L/R", "Eine Soundkarte: links Master, rechts Kopfhörer"),
            third,
        ] {
            let on = m.head_mode == mode;
            if hw::button(ui, vec2(bw, 26.0 * k), tr(label), fonts::bold(11.5 * k), hw::Look::Color(theme::yellow()), on, true).on_hover_text(tr(tip)).clicked() {
                out.push(MixerAction::HeadMode(mode));
            }
        }
    });
}

/// Bereiche unter dem Crossfader, als Reiter: immer höchstens einer offen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CenterTab {
    Pads,
    Headphones,
    Fade,
    Motor,
}

impl CenterTab {
    pub const ALL: [Self; 4] = [Self::Pads, Self::Headphones, Self::Fade, Self::Motor];

    /// Schlüssel für die Einstellungen.
    pub fn key(self) -> &'static str {
        match self {
            Self::Pads => "pads",
            Self::Headphones => "kopfhoerer",
            Self::Fade => "fade",
            Self::Motor => "motor",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == key)
    }

    /// Beschriftung (deutscher Schlüssel für `tr`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Pads => "PADS",
            // Kurz, damit alle Reiter gleich breit sein können (wie „PHONES“ an Mischpulten)
            Self::Headphones => "HÖRER",
            // Nicht „FADE“: so heißt schon die große Taste darüber (sofort überblenden)
            Self::Fade => "KURVE",
            Self::Motor => "MOTOR",
        }
    }
}

/// Reiterleiste und der offene Bereich. Ein Klick auf den offenen Reiter klappt ihn zu. Die Fläche ist
/// immer so hoch wie der größte Bereich, damit beim Umschalten nichts springt. Liefert den neu gewählten
/// Reiter, wenn sich etwas geändert hat.
pub fn show_tabs(
    ui: &mut egui::Ui,
    open: Option<CenterTab>,
    mm: &MixerModel<'_>,
    mixer_out: &mut Vec<MixerAction>,
    fm: &FadeModel<'_>,
    fade_out: &mut Vec<FadeAction>,
) -> Option<Option<CenterTab>> {
    let k = scale(ui);
    let mut changed = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let n = CenterTab::ALL.len() as f32;
        let bw = (ui.available_width() - (n - 1.0) * ui.spacing().item_spacing.x) / n;
        for tab in CenterTab::ALL {
            let on = open == Some(tab);
            let label = tr(tab.label());
            // Zu lange Beschriftungen werden kleiner, damit sie in schmale Reiter passen
            let size = (10.5 * k).min((bw - 8.0) / (label.chars().count() as f32 * 0.7));
            let b = hw::button(ui, vec2(bw, 22.0 * k), label, fonts::bold(size), hw::Look::Color(theme::ink()), on, true);
            let tip = match (tab, on) {
                (CenterTab::Headphones, true) => tr("Kopfhörer · zuklappen"),
                (CenterTab::Headphones, false) => tr("Kopfhörer · aufklappen"),
                (_, true) => tr("Zuklappen"),
                (_, false) => tr("Aufklappen"),
            };
            if b.on_hover_text(tip).clicked() {
                changed = Some(if on { None } else { Some(tab) });
            }
        }
    });
    let Some(tab) = changed.unwrap_or(open) else { return changed };
    ui.add_space(6.0);
    // Feste Höhe: so hoch wie KURVE (Kurve plus Zeile für die Überblendzeit) oder der größte schon gesehene Bereich
    let w = ui.available_width();
    let h_id = ui.id().with("center-tab-h");
    let seen: f32 = ui.data(|d| d.get_temp(h_id)).unwrap_or(0.0);
    let h = (curve_height(w) + 6.0 + 26.0 * (w / 230.0).clamp(1.0, 1.5)).max(seen);
    let top = ui.cursor().top();
    match tab {
        CenterTab::Pads => pads(ui, mm, mixer_out),
        CenterTab::Headphones => headphones(ui, mm, mixer_out),
        CenterTab::Fade => fade_settings(ui, fm, fade_out),
        CenterTab::Motor => motor_settings(ui, fm, fade_out),
    }
    let used = ui.cursor().top() - top;
    if used > seen {
        ui.data_mut(|d| d.insert_temp(h_id, used));
    }
    if used < h {
        ui.add_space(h - used);
    }
    changed
}

/// Ziehen an einer Linie der Blendkurve. Gegriffen wird beim Drücken der nächstgelegene Stützpunkt
/// der näheren Linie; B ist das Spiegelbild von A, darum wirkt Ziehen an B auf den gespiegelten Punkt.
fn curve_editor(ui: &egui::Ui, resp: &egui::Response, rect: Rect, curve: Curve) -> Option<Curve> {
    let id = resp.id.with("griff");
    let pos = resp.interact_pointer_pos()?;
    let to_p = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    let to_g = |y: f32| ((rect.bottom() - 5.0 - y) / (rect.height() - 10.0)).clamp(0.0, 1.0);
    if resp.drag_started() {
        let p = to_p(pos.x);
        let (ga, gb) = curve.gains(p * 2.0 - 1.0);
        let g = to_g(pos.y);
        // Welche Linie ist näher? Auf B gegriffen heißt: Punkt an der gespiegelten Stelle von A.
        // Weil B(p) = A(1 − p), ist die Höhe dort genau der Stützpunkt von A.
        let pa = if (g - ga).abs() <= (g - gb).abs() { p } else { 1.0 - p };
        let idx = CURVE_KNOTS.iter().enumerate().min_by(|a, b| (a.1 - pa).abs().total_cmp(&(b.1 - pa).abs())).map(|(i, _)| i).unwrap_or(0);
        ui.data_mut(|d| d.insert_temp(id, idx));
    }
    if !resp.dragged() {
        return None;
    }
    let idx: usize = ui.data(|d| d.get_temp(id))?;
    let mut pts = curve.points();
    let target = to_g(pos.y);
    let upper = if idx == 0 { 1.0 } else { pts[idx - 1] };
    let lower = if idx == 3 { 0.0 } else { pts[idx + 1] };
    pts[idx] = target.clamp(lower, upper);
    Some(Curve::Custom(pts))
}

/// LCD einer Zeit. Klick öffnet die Eingabe direkt im Display: Enter oder Wegklicken übernimmt,
/// Esc bricht ab. Komma oder Punkt, gerundet auf `typed_step` und auf den Reglerbereich begrenzt.
fn secs_lcd(ui: &mut egui::Ui, lcd: Rect, resp: egui::Response, secs: f32, size: f32, f: &SecsField) -> Option<f32> {
    let edit_id = ui.id().with((f.id, "edit"));
    let mut text: Option<String> = ui.data(|d| d.get_temp(edit_id));
    if text.is_none() && resp.clicked() {
        text = Some(format!("{secs:.prec$}", prec = f.decimals));
    }
    let Some(mut buf) = text else {
        hw::lcd_text(ui.painter(), lcd.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, &format!("{secs:4.prec$}", prec = f.decimals), size, theme::well_ink());
        resp.on_hover_cursor(egui::CursorIcon::Text).on_hover_text(tr("Klicken, um die Zeit einzugeben"));
        return None;
    };
    let field = egui::TextEdit::singleline(&mut buf)
        .id(edit_id.with("field"))
        .font(fonts::lcd(size))
        .text_color(theme::well_ink())
        .horizontal_align(egui::Align::RIGHT)
        .vertical_align(egui::Align::Center)
        .frame(egui::Frame::NONE)
        .margin(vec2(6.0, 0.0))
        .char_limit(5);
    let r = ui.put(lcd, field);
    if !r.has_focus() && !r.lost_focus() {
        // Erstes Bild: Fokus holen und alles markieren, damit man gleich überschreiben kann
        r.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(buf.chars().count()),
            )));
            state.store(ui.ctx(), r.id);
        }
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(edit_id));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return None;
        }
        let v: f32 = buf.trim().trim_end_matches(['s', 'S']).trim().replace(',', ".").parse().ok()?;
        return v.is_finite().then(|| ((v / f.typed_step).round() * f.typed_step).clamp(f.min, f.max));
    }
    ui.data_mut(|d| d.insert_temp(edit_id, buf));
    None
}
