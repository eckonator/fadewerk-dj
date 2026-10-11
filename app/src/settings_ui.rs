// SPDX-License-Identifier: GPL-3.0-or-later
//! Einstellungsfenster: Allgemein, Audio, Effekte, Sprache, Aussehen, Über (Copyright und Lizenzen).

use eframe::egui::{self, Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, vec2};

use crate::audio::{self, DeviceInfo};
use crate::i18n::{Lang, tr, trf};
use crate::settings::Settings;
use crate::theme::{self, SKINS, SkinId};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    General,
    Audio,
    Effects,
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

/// Innere Breite des Einstellungsfensters, für alle Seiten gleich. Die breiteste Seite (Effekte: Nummer,
/// Symbol, Name, Datei, Tasten) gibt sie vor; sonst drückt sie das Fenster breiter.
const SETTINGS_W: f32 = 557.0;

pub enum SettingsAction {
    Device(Option<String>),
    HeadDevice(Option<String>),
    /// Kurzen Testton über Master oder (`false`) auf dem Kopfhörer spielen.
    TestTone(bool),
    Buffer(Option<u32>),
    Language(Lang),
    Skin(SkinId),
    Brushed(bool),
    Lite(bool),
    Platters(bool),
    OpenGl(bool),
    UpdateCheck(bool),
    /// Sofort bei GitHub nach einer neuen Version fragen.
    CheckUpdateNow,
    RecordHistory(bool),
    KeepAwake(bool),
    Zoom(f32),
    RevealLog,
    /// Eigener Name eines Effekt-Pads (leer = Standard).
    FxName(usize, String),
    /// Datei für ein Effekt-Pad auswählen (öffnet den Dateidialog).
    FxPickFile(usize),
    /// Effekt-Pad auf den mitgelieferten Klang zurücksetzen.
    FxReset(usize),
    /// Effekt eines Pads auf dem Kopfhörer anhören.
    FxPreview(usize),
    /// Symbol eines Effekt-Pads (`FxIcon::key`).
    FxIcon(usize, String),
    /// Alle Effekt-Pads auf die mitgelieferten Klänge, Namen und Symbole zurücksetzen.
    FxResetAll,
    /// Größe der Caches messen (für die Anzeige unter Allgemein).
    MeasureCaches,
    /// Caches leeren, mit `bpm` auch die ermittelten BPM.
    ClearCaches { bpm: bool },
    /// Alles löschen (Einstellungen, Playlists, Historie, Caches) und neu starten.
    ResetAll,
}

pub struct SettingsCtx<'a> {
    pub settings: &'a Settings,
    pub active_device: Option<&'a str>,
    /// Kennung des tatsächlich geöffneten Master-Geräts (auch bei „Systemstandard“).
    pub active_device_id: Option<&'a str>,
    pub active_head: Option<&'a str>,
    pub sample_rate: u32,
    pub channels: usize,
    /// Gemessene Größe der Caches, `None` solange nicht gemessen.
    pub cache_sizes: Option<crate::caches::CacheSizes>,
    /// Spielt ein Player? Dann lässt sich nicht zurücksetzen (die App startet neu).
    pub playing: bool,
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    tab: Tab,
    devices: Option<Vec<DeviceInfo>>,
    /// Im Tab „Über“ aufgeklappter Lizenztext (Index in `LICENSE_TEXTS`).
    license_view: Option<usize>,
    /// Pad, für das gerade ein Symbol gewählt wird.
    icon_picker: Option<usize>,
    /// „Alle Effekte zurücksetzen“ wurde einmal geklickt und wartet auf Bestätigung.
    confirm_fx_reset: bool,
    /// „Caches leeren“ bzw. „Zurücksetzen“ wartet auf Bestätigung.
    confirm_clear: bool,
    confirm_reset: bool,
    /// Beim Leeren auch die ermittelten BPM löschen.
    clear_bpm: bool,
}

const BUFFERS: [Option<u32>; 7] = [None, Some(64), Some(128), Some(256), Some(512), Some(1024), Some(2048)];

impl SettingsUi {
    /// Öffnet das Fenster auf einer Seite (Entwicklerhilfen, Klick auf die Versionsnummer).
    pub fn open_at(&mut self, page: &str) {
        self.open = true;
        self.tab = match page {
            "sprache" => Tab::Language,
            "aussehen" => Tab::Look,
            "ueber" => Tab::About,
            "audio" => Tab::Audio,
            "effekte" => Tab::Effects,
            _ => Tab::General,
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
            .default_width(SETTINGS_W)
            .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(SETTINGS_W);
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::General, "Allgemein"),
                        (Tab::Audio, "Audio"),
                        (Tab::Effects, "Effekte"),
                        (Tab::Language, "Sprache"),
                        (Tab::Look, "Aussehen"),
                        (Tab::About, "Über"),
                    ] {
                        let on = self.tab == tab;
                        let b = egui::Button::new(RichText::new(tr(label)).size(14.0).color(if on { theme::ground() } else { theme::dim() }))
                            .fill(if on { theme::ink() } else { theme::panel2() })
                            .min_size(vec2(80.0, 28.0));
                        if ui.add(b).clicked() {
                            self.tab = tab;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                match self.tab {
                    Tab::General => self.general_tab(ui, c, out),
                    Tab::Audio => self.audio_tab(ui, c, out),
                    Tab::Effects => self.effects_tab(ui, c, out),
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

    /// Effekt-Pads: je Pad Symbol, Name und Datei, Anhören und Zurücksetzen auf den mitgelieferten Klang.
    fn effects_tab(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        ui.label(RichText::new(tr("Effekt-Pads")).strong());
        ui.label(
            RichText::new(tr("Jedes Pad spielt einen mitgelieferten Effekt oder eine eigene Datei (MP3, WAV, FLAC, OGG, M4A). Auslösen mit Klick oder den Tasten 1 bis 8, Rechtsklick auf ein Pad hört es im Kopfhörer vor."))
                .size(12.0)
                .color(theme::dim()),
        );
        ui.add_space(8.0);
        egui::Grid::new("effekte").num_columns(6).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            for slot in 0..fw_engine::FX_SLOTS {
                let s = c.settings.fx_slot(slot);
                ui.label(RichText::new(format!("{}", slot + 1)).font(crate::fonts::bold(14.0)).color(theme::dim()));
                let icon = s.icon(slot);
                let picking = self.icon_picker == Some(slot);
                let b = crate::hw::custom_button(ui, vec2(30.0, 26.0), crate::hw::Look::Color(theme::select_accent()), picking, true, |p, r, color| {
                    icon.paint(p, r.shrink(5.0), color);
                });
                if b.on_hover_text(tr("Symbol wählen")).clicked() {
                    self.icon_picker = if picking { None } else { Some(slot) };
                }
                // Ohne eigenen Namen steht der des Klangs als Hinweis im Feld
                let mut name = s.name.clone();
                let hint = crate::fx::FxSlot { name: String::new(), ..s.clone() }.label(slot);
                let r = ui.add_sized(vec2(120.0, 22.0), egui::TextEdit::singleline(&mut name).hint_text(hint).char_limit(14));
                if r.changed() {
                    out.push(SettingsAction::FxName(slot, name));
                }
                let file = match &s.file {
                    Some(f) => f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    None => tr("Mitgeliefert").to_owned(),
                };
                let shown = ui.add_sized(vec2(140.0, 20.0), egui::Label::new(RichText::new(&file).size(12.5).color(if s.file.is_some() { theme::ink() } else { theme::faint() })).truncate());
                if let Some(f) = &s.file {
                    shown.on_hover_text(f.display().to_string());
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("Datei …")).clicked() {
                        out.push(SettingsAction::FxPickFile(slot));
                    }
                    if ui.add_enabled(s.file.is_some() || !s.name.is_empty() || s.icon.is_some(), egui::Button::new(tr("Standard")))
                        .on_hover_text(tr("Mitgelieferten Effekt, Namen und Symbol wiederherstellen"))
                        .clicked()
                    {
                        out.push(SettingsAction::FxReset(slot));
                    }
                });
                if ui.button("🎧").on_hover_text(tr("Im Kopfhörer anhören")).clicked() {
                    out.push(SettingsAction::FxPreview(slot));
                }
                ui.end_row();
            }
        });
        // Auswahl des Symbols für das gewählte Pad
        if let Some(slot) = self.icon_picker {
            let current = c.settings.fx_slot(slot).icon(slot);
            ui.add_space(8.0);
            ui.label(RichText::new(trf("Symbol für Pad {}", &[&(slot + 1)])).strong());
            ui.horizontal_wrapped(|ui| {
                for icon in crate::fx_icons::FxIcon::ALL {
                    let on = icon == current;
                    let b = crate::hw::custom_button(ui, vec2(38.0, 34.0), crate::hw::Look::Color(theme::select_accent()), on, true, |p, r, color| {
                        icon.paint(p, r.shrink(8.0), color);
                    });
                    if b.on_hover_text(tr(icon.name())).clicked() {
                        out.push(SettingsAction::FxIcon(slot, icon.key().to_owned()));
                        self.icon_picker = None;
                    }
                }
            });
        }
        // Alle Pads auf einmal zurücksetzen, mit Rückfrage (eigene Dateien gehen dabei verloren)
        ui.add_space(10.0);
        let changed = (0..fw_engine::FX_SLOTS).any(|i| c.settings.fx_slot(i) != crate::fx::FxSlot::default());
        ui.horizontal(|ui| {
            if self.confirm_fx_reset && changed {
                ui.label(RichText::new(tr("Alle eigenen Dateien, Namen und Symbole entfernen?")).color(theme::yellow()));
                if ui.button(tr("Ja, zurücksetzen")).clicked() {
                    out.push(SettingsAction::FxResetAll);
                    self.confirm_fx_reset = false;
                    self.icon_picker = None;
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    self.confirm_fx_reset = false;
                }
            } else {
                self.confirm_fx_reset = false;
                let b = ui.add_enabled(changed, egui::Button::new(tr("Alle Effekte zurücksetzen")));
                if b.on_hover_text(tr("Alle Pads wieder mit den mitgelieferten Effekten, Namen und Symbolen belegen")).clicked() {
                    self.confirm_fx_reset = true;
                }
            }
        });
    }

    /// Allgemeine Häkchen, darunter Caches und ganz unten, abgesetzt, das Zurücksetzen.
    fn general_tab(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        general_options(ui, c, out);

        ui.add_space(10.0);
        ui.label(RichText::new(tr("Caches")).strong());
        ui.label(
            RichText::new(tr("Gemerkte Tags, Ordnerlisten und Wellenformen beschleunigen große Sammlungen und Netzlaufwerke. Leeren ist ungefährlich: Alles wird bei Bedarf neu gelesen."))
                .size(12.0)
                .color(theme::dim()),
        );
        use crate::caches::fmt_bytes;
        match c.cache_sizes {
            Some(s) => {
                let line = trf("Tags {} · Ordnerlisten {} · Wellenformen {} · ermittelte BPM {}", &[&fmt_bytes(s.tags), &fmt_bytes(s.folders), &fmt_bytes(s.waves), &fmt_bytes(s.bpm)]);
                ui.label(RichText::new(line).size(12.5));
            }
            None => {
                out.push(SettingsAction::MeasureCaches);
                ui.label(RichText::new(tr("Wird gemessen …")).size(12.5).color(theme::dim()));
            }
        }
        ui.checkbox(&mut self.clear_bpm, RichText::new(tr("Auch ermittelte BPM löschen")).size(14.0))
            .on_hover_text(tr("Die Tempo-Erkennung dauert lange; ohne Häkchen bleiben ermittelte BPM erhalten."));
        ui.horizontal(|ui| {
            if self.confirm_clear {
                ui.label(RichText::new(tr("Caches wirklich leeren?")).color(theme::yellow()));
                if ui.button(tr("Ja, leeren")).clicked() {
                    out.push(SettingsAction::ClearCaches { bpm: self.clear_bpm });
                    self.confirm_clear = false;
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    self.confirm_clear = false;
                }
            } else if ui.button(tr("Caches leeren …")).clicked() {
                self.confirm_clear = true;
            }
        });

        // Abgesetzt und rot, damit man es nicht mit „Caches leeren“ verwechselt
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(RichText::new(tr("Zurücksetzen")).strong().color(theme::cue()));
        ui.label(
            RichText::new(tr("Löscht alle Einstellungen, Playlists, die Historie, eigene Effekt-Pads und alle Caches. Deine Musikdateien bleiben unberührt. Danach startet FadeWerk DJ neu wie beim ersten Mal."))
                .size(12.0)
                .color(theme::dim()),
        );
        ui.horizontal(|ui| {
            if self.confirm_reset && !c.playing {
                ui.label(RichText::new(tr("Wirklich alles löschen? Das lässt sich nicht rückgängig machen.")).color(theme::cue()));
                if ui.button(RichText::new(tr("Ja, zurücksetzen")).color(theme::cue())).clicked() {
                    out.push(SettingsAction::ResetAll);
                    self.confirm_reset = false;
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    self.confirm_reset = false;
                }
            } else {
                self.confirm_reset = false;
                let b = ui
                    .add_enabled(!c.playing, egui::Button::new(RichText::new(tr("FadeWerk DJ zurücksetzen …")).color(theme::cue())))
                    .on_disabled_hover_text(tr("Gesperrt, solange ein Player spielt"));
                if b.clicked() {
                    self.confirm_reset = true;
                }
            }
        });
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
            if ui.button(tr("Testton")).on_hover_text(tr("Spielt einen kurzen Ton über die Lautsprecher (Master)")).clicked() {
                out.push(SettingsAction::TestTone(true));
            }
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
        ui.horizontal(|ui| {
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
            if ui.button(tr("Testton")).on_hover_text(tr("Spielt einen kurzen Ton auf dem Kopfhörer")).clicked() {
                out.push(SettingsAction::TestTone(false));
            }
        });
        let note = match c.active_head {
            Some(name) => trf("Kopfhörer: {}", &[&name]),
            None if s.head_device.is_some() => tr("Das gewählte Gerät ist nicht aktiv (beim Start nicht verbunden). Zum Einschalten in der Liste erneut wählen.").to_owned(),
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

/// Was zu keinem Gerät und keiner Optik gehört: Historie, Ruhezustand, Updates.
fn general_options(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    ui.label(RichText::new(tr("Historie")).strong());
    let mut record = c.settings.record_history;
    if ui.checkbox(&mut record, RichText::new(tr("Gespielte Titel aufzeichnen")).size(14.0)).changed() {
        out.push(SettingsAction::RecordHistory(record));
    }
    ui.label(
        RichText::new(tr("Ein Titel zählt, sobald er 30 Sekunden über Master zu hören war. Zu finden im Ordnerbrowser unter „Historie“."))
            .size(12.0)
            .color(theme::dim()),
    );

    ui.add_space(10.0);
    ui.label(RichText::new(tr("Ruhezustand")).strong());
    let mut awake = c.settings.keep_awake;
    if ui.checkbox(&mut awake, RichText::new(tr("Ruhezustand verhindern, solange Musik spielt")).size(14.0)).changed() {
        out.push(SettingsAction::KeepAwake(awake));
    }
    ui.label(
        RichText::new(tr("Bildschirm und Rechner bleiben an, solange ein Player oder das Vorhören läuft. Danach gilt wieder die Einstellung des Systems."))
            .size(12.0)
            .color(theme::dim()),
    );

    ui.add_space(10.0);
    ui.label(RichText::new(tr("Updates")).strong());
    let mut check = c.settings.update_check.unwrap_or(false);
    if ui.checkbox(&mut check, RichText::new(tr("Beim Start bei GitHub nach einer neuen Version suchen")).size(14.0)).changed() {
        out.push(SettingsAction::UpdateCheck(check));
    }
    ui.label(RichText::new(tr("Gesendet wird nur die Frage nach der neuesten Version, keine Daten über Titel oder Ordner. GitHub sieht dabei wie bei jedem Webseitenaufruf deine IP-Adresse.")).size(11.5).color(theme::dim()));
    ui.add_space(4.0);
    if ui.button(tr("Jetzt nach Updates suchen")).clicked() {
        out.push(SettingsAction::CheckUpdateNow);
    }
    if crate::self_update::asset_suffix().is_some() {
        ui.label(RichText::new(tr("Eine neue Version installiert sich per Knopf selbst; die App startet danach neu. Auf dem Mac nur, wenn sie mit demselben Zertifikat signiert ist.")).size(11.5).color(theme::dim()));
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
            if theme::brushed() {
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
    // In der sparsamen Darstellung gibt es keinen Stahl
    if ui.add_enabled(!c.settings.lite_ui, egui::Checkbox::new(&mut brushed, RichText::new(tr("Gebürsteter Stahl")).size(14.0))).changed() {
        out.push(SettingsAction::Brushed(brushed));
    }
    ui.label(RichText::new(tr("Gehäuseflächen wirken wie gebürstetes Metall in den Farben des Skins.")).size(11.5).color(theme::dim()));
    ui.add_space(10.0);
    let mut platters = c.settings.platters;
    if ui.checkbox(&mut platters, RichText::new(tr("Plattenspieler zeigen")).size(14.0)).changed() {
        out.push(SettingsAction::Platters(platters));
    }
    ui.label(
        RichText::new(tr("Ohne Plattenteller bekommen die Bedienelemente mehr Platz. Scratchen geht weiter in der Wellenform."))
            .size(11.5)
            .color(theme::dim()),
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Zoom")).size(14.0));
        ui.add_space(6.0);
        let z = c.settings.ui_zoom;
        let font = egui::FontId::proportional(15.0);
        if crate::hw::button(ui, vec2(32.0, 26.0), "−", font.clone(), crate::hw::Look::Plain, false, z > 0.71).clicked() {
            out.push(SettingsAction::Zoom(z - 0.1));
        }
        ui.label(RichText::new(format!("{:.0} %", z * 100.0)).font(crate::fonts::lcd(15.0)).color(theme::ink()));
        if crate::hw::button(ui, vec2(32.0, 26.0), "+", font.clone(), crate::hw::Look::Plain, false, z < 1.49).clicked() {
            out.push(SettingsAction::Zoom(z + 0.1));
        }
        if crate::hw::button(ui, vec2(64.0, 26.0), "100 %", egui::FontId::proportional(13.0), crate::hw::Look::Plain, false, (z - 1.0).abs() > 0.01).clicked() {
            out.push(SettingsAction::Zoom(1.0));
        }
    });
    let tip = if cfg!(target_os = "macos") { tr("Ganze Oberfläche kleiner oder größer, auch mit ⌘ + und ⌘ − (⌘ 0 = 100 %).") } else { tr("Ganze Oberfläche kleiner oder größer, auch mit Strg + und Strg − (Strg 0 = 100 %).") };
    ui.label(RichText::new(tip).size(11.5).color(theme::dim()));
    ui.add_space(10.0);
    let mut lite = c.settings.lite_ui;
    if ui.checkbox(&mut lite, RichText::new(tr("Sparsame Darstellung (für ältere oder langsame Rechner)")).size(14.0)).changed() {
        out.push(SettingsAction::Lite(lite));
    }
    ui.label(
        RichText::new(tr("Ohne Schatten, Verläufe, Leuchten und Frequenzbänder, höchstens 30 Bilder pro Sekunde. Spart Rechenzeit, der Ton bleibt gleich."))
            .size(11.5)
            .color(theme::dim()),
    );
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
