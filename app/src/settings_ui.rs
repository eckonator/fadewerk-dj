// SPDX-License-Identifier: GPL-3.0-or-later
//! Einstellungsfenster: Audio, Sprache, Aussehen, Über (Copyright und Lizenzen).

use eframe::egui::{self, Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, vec2};

use crate::audio::{self, DeviceInfo};
use crate::i18n::{Lang, tr, trf};
use crate::settings::Settings;
use crate::theme::{self, SKINS, SkinId};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Audio,
    Language,
    Look,
    About,
}

/// Lizenztexte, die im Tab „Über“ angezeigt werden können: (Bezeichnung, Text).
const LICENSE_TEXTS: [(&str, &str); 3] = [
    ("GNU GPL 3.0 (FadeWerk DJ)", include_str!("../../LICENSE")),
    ("SIL OFL 1.1 (Chakra Petch)", include_str!("../../assets/fonts/OFL-ChakraPetch.txt")),
    ("SIL OFL 1.1 (DSEG)", include_str!("../../assets/fonts/OFL-DSEG.txt")),
];

pub enum SettingsAction {
    Device(Option<String>),
    HeadDevice(Option<String>),
    Buffer(Option<u32>),
    Language(Lang),
    Skin(SkinId),
    Brushed(bool),
    OpenGl(bool),
    RevealLog,
}

pub struct SettingsCtx<'a> {
    pub settings: &'a Settings,
    pub active_device: Option<&'a str>,
    /// Kennung des tatsächlich geöffneten Master-Geräts (auch bei „Systemstandard“).
    pub active_device_id: Option<&'a str>,
    pub active_head: Option<&'a str>,
    pub sample_rate: u32,
    pub channels: usize,
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    tab: Tab,
    devices: Option<Vec<DeviceInfo>>,
    /// Im Tab „Über“ aufgeklappter Lizenztext (Index in `LICENSE_TEXTS`).
    license_view: Option<usize>,
}

const BUFFERS: [Option<u32>; 7] = [None, Some(64), Some(128), Some(256), Some(512), Some(1024), Some(2048)];

impl SettingsUi {
    /// Öffnet das Fenster auf einer Seite (für die Entwicklerhilfen).
    pub fn open_at(&mut self, page: &str) {
        self.open = true;
        self.tab = match page {
            "sprache" => Tab::Language,
            "aussehen" => Tab::Look,
            "ueber" => Tab::About,
            _ => Tab::Audio,
        };
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            // Beim Öffnen frisch einlesen, damit neu angesteckte Geräte auftauchen
            self.devices = None;
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new(tr("Einstellungen"))
            .id(egui::Id::new("einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .default_width(520.0)
            .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(520.0);
                ui.horizontal(|ui| {
                    for (tab, label) in [(Tab::Audio, "Audio"), (Tab::Language, "Sprache"), (Tab::Look, "Aussehen"), (Tab::About, "Über")] {
                        let on = self.tab == tab;
                        let b = egui::Button::new(RichText::new(tr(label)).size(14.0).color(if on { theme::ground() } else { theme::dim() }))
                            .fill(if on { theme::ink() } else { theme::panel2() })
                            .min_size(vec2(110.0, 28.0));
                        if ui.add(b).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                match self.tab {
                    Tab::Audio => self.audio_tab(ui, c, out),
                    Tab::Language => language_tab(ui, c, out),
                    Tab::Look => look_tab(ui, c, out),
                    Tab::About => self.about_tab(ui, out),
                }
                if self.tab != Tab::About {
                    ui.add_space(10.0);
                    ui.label(RichText::new(tr("Einstellungen werden automatisch gespeichert.")).size(11.5).color(theme::faint()));
                }
            });
        self.open = open;
    }

    /// Copyright, Lizenz und Fremdbestandteile („Appropriate Legal Notices“ nach GPL-3.0 Abschnitt 0).
    fn about_tab(&mut self, ui: &mut egui::Ui, out: &mut Vec<SettingsAction>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("FadeWerk DJ").font(crate::fonts::bold(22.0)));
            ui.label(RichText::new(trf("Version {}", &[&env!("CARGO_PKG_VERSION")])).color(theme::dim()));
        });
        ui.label(tr("DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade."));
        ui.add_space(8.0);
        ui.label(RichText::new("Copyright © 2026 Markus Eckert").strong());
        ui.add_space(4.0);
        ui.label(tr("FadeWerk DJ ist freie Software: Du kannst es unter den Bedingungen der GNU General Public License, Version 3 oder (nach deiner Wahl) jeder späteren Version, weitergeben und verändern."));
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Es wird OHNE JEDE GEWÄHRLEISTUNG bereitgestellt, auch ohne die implizite Gewährleistung der Marktfähigkeit oder Eignung für einen bestimmten Zweck.")).color(theme::dim()));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(tr("Quellcode:"));
            ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
        });
        if let Some(log) = crate::diagnose::log_path() {
            ui.horizontal(|ui| {
                ui.label(tr("Logdatei:"));
                if ui.small_button(crate::browser::reveal_label()).clicked() {
                    out.push(SettingsAction::RevealLog);
                }
            });
            ui.add(egui::Label::new(RichText::new(log.display().to_string()).size(11.5).color(theme::dim())).wrap());
        }

        ui.add_space(10.0);
        ui.label(RichText::new(tr("Verwendete Fremdbestandteile")).strong());
        ui.label(RichText::new(tr("Schriften Chakra Petch und DSEG (SIL Open Font License 1.1), egui und eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0) und weitere Rust-Bibliotheken. Die vollständige Liste liegt jedem Release als THIRD-PARTY-LICENSES.yml bei.")).size(12.5).color(theme::dim()));

        ui.add_space(10.0);
        ui.label(RichText::new(tr("Lizenztexte")).strong());
        ui.horizontal_wrapped(|ui| {
            for (i, (name, _)) in LICENSE_TEXTS.iter().enumerate() {
                let on = self.license_view == Some(i);
                if ui.selectable_label(on, *name).clicked() {
                    self.license_view = if on { None } else { Some(i) };
                }
            }
        });
        if let Some(i) = self.license_view {
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("lizenztext").max_height(220.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(LICENSE_TEXTS[i].1).monospace().size(10.5).color(theme::dim())).wrap());
            });
        }
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        let devices = self.devices.get_or_insert_with(audio::list_output_devices).clone();
        let s = c.settings;

        ui.label(RichText::new(tr("Lautsprecher (Master)")).strong());
        let current = s
            .output_device
            .as_ref()
            .and_then(|id| devices.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
            .unwrap_or_else(|| tr("Systemstandard").to_owned());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("device").selected_text(current).width(360.0).show_ui(ui, |ui| {
                if ui.selectable_label(s.output_device.is_none(), tr("Systemstandard")).clicked() {
                    out.push(SettingsAction::Device(None));
                }
                for d in &devices {
                    let label = format!("{}  ({} ch)", d.name, d.max_channels);
                    if ui.selectable_label(s.output_device.as_deref() == Some(d.id.as_str()), label).clicked() {
                        out.push(SettingsAction::Device(Some(d.id.clone())));
                    }
                }
            });
            if ui.button(tr("Liste aktualisieren")).clicked() {
                self.devices = None;
            }
        });
        if let Some(name) = c.active_device {
            ui.label(RichText::new(trf("Aktiv: {} · {} Hz · {} Kanäle", &[&name, &c.sample_rate, &c.channels])).size(12.0).color(theme::dim()));
        }

        ui.add_space(10.0);
        ui.label(RichText::new(tr("Kopfhörer")).strong());
        let head_current = s
            .head_device
            .as_ref()
            .and_then(|id| devices.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
            .unwrap_or_else(|| tr("Kein eigenes Gerät").to_owned());
        egui::ComboBox::from_id_salt("head-device").selected_text(head_current).width(360.0).show_ui(ui, |ui| {
            if ui.selectable_label(s.head_device.is_none(), tr("Kein eigenes Gerät")).clicked() {
                out.push(SettingsAction::HeadDevice(None));
            }
            // Das Master-Gerät taucht hier nicht auf: dafür gibt es Split L/R und Ausgang 3/4
            for d in devices.iter().filter(|d| Some(d.id.as_str()) != c.active_device_id) {
                let label = format!("{}  ({} ch)", d.name, d.max_channels);
                if ui.selectable_label(s.head_device.as_deref() == Some(d.id.as_str()), label).clicked() {
                    out.push(SettingsAction::HeadDevice(Some(d.id.clone())));
                }
            }
        });
        let note = match c.active_head {
            Some(name) => trf("Kopfhörer: {}", &[&name]),
            None => tr("Ohne eigenes Gerät läuft der Kopfhörer über „Split L/R“ oder „Ausg. 3/4“ im Mixer.").to_owned(),
        };
        ui.label(RichText::new(note).size(12.0).color(theme::dim()));
        if c.active_head.is_some() {
            ui.label(
                RichText::new(tr("Zwei Geräte laufen nie exakt gleich schnell. FadeWerk DJ gleicht das automatisch aus; der Kopfhörer liegt dabei etwa 20 ms hinter den Lautsprechern."))
                    .size(12.0)
                    .color(theme::dim()),
            );
        }

        ui.add_space(10.0);
        ui.label(RichText::new(tr("Puffergröße")).strong());
        let rate = c.sample_rate.max(1) as f32;
        let buf_label = |b: Option<u32>| match b {
            None => tr("Standard").to_owned(),
            Some(n) => trf("{} Samples ≈ {} ms", &[&n, &format!("{:.1}", n as f32 / rate * 1000.0)]),
        };
        egui::ComboBox::from_id_salt("buffer").selected_text(buf_label(s.buffer_frames)).width(360.0).show_ui(ui, |ui| {
            for b in BUFFERS {
                if ui.selectable_label(s.buffer_frames == b, buf_label(b)).clicked() {
                    out.push(SettingsAction::Buffer(b));
                }
            }
        });
        ui.label(
            RichText::new(tr("Kleiner Puffer: weniger Verzögerung beim Scratchen, aber mehr Rechenlast. Bei Knacksern größer wählen."))
                .size(12.0)
                .color(theme::dim()),
        );
    }
}

fn language_tab(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    ui.label(RichText::new(tr("Sprache der Oberfläche")).strong());
    ui.add_space(4.0);
    for l in Lang::ALL {
        if ui.radio(c.settings.language == l, RichText::new(l.native_name()).size(14.0)).clicked() {
            out.push(SettingsAction::Language(l));
        }
    }
}

fn look_tab(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    ui.label(RichText::new(tr("Skin")).strong());
    ui.add_space(4.0);
    egui::Grid::new("skins").num_columns(2).spacing(vec2(10.0, 10.0)).show(ui, |ui| {
        for (i, s) in SKINS.iter().enumerate() {
            let selected = c.settings.skin == s.id;
            let (rect, resp) = ui.allocate_exact_size(vec2(250.0, 104.0), Sense::click());
            let p = ui.painter();
            // Vorschau in den Farben des jeweiligen Skins
            p.rect_filled(rect, CornerRadius::same(6), s.ground);
            let border = if selected { Stroke::new(2.5, theme::ink()) } else { Stroke::new(1.0, theme::line()) };
            p.rect_stroke(rect, CornerRadius::same(6), border, egui::StrokeKind::Inside);
            let inner = rect.shrink(10.0);
            let panel = egui::Rect::from_min_size(inner.min, vec2(inner.width(), 32.0));
            p.rect_filled(panel, CornerRadius::same(4), s.panel);
            if c.settings.brushed_steel {
                crate::hw::brushed_overlay(p, panel, 4.0, s.dark);
            }
            let swatches: [Color32; 5] = [s.deck_a, s.deck_b, s.cue, s.play, s.well];
            for (k, col) in swatches.iter().enumerate() {
                let r = egui::Rect::from_min_size(panel.min + vec2(7.0 + k as f32 * 24.0, 7.0), vec2(18.0, 18.0));
                p.rect_filled(r, CornerRadius::same(3), *col);
            }
            let name_pos = panel.left_bottom() + vec2(0.0, 6.0);
            p.text(name_pos, Align2::LEFT_TOP, s.name(), egui::FontId::proportional(14.0), s.ink);
            let about = p.layout(s.about().to_owned(), egui::FontId::proportional(10.5), s.dim, inner.width());
            p.galley(name_pos + vec2(0.0, 20.0), about, s.dim);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                out.push(SettingsAction::Skin(s.id));
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    ui.add_space(10.0);
    let mut brushed = c.settings.brushed_steel;
    if ui.checkbox(&mut brushed, RichText::new(tr("Gebürsteter Stahl")).size(14.0)).changed() {
        out.push(SettingsAction::Brushed(brushed));
    }
    ui.label(RichText::new(tr("Gehäuseflächen wirken wie gebürstetes Metall in den Farben des Skins.")).size(11.5).color(theme::dim()));
    ui.add_space(10.0);
    let mut opengl = c.settings.opengl;
    if ui.checkbox(&mut opengl, RichText::new(tr("Grafik über OpenGL")).size(14.0)).changed() {
        out.push(SettingsAction::OpenGl(opengl));
    }
    ui.label(
        RichText::new(tr("Für Rechner, auf denen das Fenster nicht zuverlässig startet (etwa virtuelle Maschinen). Wirkt nach einem Neustart."))
            .size(11.5)
            .color(theme::dim()),
    );
}
