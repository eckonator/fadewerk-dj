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
    /// Kompakte Ansicht: flachere Pegelbrücke.
    pub compact: bool,
    /// Ein eigenes Kopfhörergerät ist offen: dann gibt es „Gerät“ statt „Ausg. 3/4“.
    pub head_device: bool,
    /// EQ je Player in dB und Kill-Tasten, Reihenfolge Bässe, Mitten, Höhen.
    pub eq_db: [[f32; 3]; 2],
    pub eq_kill: [[bool; 3]; 2],
    /// Laufende Aufnahme: Sekunden bisher, ob sie gerade abgeschlossen wird, geschriebene Bytes.
    pub recording: Option<(f64, bool, u64)>,
    /// Mikrofon an, mit aktuellem Pegel (0 bis 1).
    pub mic: Option<f32>,
    /// Beschriftung und Symbol der Effekt-Pads.
    pub fx_labels: &'a [String; fw_engine::FX_SLOTS],
    pub fx_icons: [crate::fx_icons::FxIcon; fw_engine::FX_SLOTS],
    /// Bitmaske der Pads, die gerade klingen.
    pub fx_playing: u32,
    pub fx_volume: f32,
}

pub enum MixerAction {
    Volume(usize, f32),
    HeadMode(HeadMode),
    HeadMix(f32),
    HeadVolume(f32),
    MasterVolume(f32),
    Eq { deck: usize, band: usize, db: f32 },
    EqKill { deck: usize, band: usize, on: bool },
    /// Aufnahme starten bzw. beenden.
    Record,
    Mic,
    /// Effekt-Pad auslösen: über Master oder nur auf dem Kopfhörer (vorhören).
    Fx { slot: usize, to_master: bool },
    FxVolume(f32),
}

fn small(text: &'static str) -> RichText {
    RichText::new(tr(text)).font(fonts::bold(10.5)).color(theme::dim()).extra_letter_spacing(0.8)
}

/// Vergrößerung für Tasten und Schrift, wenn die Mittelspalte breiter wird.
fn scale(ui: &egui::Ui) -> f32 {
    (ui.available_width() / 230.0).clamp(1.0, 1.3)
}

pub fn show_mixer(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    // Kopfzeile: MIC links in der Ecke, MIXER mittig, REC rechts in der Ecke
    let head = ui.vertical_centered(|ui| ui.label(RichText::new(tr("MIXER")).font(fonts::bold(15.0)).extra_letter_spacing(2.5))).response.rect;
    let (mic_w, rec_w, btn_h) = (62.0, 62.0, 20.0);
    let y = head.center().y - btn_h / 2.0;
    let mic_rect = Rect::from_min_size(pos2(head.left(), y), vec2(mic_w, btn_h));
    let rec_rect = Rect::from_min_size(pos2(head.right() - rec_w, y), vec2(rec_w, btn_h));
    if ui.put(mic_rect, |ui: &mut egui::Ui| mic_button(ui, mic_rect.size(), m.mic)).clicked() {
        out.push(MixerAction::Mic);
    }
    if ui.put(rec_rect, |ui: &mut egui::Ui| record_button(ui, rec_rect.size(), m.recording)).clicked() {
        out.push(MixerAction::Record);
    }
    // Luft zwischen Kopfzeile und EQ (in der kompakten Ansicht weniger)
    ui.add_space(if m.compact { 3.0 } else { 8.0 });
    eq_strip(ui, m, out);

    // Pegelbrücke: LED A | Fader A | MAIN | Fader B | LED B
    // Maße wachsen mit der Breite der Mittelspalte
    let w = ui.available_width();
    // Etwas flacher seit der EQ darüber sitzt, damit die Spalte auf kleinen Fenstern nicht überläuft
    let h = if m.compact { (w * 0.3).clamp(70.0, 90.0) } else { (w * 0.4).clamp(110.0, 180.0) };
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
    m.meters[2].paint_with_bands(&p, led_m, (!theme::lite().no_bands).then_some(m.bands), main_base);
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
    master_row(ui, m, out);

    // Das aktive Gerät steht in den Einstellungen; hier nur ein Hinweis, wenn es fehlt
    if !m.audio_ok {
        ui.colored_label(theme::cue(), tr("Keine Audio-Ausgabe gefunden"));
    }
}

/// MASTER-Lautstärke über die volle Breite (MIC und REC sitzen oben in der Kopfzeile).
pub fn master_row(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    ui.horizontal(|ui| {
        ui.label(small("MASTER"));
        let (resp, new) = hw::mini_fader(ui, "master", m.master_volume, settings::DEFAULT_MASTER_VOLUME, 0.0, 1.0, ui.available_width());
        if let Some(v) = hw::nudge(ui, &resp, m.master_volume, VOLUME_STEP, 0.0, 1.0).or(new) {
            out.push(MixerAction::MasterVolume(v));
        }
    });
}

/// Mikrofon-Taste: leuchtet rot, solange das Mikrofon offen ist; ein Balken unten zeigt den Pegel.
fn mic_button(ui: &mut egui::Ui, size: egui::Vec2, mic: Option<f32>) -> egui::Response {
    let font = fonts::bold(11.0);
    let b = hw::custom_button(ui, size, hw::Look::Color(theme::cue()), mic.is_some(), true, |p, r, color| {
        // Mikrofon-Symbol vor dem Text, beides zusammen mittig (wie Punkt und Text bei REC)
        let galley = p.layout_no_wrap("MIC".to_owned(), font.clone(), color);
        let icon = 12.0;
        let gap = 4.0;
        let left = r.center().x - (icon + gap + galley.size().x) / 2.0;
        crate::fx_icons::FxIcon::Mic.paint(p, Rect::from_center_size(pos2(left + icon / 2.0, r.center().y), vec2(icon, icon)), color);
        p.galley(pos2(left + icon + gap, r.center().y - galley.size().y / 2.0), galley, color);
        if let Some(level) = mic {
            // Pegel in dB von −48 bis 0 über die Breite
            let db = 20.0 * level.max(1e-6).log10();
            let t = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
            let bar = Rect::from_min_size(pos2(r.left() + 4.0, r.bottom() - 4.0), vec2((r.width() - 8.0) * t, 2.0));
            p.rect_filled(bar, 1.0, if level > 0.9 { theme::cue() } else { theme::play() });
        }
    });
    if mic.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }
    b.on_hover_text(if mic.is_some() { tr("Mikrofon ausschalten") } else { tr("Mikrofon einschalten (Gerät und Talkover unter Einstellungen → Audio)") })
}

/// Aufnahme-Taste: roter Punkt und REC, während der Aufnahme leuchtend mit blinkendem Punkt und Laufzeit.
fn record_button(ui: &mut egui::Ui, size: egui::Vec2, recording: Option<(f64, bool, u64)>) -> egui::Response {
    let stopping = recording.is_some_and(|(_, s, _)| s);
    let blink = ui.input(|i| i.time) % 1.0 < 0.6;
    let label = match recording {
        Some((secs, _, _)) => {
            let s = secs as u64;
            if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
        }
        None => "REC".to_owned(),
    };
    let b = hw::custom_button(ui, size, hw::Look::Color(theme::cue()), recording.is_some(), !stopping, |p, r, color| {
        let font = fonts::bold(11.0);
        let galley = p.layout_no_wrap(label, font, color);
        let dot_r = 3.5;
        let gap = 5.0;
        let total = 2.0 * dot_r + gap + galley.size().x;
        let left = r.center().x - total / 2.0;
        let dot = if recording.is_none() || blink { theme::cue() } else { theme::cue().gamma_multiply(0.25) };
        p.circle_filled(pos2(left + dot_r, r.center().y), dot_r, dot);
        p.galley(pos2(left + 2.0 * dot_r + gap, r.center().y - galley.size().y / 2.0), galley, color);
    });
    if recording.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    match recording {
        Some((_, _, bytes)) => b.on_hover_text(trf("Aufnahme beenden ({} MB)", &[&format!("{:.0}", bytes as f64 / 1e6)])),
        None => b.on_hover_text(tr("Mix aufnehmen (Format und Ordner unter Einstellungen → Audio)")),
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
/// Stellung eines EQ-Knopfs (0 bis 1) zu dB und zurück. 0 dB steht auf 12 Uhr: die linke Hälfte des Wegs senkt
/// bis −26 dB, die rechte hebt bis +6 dB.
pub fn eq_to_norm(db: f32) -> f32 {
    if db <= 0.0 { 0.5 * (1.0 - db / settings::EQ_MIN_DB) } else { 0.5 + 0.5 * db / settings::EQ_MAX_DB }
}

pub fn eq_from_norm(n: f32) -> f32 {
    if n <= 0.5 { settings::EQ_MIN_DB * (1.0 - 2.0 * n) } else { settings::EQ_MAX_DB * (2.0 * n - 1.0) }
}

fn eq_strip(ui: &mut egui::Ui, m: &MixerModel<'_>, out: &mut Vec<MixerAction>) {
    let w = ui.available_width();
    let (d, group_w, slot) = eq_geometry(w);
    let (label_h, kill_h) = (12.0, 15.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, label_h + d + 3.0 + kill_h), Sense::hover());
    let (to_norm, to_db) = (eq_to_norm, eq_from_norm);
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

/// Was die Übergangsleiste zeigt (berechnet in `app::transition_view`).
pub struct TransitionView {
    /// Statuszeile; steht in der Leiste, solange kein Übergang geplant ist oder läuft.
    pub status: String,
    /// Player, der gerade spielt und ausgeblendet wird.
    pub from: usize,
    /// Geplant: Sekunden, bis die Überblendung beginnt.
    pub until: Option<f64>,
    /// Länge der ganzen Überblendung in Echtzeit.
    pub lead: f32,
    /// Eingestellte Überblendzeit (in der Automatik-Ansicht in der Leiste änderbar).
    pub fade_secs: f32,
    /// FadeWerk zweiphasig: so lange blendet erst der neue Titel ein, danach der alte aus.
    pub split_in: Option<f32>,
    /// Läuft gerade: Fortschritt 0…1 und Restsekunden.
    pub fading: Option<(f32, f32)>,
    /// Titel, der hereinkommt.
    pub next: String,
    /// Was danach auf dem jetzt spielenden Player folgt.
    pub after: Option<(usize, String)>,
    pub curve: Curve,
    pub bass_swap: bool,
    /// Übergang im Takt (Beatraster beider Titel, AUTO SYNC an).
    pub on_beat: bool,
    /// Geplant, aber nicht im Takt: kurzer Grund und Erklärung (nur mit AUTO SYNC).
    pub beat_miss: Option<(String, String)>,
}

pub struct FadeModel<'a> {
    pub auto: bool,
    pub fade_secs: f32,
    pub curve: Curve,
    pub crossfader: f32,
    pub transition: &'a TransitionView,
    /// Übergangsleiste unter den Tasten zeigen (in der Automatik-Ansicht steht sie oben groß).
    pub show_transition: bool,
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
        let tip = tr("Beim Titelwechsel bekommt der neue Titel das Tempo des laufenden. Haben beide ein Beatraster und liegen höchstens 6 % auseinander, blendet Auto-Fade im Takt: Beginn an einer Phrasengrenze, Einstieg auf dem Taktbeginn, Länge in ganzen Takten. Danach läuft der Pitch langsam auf 0 % zurück (0,1 % pro Sekunde).");
        let sync = hw::button(ui, vec2(small_w, 34.0 * k), "AUTO\nSYNC", fonts::bold(10.0 * k), hw::Look::Color(theme::yellow()), m.auto_sync, true);
        if sync.on_hover_text(tip).clicked() {
            out.push(FadeAction::AutoSync(!m.auto_sync));
        }
    });
    if m.show_transition {
        ui.add_space(2.0);
        transition_bar(ui, m.transition, None, false, true);
    }
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

/// Übergangsleiste: wann übergeblendet wird, wie lange und wohin. Zwei Spuren zeigen die Lautstärke des
/// spielenden und des kommenden Titels über die Zeit, der weiße Strich ist „jetzt“. Vor der Überblendung läuft
/// ein Countdown, währenddessen wandert der Strich durch den Überblendbereich und die Leiste wird gelb.
/// Immer gleich hoch, damit die Mittelspalte nicht springt.
/// `height`: feste Höhe, alles wächst mit (Automatik-Ansicht); sonst richtet sie sich nach der Breite. Mit
/// `edit_secs` ist die Überblendzeit im Fuß ein kleines LCD: Klick zum Eintippen, Mausrad oder Pfeiltasten in
/// halben Sekunden; liefert dann den neuen Wert. Ohne `framed` zeichnet sie kein eigenes Display (sie sitzt dann
/// in einem gemeinsamen mit den Pegeln).
pub fn transition_bar(ui: &mut egui::Ui, t: &TransitionView, height: Option<f32>, edit_secs: bool, framed: bool) -> Option<f32> {
    let k = height.map_or(scale(ui).min(1.15), |h| h / 86.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 86.0 * k), Sense::hover());
    let painter = ui.painter().clone();
    let p = &painter;
    if framed {
        hw::well(p, rect, 6.0);
    }
    let inner = rect.shrink2(vec2(10.0, 7.0));
    let ink = theme::well_ink();
    let (from, to) = (t.from, 1 - t.from);

    let Some((now, until, fade_end)) = (match (t.fading, t.until) {
        // Fenster während der Überblendung: etwas Vorlauf und Nachlauf um den Überblendbereich
        (Some((progress, _)), _) => Some((progress * t.lead, 0.0, t.lead)),
        (None, Some(until)) => Some((0.0, until as f32, until as f32 + t.lead)),
        _ => None,
    }) else {
        // Nichts geplant: Statustext mittig in der leeren Leiste
        let g = p.layout(t.status.clone(), fonts::bold(11.5 * k), ink.gamma_multiply(0.6), inner.width());
        p.galley(rect.center() - g.size() / 2.0, g, ink);
        return None;
    };
    let (w0, w1) = if t.fading.is_some() { (-0.12 * t.lead, t.lead * 1.12) } else { (0.0, (fade_end + t.lead * 0.25).max(t.lead * 3.0)) };
    let x = |secs: f32| inner.left() + ((secs - w0) / (w1 - w0)).clamp(0.0, 1.0) * inner.width();

    // Kopf: was hereinkommt und der Countdown bzw. die Restzeit
    let head_y = inner.top() + 9.0 * k;
    let (caption, cap_color, count, count_color) = match t.fading {
        Some((_, rest)) => (tr("BLENDET ÜBER ZU"), theme::yellow(), rest as f64, theme::yellow()),
        None => (tr("ÜBERGANG ZU"), ink.gamma_multiply(0.55), until as f64, ink),
    };
    let count_text = crate::deck_ui::fmt_time(count);
    let count_w = p.layout_no_wrap(count_text.clone(), fonts::lcd(17.0 * k), ink).size().x;
    hw::lcd_text(p, pos2(inner.right(), head_y), Align2::RIGHT_CENTER, &count_text, 17.0 * k, count_color);
    let mut cx = inner.left();
    // Wird es eng (Netbook), fällt „ÜBERGANG ZU“ weg; Buchstabe und Titel bleiben
    let room = inner.width() - count_w - 10.0;
    let cap_w = p.layout_no_wrap(caption.to_owned(), fonts::bold(9.5 * k), cap_color).size().x;
    let caption = if room - cap_w < 90.0 * k { String::new() } else { caption.to_owned() };
    for (text, font, color) in [
        (caption, fonts::bold(9.5 * k), cap_color),
        (format!(" {} ", theme::deck_letter(to)), fonts::bold(13.0 * k), theme::deck_color(to)),
    ] {
        let g = p.layout_no_wrap(text, font, color);
        let w = g.size().x;
        p.galley(pos2(cx, head_y - g.size().y / 2.0), g, color);
        cx += w;
    }
    let title_w = (inner.right() - count_w - 10.0 - cx).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(t.next.clone(), egui::TextFormat::simple(fonts::bold(12.0 * k), ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
    let g = p.layout_job(job);
    p.galley(pos2(cx, head_y - g.size().y / 2.0), g, ink);

    // Zwei Spuren: Lautstärke des spielenden (oben) und des kommenden Titels über die Zeit
    let lane_h = 12.0 * k;
    let top = inner.top() + 22.0 * k;
    let in_end = until + t.split_in.unwrap_or(0.0);
    let out_start = if t.split_in.is_some() { in_end } else { until };
    let lanes = [
        (from, [(w0, 1.0), (out_start, 1.0), (fade_end, 0.0), (w1, 0.0)]),
        (to, [(w0, 0.0), (until, 0.0), (if t.split_in.is_some() { in_end } else { fade_end }, 1.0), (w1, 1.0)]),
    ];
    for (i, (d, pts)) in lanes.iter().enumerate() {
        let lane = Rect::from_min_size(pos2(inner.left(), top + i as f32 * (lane_h + 4.0)), vec2(inner.width(), lane_h));
        p.rect_filled(lane, 3.0, Color32::from_white_alpha(14));
        let color = theme::deck_color(*d);
        let mut mesh = egui::Mesh::default();
        for pair in pts.windows(2) {
            let ((t0, l0), (t1, l1)) = (pair[0], pair[1]);
            let (x0, x1) = (x(t0), x(t1));
            if x1 - x0 < 0.5 {
                continue;
            }
            let (c0, c1) = (color.gamma_multiply(l0), color.gamma_multiply(l1));
            let base = mesh.vertices.len() as u32;
            for (pos, c) in [(pos2(x0, lane.top()), c0), (pos2(x1, lane.top()), c1), (pos2(x1, lane.bottom()), c1), (pos2(x0, lane.bottom()), c0)] {
                mesh.colored_vertex(pos, c);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        p.add(mesh);
        // Buchstabe dunkel auf der vollen Spur, sonst in der Farbe des Players
        let lit = pts[0].1 > 0.5;
        let letter_color = if lit { Color32::from_black_alpha(220) } else { color };
        p.text(pos2(lane.left() + 5.0, lane.center().y), Align2::LEFT_CENTER, theme::deck_letter(*d), fonts::bold(9.5 * k), letter_color);
    }
    // Überblendbereich gestrichelt begrenzen, „jetzt“ als weißer Strich über beide Spuren
    let lanes_y = egui::Rangef::new(top - 2.0, top + 2.0 * lane_h + 6.0);
    for secs in [until, fade_end] {
        let xs = x(secs);
        let mut y = lanes_y.min;
        while y < lanes_y.max {
            p.line_segment([pos2(xs, y), pos2(xs, (y + 3.0).min(lanes_y.max))], Stroke::new(1.0, Color32::from_white_alpha(90)));
            y += 5.0;
        }
    }
    p.vline(x(now), lanes_y, Stroke::new(2.0, Color32::WHITE));

    // Fuß: Länge, Kurve, Bass-Swap links; was danach auf dem spielenden Player kommt rechts
    let foot_y = inner.bottom() - 6.0 * k;
    let curve = match t.curve {
        Curve::Soft => tr("Weich"),
        Curve::Linear => tr("Linear"),
        Curve::Cut => tr("Hart"),
        Curve::Custom(_) => tr("Eigene"),
    };
    let small = fonts::bold(10.0 * k);
    let mut changed = None;
    let mut x0 = inner.left();
    let mut left = if edit_secs {
        // Überblendzeit als kleines LCD zum Ändern; bei FadeWerk-Bereichen steht die geplante Länge dahinter
        let lcd = Rect::from_min_size(pos2(x0, foot_y - 9.0 * k), vec2(44.0 * k, 18.0 * k));
        hw::well(p, lcd, 3.0);
        let resp = ui.interact(lcd, ui.id().with("trans_secs"), Sense::click());
        let snap = |v: f32| ((v / FADE_TIME.step).round() * FADE_TIME.step).clamp(FADE_TIME.min, FADE_TIME.max);
        let keyed = hw::nudge(ui, &resp, snap(t.fade_secs), FADE_TIME.step, FADE_TIME.min, FADE_TIME.max);
        changed = secs_lcd(ui, lcd, resp, t.fade_secs, 11.0 * k, &FADE_TIME).or(keyed);
        x0 = lcd.right() + 5.0;
        if (t.lead - t.fade_secs).abs() > 0.05 { format!("s · FadeWerk {:.0} s · {curve}", t.lead) } else { format!("s · {curve}") }
    } else {
        format!("{:.0} s · {curve}", t.lead)
    };
    if t.bass_swap {
        left.push_str(" · Bass-Swap");
    }
    if t.on_beat {
        left.push_str(" · ");
        left.push_str(tr("im Takt"));
    }
    let lg = p.layout_no_wrap(left, small.clone(), ink.gamma_multiply(0.75));
    let mut left_w = lg.size().x + (x0 - inner.left());
    let miss_x = x0 + lg.size().x;
    p.galley(pos2(x0, foot_y - lg.size().y / 2.0), lg, ink);
    // Warum nicht im Takt: gedämpft hinter der Kurve, ausführlich als Tooltip
    if let Some((short, tip)) = &t.beat_miss {
        let mut job = egui::text::LayoutJob::simple_singleline(format!(" · {short}"), small.clone(), ink.gamma_multiply(0.45));
        job.wrap = egui::text::TextWrapping::truncate_at_width((inner.right() - miss_x).max(0.0));
        let g = p.layout_job(job);
        let r = Rect::from_min_size(pos2(miss_x, foot_y - g.size().y / 2.0), g.size());
        left_w += g.size().x;
        p.galley(r.min, g, ink);
        ui.interact(r, ui.id().with("beat_miss"), Sense::hover()).on_hover_text(tip);
    }
    if let Some((d, title)) = &t.after {
        let room = inner.width() - left_w - 14.0;
        let mut job = egui::text::LayoutJob::default();
        job.append(tr("danach: "), 0.0, egui::TextFormat::simple(small.clone(), ink.gamma_multiply(0.6)));
        job.append(theme::deck_letter(*d), 0.0, egui::TextFormat::simple(small.clone(), theme::deck_color(*d)));
        job.append(&format!(" · {title}"), 0.0, egui::TextFormat::simple(small.clone(), ink.gamma_multiply(0.75)));
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let g = p.layout_job(job);
        p.galley(pos2(inner.right() - g.size().x, foot_y - g.size().y / 2.0), g, ink);
    }
    changed
}

/// Pegel A | MAIN | B im Display der Automatik-Ansicht. Ohne eigene Schächte (das Display ist schon dunkles Glas),
/// die Beschriftung steht unten im Display, damit die Säulen mit dem Übergang daneben bündig enden.
pub fn display_meters(p: &egui::Painter, rect: Rect, meters: &[Meter; 3], bands: &crate::meter::Bands) {
    let label_h = 12.0;
    let bar = rect.with_max_y(rect.bottom() - label_h - 4.0);
    let led_w = (rect.width() * 0.17).clamp(9.0, 13.0);
    let gap = 5.0;
    let main_w = rect.width() - 2.0 * (led_w + gap);
    let a = Rect::from_min_size(bar.min, vec2(led_w, bar.height()));
    let main = Rect::from_min_size(pos2(a.right() + gap, bar.top()), vec2(main_w, bar.height()));
    let b = Rect::from_min_size(pos2(main.right() + gap, bar.top()), vec2(led_w, bar.height()));
    meters[0].paint(p, a);
    meters[2].paint_with_bands(p, main, (!theme::lite().no_bands).then_some(bands), led_w * 1.7);
    meters[1].paint(p, b);
    let ly = rect.bottom() - label_h / 2.0;
    let ink = theme::well_ink().gamma_multiply(0.55);
    p.text(pos2(a.center().x, ly), Align2::CENTER_CENTER, "A", fonts::bold(10.0), theme::deck_color(0));
    p.text(pos2(main.center().x, ly), Align2::CENTER_CENTER, "MAIN", fonts::bold(9.0), ink);
    p.text(pos2(b.center().x, ly), Align2::CENTER_CENTER, "B", fonts::bold(10.0), theme::deck_color(1));
}

/// Bedienfeld der Automatik-Ansicht rechts neben dem Display, über dessen ganze Höhe: drei gleich hohe Zeilen
/// AUTO | FADE, BASS SWAP | AUTO SYNC, REC, rechts daneben der senkrechte MASTER-Fader wie an einem Mischpult. So enden alle
/// Teile auf derselben Ober- und Unterkante wie das Display. Den Crossfader gibt es nur in der Live-Ansicht.
pub fn auto_controls(ui: &mut egui::Ui, f: &FadeModel<'_>, m: &MixerModel<'_>, fade_out: &mut Vec<FadeAction>, mixer_out: &mut Vec<MixerAction>) {
    let r = ui.max_rect();
    let gap = 6.0;
    // Master: Fader über die Höhe, Beschriftung darunter
    let label_h = 13.0;
    let fader_w = 40.0;
    let fader = Rect::from_min_max(pos2(r.right() - fader_w, r.top()), pos2(r.right(), r.bottom() - label_h - 3.0));
    let (resp, new) = hw::vertical_fader(ui, fader, "master_auto", m.master_volume, settings::DEFAULT_MASTER_VOLUME, false);
    if let Some(v) = hw::nudge(ui, &resp, m.master_volume, VOLUME_STEP, 0.0, 1.0).or(new) {
        mixer_out.push(MixerAction::MasterVolume(v));
    }
    resp.on_hover_text(tr("Master-Lautstärke"));
    ui.painter().text(pos2(fader.center().x, r.bottom() - label_h / 2.0), Align2::CENTER_CENTER, "MASTER", fonts::bold(9.5), theme::dim());

    // Drei gleich hohe Zeilen
    let grid = Rect::from_min_max(r.min, pos2(fader.left() - 12.0, r.bottom()));
    let row_h = (grid.height() - 2.0 * gap) / 3.0;
    let bw = (grid.width() - gap) / 2.0;
    let cell = |row: usize, col: usize, span: usize| {
        Rect::from_min_size(pos2(grid.left() + col as f32 * (bw + gap), grid.top() + row as f32 * (row_h + gap)), vec2(bw * span as f32 + gap * (span - 1) as f32, row_h))
    };
    let big = fonts::bold((row_h * 0.36).clamp(12.0, 16.0));
    let small_font = fonts::bold((row_h * 0.27).clamp(9.5, 11.5));
    let mut put = |rect: Rect, add: &mut dyn FnMut(&mut egui::Ui) -> egui::Response| ui.put(rect, |ui: &mut egui::Ui| add(ui));
    if put(cell(0, 0, 1), &mut |ui| hw::button(ui, cell(0, 0, 1).size(), "AUTO", big.clone(), hw::Look::Color(theme::play()), f.auto, true)).clicked() {
        fade_out.push(FadeAction::Auto(!f.auto));
    }
    if put(cell(0, 1, 1), &mut |ui| hw::button(ui, cell(0, 1, 1).size(), tr("FADE"), big.clone(), hw::Look::Plain, false, true)).on_hover_text(tr("Jetzt überblenden")).clicked() {
        fade_out.push(FadeAction::FadeNow);
    }
    let tip = tr("Beim Überblenden kommt der neue Titel ohne Bässe herein, in der Mitte tauschen die Bässe. Klingt sauberer als ein reiner Lautstärke-Fade.");
    if put(cell(1, 0, 1), &mut |ui| hw::button(ui, cell(1, 0, 1).size(), "BASS SWAP", small_font.clone(), hw::Look::Color(theme::yellow()), f.bass_swap, true)).on_hover_text(tip).clicked() {
        fade_out.push(FadeAction::BassSwap(!f.bass_swap));
    }
    let tip = tr("Beim Titelwechsel bekommt der neue Titel das Tempo des laufenden. Haben beide ein Beatraster und liegen höchstens 6 % auseinander, blendet Auto-Fade im Takt: Beginn an einer Phrasengrenze, Einstieg auf dem Taktbeginn, Länge in ganzen Takten. Danach läuft der Pitch langsam auf 0 % zurück (0,1 % pro Sekunde).");
    if put(cell(1, 1, 1), &mut |ui| hw::button(ui, cell(1, 1, 1).size(), "AUTO SYNC", small_font.clone(), hw::Look::Color(theme::yellow()), f.auto_sync, true)).on_hover_text(tip).clicked() {
        fade_out.push(FadeAction::AutoSync(!f.auto_sync));
    }
    if put(cell(2, 0, 1), &mut |ui| mic_button(ui, cell(2, 0, 1).size(), m.mic)).clicked() {
        mixer_out.push(MixerAction::Mic);
    }
    if put(cell(2, 1, 1), &mut |ui| record_button(ui, cell(2, 1, 1).size(), m.recording)).clicked() {
        mixer_out.push(MixerAction::Record);
    }
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
                // Symbol oben, Name klein darunter; einfache Pads nur mit Namen, mittig (die Linien-Symbole kosten Vertices)
                let lite = theme::lite().simple_pads;
                let b = hw::custom_button(ui, vec2(bw, 46.0 * k), hw::Look::Color(theme::select_accent()), lit, true, |p, r, color| {
                    if !lite {
                        let icon_rect = Rect::from_center_size(pos2(r.center().x, r.top() + r.height() * 0.38), vec2(r.height() * 0.42, r.height() * 0.42));
                        icon.paint(p, icon_rect, color);
                    }
                    let font = fonts::bold(if label.chars().count() > 8 { 8.0 } else { 9.0 } * k);
                    let galley = p.layout(label.clone(), font, color, r.width() - 6.0);
                    let y = if lite { r.center().y - galley.size().y / 2.0 } else { r.bottom() - galley.size().y - 4.0 * k };
                    let pos = pos2(r.center().x - galley.size().x / 2.0, y);
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
