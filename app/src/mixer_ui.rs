// SPDX-License-Identifier: GPL-3.0-or-later
//! Mixer (Pegel, Kanalfader, Kopfhörer) und Auto-Fade-Bereich.

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use fw_engine::{CURVE_KNOTS, Curve, HeadMode, Motor, eq};

use crate::meter::Meter;
use crate::i18n::tr;
use crate::theme;
use crate::{fonts, hw, settings};

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
}

pub enum MixerAction {
    Volume(usize, f32),
    HeadMode(HeadMode),
    HeadMix(f32),
    HeadVolume(f32),
    MasterVolume(f32),
    Eq { deck: usize, band: usize, db: f32 },
    EqKill { deck: usize, band: usize, on: bool },
}

pub fn section_title(ui: &mut egui::Ui, text: &'static str) {
    ui.vertical_centered(|ui| ui.label(RichText::new(tr(text)).font(fonts::bold(15.0)).extra_letter_spacing(2.5)));
}

fn small(text: &'static str) -> RichText {
    RichText::new(tr(text)).font(fonts::bold(10.5)).color(theme::dim()).extra_letter_spacing(0.8)
}

/// Vergrößerung für Tasten und Schrift, wenn die Mittelspalte breiter wird.
fn scale(ui: &egui::Ui) -> f32 {
    (ui.available_width() / 230.0).clamp(1.0, 1.3)
}

pub fn show_mixer(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let k = scale(ui);
    section_title(ui, "MIXER");
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
        if let (_, Some(v)) = hw::vertical_fader(ui, r, ("chfader", deck), m.volume[deck], settings::DEFAULT_CHANNEL_VOLUME, false) {
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
        if let (_, Some(v)) = hw::mini_fader(ui, "master", m.master_volume, settings::DEFAULT_MASTER_VOLUME, 0.0, 1.0, ui.available_width() - 4.0) {
            out.push(MixerAction::MasterVolume(v));
        }
    });

    // Kopfhörer-Block (Regler und Betriebsart) durch eine Linie vom Master getrennt
    ui.separator();
    ui.label(small("KOPFHÖRER"));
    ui.horizontal(|ui| {
        ui.label(small("CUE"));
        let (resp, new) = hw::mini_fader(ui, "headmix", m.head_mix, settings::DEFAULT_HEAD_MIX, 0.0, 1.0, ui.available_width() - 34.0);
        resp.on_hover_text(tr("Mischung Vorhören ↔ Master"));
        if let Some(v) = new {
            out.push(MixerAction::HeadMix(v));
        }
        ui.label(small("MST"));
    });
    ui.horizontal(|ui| {
        ui.label(small("PEGEL"));
        if let (_, Some(v)) = hw::mini_fader(ui, "headvol", m.head_volume, settings::DEFAULT_HEAD_VOLUME, 0.0, 1.0, ui.available_width() - 4.0) {
            out.push(MixerAction::HeadVolume(v));
        }
    });
    // Betriebsart unter den Reglern, damit sie mit den Auto-Fade-Tasten darunter eine Tastengruppe bildet
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
            resp.on_hover_text(format!("{} {:+.1} dB", tr(name), db));
            if let Some(n) = new {
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
    resp.on_hover_text(tr("Crossfader"));
    if let Some(x) = new {
        out.push(FadeAction::Crossfader(x));
    }
    let p = ui.painter();
    p.text(pos2(row.left() + label_w / 2.0 - 2.0, row.center().y), Align2::CENTER_CENTER, "A", fonts::bold(14.0), theme::deck_color(0));
    p.text(pos2(row.right() - label_w / 2.0 + 2.0, row.center().y), Align2::CENTER_CENTER, "B", fonts::bold(14.0), theme::deck_color(1));


    ui.add_space(4.0);
    // Blendkurve: Linien lassen sich ziehen, Rechtsklick setzt auf „Weich“ zurück
    // Flaches Breitformat, nach oben begrenzt, damit die Box auf großen Monitoren nicht wuchtig wird
    let cw = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(cw, (cw * 0.33).clamp(84.0, 124.0)), Sense::click_and_drag());
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

    // Überblendzeit unter der Kurve
    ui.add_space(6.0);
    // Überschrift wie bei KOPFHÖRER, darunter Überblendzeit als kleines LCD und Regler
    ui.label(small("AUTO-FADE"));
    ui.horizontal(|ui| {
        let scale = (ui.available_width() / 230.0).clamp(1.0, 1.5);
        let (lcd, _) = ui.allocate_exact_size(vec2(52.0, 22.0) * scale, Sense::hover());
        hw::well(ui.painter(), lcd, 4.0);
        hw::lcd_text(ui.painter(), lcd.right_center() - vec2(6.0, 0.0), Align2::RIGHT_CENTER, &format!("{:4.1}", m.fade_secs), 13.0 * scale, theme::well_ink());
        ui.label(small("SEK."));
        let (resp, new) = hw::mini_fader(ui, "fadesecs", m.fade_secs, settings::DEFAULT_FADE_SECS, 1.0, 20.0, ui.available_width() - 4.0);
        resp.on_hover_text(tr("Überblendzeit"));
        if let Some(v) = new {
            out.push(FadeAction::FadeSecs((v * 2.0).round() / 2.0));
        }
    });

    egui::CollapsingHeader::new(small("MOTOR · ANLAUF & AUSLAUF")).id_salt("motor").show(ui, |ui| {
        let mut motor = m.motor;
        let mut changed = false;
        ui.spacing_mut().slider_width = ui.available_width() - 110.0;
        ui.horizontal(|ui| {
            ui.label(small("ANLAUF "));
            let r = ui.add(egui::Slider::new(&mut motor.spin_up_secs, 0.1..=2.0).step_by(0.05).suffix(" s"));
            if r.secondary_clicked() || r.double_clicked() {
                motor.spin_up_secs = Motor::default().spin_up_secs;
                changed = true;
            }
            changed |= r.changed();
        });
        ui.horizontal(|ui| {
            ui.label(small("AUSLAUF"));
            let r = ui.add(egui::Slider::new(&mut motor.spin_down_secs, 0.2..=4.0).step_by(0.1).suffix(" s"));
            if r.secondary_clicked() || r.double_clicked() {
                motor.spin_down_secs = Motor::default().spin_down_secs;
                changed = true;
            }
            changed |= r.changed();
        });
        if changed {
            out.push(FadeAction::Motor(motor));
        }
    });
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
