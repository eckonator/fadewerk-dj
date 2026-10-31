// SPDX-License-Identifier: GPL-3.0-or-later
//! Einstellungsfenster: Seitenleiste links (Allgemein, Audio, Musik, Effekte, Darstellung, Über), rechts die Seite
//! aus Karten. Ein/Aus sind LED-Taster wie an den Playern, längere Erklärungen stehen im Tooltip hinter ⓘ.

use eframe::egui::{self, Align2, Color32, CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::audio::{self, DeviceInfo};
use crate::i18n::{Lang, tr, trf};
use crate::settings::Settings;
use crate::theme::{self, SKINS, SkinId};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    General,
    /// Geräte, Puffer, andere Geräusche, Mix aufnehmen, Historie.
    Audio,
    /// Titel online erkennen, Caches.
    Music,
    Effects,
    /// Aussehen und Leistung, umgeschaltet über `SettingsUi::performance`.
    Display,
    About,
}

impl Page {
    const ALL: [Page; 6] = [Page::General, Page::Audio, Page::Music, Page::Effects, Page::Display, Page::About];

    fn name(self) -> &'static str {
        match self {
            Page::General => tr("Allgemein"),
            Page::Audio => tr("Audio"),
            Page::Music => tr("Musik"),
            Page::Effects => tr("Effekte"),
            Page::Display => tr("Darstellung"),
            Page::About => tr("Über"),
        }
    }
}

/// Lizenztexte, die auf der Seite „Über“ angezeigt werden können: (Bezeichnung, Text).
const LICENSE_TEXTS: [(&str, &str); 3] = [
    ("GNU GPL 3.0 (FadeWerk DJ)", include_str!("../../LICENSE")),
    ("SIL OFL 1.1 (Chakra Petch)", include_str!("../../assets/fonts/OFL-ChakraPetch.txt")),
    ("SIL OFL 1.1 (DSEG)", include_str!("../../assets/fonts/OFL-DSEG.txt")),
];

/// Breite der Seiten, für alle gleich. Die breiteste Seite (Effekte: Nummer, Symbol, Name, Datei, Tasten, in einer
/// Karte) gibt sie vor; sonst drückt sie das Fenster breiter.
const PAGE_W: f32 = 584.0;
/// Seitenleiste links und Abstand bis zur Seite.
const NAV_W: f32 = 150.0;
const NAV_GAP: f32 = 18.0;
/// Höhe eines Eintrags in der Seitenleiste.
const NAV_ROW_H: f32 = 34.0;
/// Abstand zwischen zwei Karten.
const CARD_GAP: f32 = 10.0;
/// Breite der Beschriftung vor Auswahlfeldern und Tasten in einer Karte („Puffergröße“, „Zoom“ …).
const FIELD_W: f32 = 156.0;
/// LED-Taster AN/AUS rechts in einer Schalter-Zeile.
const SWITCH: egui::Vec2 = vec2(48.0, 22.0);
/// Höhe von Tasten und Segment-Tasten in den Karten.
const BTN_H: f32 = 26.0;
/// Abstand des Fensters vom oberen Rand beim Öffnen.
const SETTINGS_TOP: f32 = 48.0;
/// So viel Platz bleibt unter dem Scrollbereich der Seiten (Rand des Fensters und Abstand zum Bildschirmrand).
const SETTINGS_BOTTOM_RESERVE: f32 = 40.0;

pub enum SettingsAction {
    Device(Option<String>),
    HeadDevice(Option<String>),
    /// Kurzen Testton über Master oder (`false`) auf dem Kopfhörer spielen.
    TestTone(bool),
    Buffer(Option<u32>),
    Language(Lang),
    Skin(SkinId),
    Brushed(bool),
    Lite(theme::Lite),
    Platters(bool),
    /// Kompakte Ansicht: `None` = automatisch.
    Compact(Option<bool>),
    OpenGl(bool),
    UpdateCheck(bool),
    OnlineLookup(bool),
    /// Sofort bei GitHub nach einer neuen Version fragen.
    CheckUpdateNow,
    RecordHistory(bool),
    RecordFormat(crate::recorder::RecordFormat),
    /// Ordner für Aufnahmen wählen (öffnet den Dialog).
    RecordPickDir,
    /// Ordner für Aufnahmen auf den Standard zurück.
    RecordDirDefault,
    /// Ordner für Aufnahmen im Dateimanager zeigen.
    RecordRevealDir,
    KeepAwake(bool),
    /// Systemtöne und andere Programme stumm schalten.
    MuteOthers(bool),
    /// macOS: Ausgabegerät exklusiv belegen.
    Exclusive(bool),
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
    /// Größe der Caches messen (für die Anzeige unter Musik).
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
    /// Kompakte Ansicht aktiv (auch automatisch)? Dann gibt es keine Plattenteller.
    pub compact: bool,
}

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    /// Im nächsten Bild oben mittig hinstellen (beim Öffnen), danach frei verschiebbar.
    place: bool,
    page: Page,
    /// Unter „Darstellung“ die Leistung statt des Aussehens zeigen.
    performance: bool,
    devices: Option<Vec<DeviceInfo>>,
    /// Auf der Seite „Über“ aufgeklappter Lizenztext (Index in `LICENSE_TEXTS`).
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
        self.place = true;
        self.performance = page == "leistung";
        self.page = match page {
            "aussehen" | "leistung" | "darstellung" => Page::Display,
            "aufnahme" | "audio" => Page::Audio,
            "musik" => Page::Music,
            "ueber" => Page::About,
            "effekte" => Page::Effects,
            _ => Page::General,
        };
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.place = self.open;
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
        // Oben mittig statt zentriert: Die Seiten sind verschieden hoch, die Seitenleiste bleibt so beim
        // Durchklicken an ihrem Platz. Nur beim Öffnen gesetzt, danach lässt sich das Fenster ziehen.
        let width = NAV_W + NAV_GAP + PAGE_W;
        let mut window = egui::Window::new(tr("Einstellungen"))
            .id(egui::Id::new("einstellungen"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_TOP);
        if std::mem::take(&mut self.place) {
            let screen = ctx.content_rect();
            window = window.current_pos(egui::pos2(screen.center().x, screen.top() + SETTINGS_TOP));
        }
        window
            .default_width(width)
            .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(width);
                // Seiten scrollbar, höchstens so hoch wie der Bildschirm unter dem Fenster erlaubt (auf Netbooks
                // mit 600 Pixeln war „Aussehen“ sonst nicht bis unten erreichbar)
                let max_h = (ctx.content_rect().bottom() - ui.cursor().top() - SETTINGS_BOTTOM_RESERVE).max(160.0);
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let nav_h = Page::ALL.len() as f32 * (NAV_ROW_H + 2.0);
                    ui.allocate_ui_with_layout(vec2(NAV_W, nav_h), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for page in Page::ALL {
                            if nav_entry(ui, page, self.page == page).clicked() {
                                self.page = page;
                                self.icon_picker = None;
                            }
                        }
                    });
                    let (line, _) = ui.allocate_exact_size(vec2(NAV_GAP, nav_h), Sense::hover());
                    let page = ui.allocate_ui_with_layout(vec2(PAGE_W, max_h), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.set_width(PAGE_W);
                        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                        // Feste statt schwebender Leiste: sonst merkt man auf kleinen Bildschirmen nicht, dass es weitergeht
                        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                        egui::ScrollArea::vertical()
                            .id_salt(("einstellungen", self.page as u8, self.performance))
                            .max_height(max_h)
                            .min_scrolled_height(max_h.min(nav_h))
                            .show(ui, |ui| self.page_content(ui, c, out));
                    });
                    // Trennlinie zwischen Leiste und Seite über die ganze Höhe der Seite
                    let x = line.left() + 8.0;
                    let bottom = page.response.rect.bottom().max(line.bottom());
                    ui.painter().line_segment([pos2(x, line.top()), pos2(x, bottom)], Stroke::new(1.0, theme::line()));
                });
            });
        self.open = open;
    }

    fn page_content(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        // Rand rechts für die Scrollleiste, damit Karten nicht darunter liegen
        ui.set_width(PAGE_W - 14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.page.name()).font(crate::fonts::bold(19.0)).color(theme::ink()));
            if self.page == Page::Display {
                // Aussehen und Leistung als Unterseiten: zusammen wäre die Seite sehr lang
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let options = [(true, tr("Leistung"), true), (false, tr("Aussehen"), true)];
                    if let Some(perf) = segmented(ui, self.performance, &options) {
                        self.performance = perf;
                    }
                });
            }
        });
        ui.add_space(6.0);
        match self.page {
            Page::General => {
                general_page(ui, c, out);
                self.reset_card(ui, c, out);
            }
            Page::Audio => {
                self.audio_cards(ui, c, out);
                recording_cards(ui, c, out);
            }
            Page::Music => self.music_page(ui, c, out),
            Page::Effects => self.effects_page(ui, c, out),
            Page::Display if self.performance => performance_page(ui, c, out),
            Page::Display => look_page(ui, c, out),
            Page::About => self.about_page(ui, out),
        }
        if self.page != Page::About {
            ui.label(RichText::new(tr("Einstellungen werden automatisch gespeichert.")).size(11.5).color(theme::faint()));
        }
    }

    /// Effekt-Pads: je Pad Symbol, Name und Datei, Anhören und Zurücksetzen auf den mitgelieferten Klang.
    fn effects_page(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        card(ui, tr("Effekt-Pads"), |ui| {
            note(ui, tr("Jedes Pad spielt einen mitgelieferten Effekt oder eine eigene Datei (MP3, WAV, FLAC, OGG, M4A). Auslösen mit Klick oder den Tasten 1 bis 8, Rechtsklick auf ein Pad hört es im Kopfhörer vor."));
            ui.add_space(4.0);
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
                    let r = ui.add_sized(vec2(104.0, 22.0), egui::TextEdit::singleline(&mut name).hint_text(hint).char_limit(14));
                    if r.changed() {
                        out.push(SettingsAction::FxName(slot, name));
                    }
                    let file = match &s.file {
                        Some(f) => f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        None => tr("Mitgeliefert").to_owned(),
                    };
                    let shown = ui.add_sized(vec2(116.0, 20.0), egui::Label::new(RichText::new(&file).size(12.5).color(if s.file.is_some() { theme::ink() } else { theme::faint() })).truncate());
                    if let Some(f) = &s.file {
                        shown.on_hover_text(f.display().to_string());
                    }
                    ui.horizontal(|ui| {
                        if action(ui, tr("Datei …"), true).clicked() {
                            out.push(SettingsAction::FxPickFile(slot));
                        }
                        if action(ui, tr("Standard"), s.file.is_some() || !s.name.is_empty() || s.icon.is_some())
                            .on_hover_text(tr("Mitgelieferten Effekt, Namen und Symbol wiederherstellen"))
                            .clicked()
                        {
                            out.push(SettingsAction::FxReset(slot));
                        }
                    });
                    if action(ui, "🎧", true).on_hover_text(tr("Im Kopfhörer anhören")).clicked() {
                        out.push(SettingsAction::FxPreview(slot));
                    }
                    ui.end_row();
                }
            });
            // Auswahl des Symbols für das gewählte Pad
            if let Some(slot) = self.icon_picker {
                let current = c.settings.fx_slot(slot).icon(slot);
                ui.add_space(6.0);
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
            ui.add_space(6.0);
            let changed = (0..fw_engine::FX_SLOTS).any(|i| c.settings.fx_slot(i) != crate::fx::FxSlot::default());
            ui.horizontal(|ui| {
                if self.confirm_fx_reset && changed {
                    ui.label(RichText::new(tr("Alle eigenen Dateien, Namen und Symbole entfernen?")).color(theme::yellow()));
                    if action(ui, tr("Ja, zurücksetzen"), true).clicked() {
                        out.push(SettingsAction::FxResetAll);
                        self.confirm_fx_reset = false;
                        self.icon_picker = None;
                    }
                    if action(ui, tr("Abbrechen"), true).clicked() {
                        self.confirm_fx_reset = false;
                    }
                } else {
                    self.confirm_fx_reset = false;
                    let b = action(ui, tr("Alle Effekte zurücksetzen"), changed);
                    if b.on_hover_text(tr("Alle Pads wieder mit den mitgelieferten Effekten, Namen und Symbolen belegen")).clicked() {
                        self.confirm_fx_reset = true;
                    }
                }
            });
        });
    }

    /// Online-Erkennung und Caches: was beim Lesen der Musiksammlung hilft.
    fn music_page(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        card(ui, tr("Titel online erkennen"), |ui| {
            let r = switch(
                ui,
                tr("„Online suchen“ im Tag-Fenster erlauben"),
                c.settings.online_lookup,
                true,
                tr("Nur auf Knopfdruck. Die Musikdatei bleibt auf deinem Rechner."),
                tr("Nur auf Knopfdruck: Ein akustischer Fingerabdruck und die Länge gehen an AcoustID, ohne Treffer Interpret und Titel an MusicBrainz, für das Cover die Album-Kennung an das Cover Art Archive. Die Musikdatei bleibt auf deinem Rechner."),
            );
            if r.clicked() {
                out.push(SettingsAction::OnlineLookup(!c.settings.online_lookup));
            }
        });

        card(ui, tr("Caches"), |ui| {
            note(ui, tr("Gemerkte Tags, Ordnerlisten und Wellenformen beschleunigen große Sammlungen und Netzlaufwerke. Leeren ist ungefährlich: Alles wird bei Bedarf neu gelesen."));
            use crate::caches::fmt_bytes;
            match c.cache_sizes {
                Some(s) => {
                    let line = trf("Tags {} · Ordnerlisten {} · Wellenformen {} · ermittelte BPM und Tonarten {}", &[&fmt_bytes(s.tags), &fmt_bytes(s.folders), &fmt_bytes(s.waves), &fmt_bytes(s.bpm)]);
                    ui.label(RichText::new(line).size(12.5).color(theme::ink()));
                }
                None => {
                    out.push(SettingsAction::MeasureCaches);
                    ui.label(RichText::new(tr("Wird gemessen …")).size(12.5).color(theme::dim()));
                }
            }
            ui.add_space(2.0);
            let r = switch(ui, tr("Auch ermittelte BPM und Tonarten löschen"), self.clear_bpm, true, "", tr("Die Tempo-Erkennung dauert lange; ohne Häkchen bleiben ermittelte BPM erhalten."));
            if r.clicked() {
                self.clear_bpm = !self.clear_bpm;
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if self.confirm_clear {
                    ui.label(RichText::new(tr("Caches wirklich leeren?")).color(theme::yellow()));
                    if action(ui, tr("Ja, leeren"), true).clicked() {
                        out.push(SettingsAction::ClearCaches { bpm: self.clear_bpm });
                        self.confirm_clear = false;
                    }
                    if action(ui, tr("Abbrechen"), true).clicked() {
                        self.confirm_clear = false;
                    }
                } else if action(ui, tr("Caches leeren …"), true).clicked() {
                    self.confirm_clear = true;
                }
            });
        });
    }

    /// Alles löschen und neu starten, ganz unten unter Allgemein.
    fn reset_card(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        // Rot abgesetzt, damit man es nicht mit „Caches leeren“ verwechselt
        ui.add_space(8.0);
        card_with(ui, tr("Zurücksetzen"), theme::cue(), |ui| {
            note(ui, tr("Löscht alle Einstellungen, Playlists, die Historie, eigene Effekt-Pads und alle Caches. Deine Musikdateien bleiben unberührt. Danach startet FadeWerk DJ neu wie beim ersten Mal."));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let red = crate::hw::Look::Color(theme::cue());
                if self.confirm_reset && !c.playing {
                    ui.label(RichText::new(tr("Wirklich alles löschen? Das lässt sich nicht rückgängig machen.")).color(theme::cue()));
                    if action_look(ui, tr("Ja, zurücksetzen"), red, true).clicked() {
                        out.push(SettingsAction::ResetAll);
                        self.confirm_reset = false;
                    }
                    if action(ui, tr("Abbrechen"), true).clicked() {
                        self.confirm_reset = false;
                    }
                } else {
                    self.confirm_reset = false;
                    let b = action_look(ui, tr("FadeWerk DJ zurücksetzen …"), red, !c.playing).on_disabled_hover_text(tr("Gesperrt, solange ein Player spielt"));
                    if b.clicked() {
                        self.confirm_reset = true;
                    }
                }
            });
        });
    }

    /// Copyright, Lizenz und Fremdbestandteile („Appropriate Legal Notices“ nach GPL-3.0 Abschnitt 0).
    fn about_page(&mut self, ui: &mut egui::Ui, out: &mut Vec<SettingsAction>) {
        card(ui, "FadeWerk DJ", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("FadeWerk DJ").font(crate::fonts::bold(22.0)).color(theme::ink()));
                ui.label(RichText::new(trf("Version {}", &[&env!("CARGO_PKG_VERSION")])).color(theme::dim()));
            });
            ui.label(tr("DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade."));
            ui.add_space(4.0);
            ui.label(RichText::new("Copyright © 2026 Markus Eckert").strong());
            ui.label(tr("FadeWerk DJ ist freie Software: Du kannst es unter den Bedingungen der GNU General Public License, Version 3 oder (nach deiner Wahl) jeder späteren Version, weitergeben und verändern."));
            ui.label(RichText::new(tr("Es wird OHNE JEDE GEWÄHRLEISTUNG bereitgestellt, auch ohne die implizite Gewährleistung der Marktfähigkeit oder Eignung für einen bestimmten Zweck.")).color(theme::dim()));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(tr("Quellcode:"));
                ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
            });
            if let Some(log) = crate::diagnose::log_path() {
                ui.horizontal(|ui| {
                    ui.label(tr("Logdatei:"));
                    if action(ui, crate::browser::reveal_label(), true).clicked() {
                        out.push(SettingsAction::RevealLog);
                    }
                });
                ui.add(egui::Label::new(RichText::new(log.display().to_string()).size(11.5).color(theme::dim())).wrap());
            }
        });

        card(ui, tr("Verwendete Fremdbestandteile"), |ui| {
            note(ui, tr("Schriften Chakra Petch und DSEG (SIL Open Font License 1.1), egui und eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0), rusty-chromaprint (MIT), LAME für MP3-Aufnahmen (LGPL) und weitere Rust-Bibliotheken. Titelerkennung über AcoustID und MusicBrainz (Daten CC0), Cover über das Cover Art Archive. Die vollständige Liste liegt jedem Release als THIRD-PARTY-LICENSES.yml bei."));
        });

        card(ui, tr("Lizenztexte"), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, (name, _)) in LICENSE_TEXTS.iter().enumerate() {
                    let on = self.license_view == Some(i);
                    let font = egui::FontId::proportional(13.0);
                    let w = ui.painter().layout_no_wrap((*name).to_owned(), font.clone(), theme::ink()).size().x + 22.0;
                    let look = if on { crate::hw::Look::Color(theme::select_accent()) } else { crate::hw::Look::Plain };
                    if crate::hw::button(ui, vec2(w, BTN_H), name, font, look, on, true).clicked() {
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
        });
    }

    /// Lautsprecher (mit Puffer), Kopfhörer und andere Geräusche.
    fn audio_cards(&mut self, ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
        let devices = self.devices.get_or_insert_with(audio::list_output_devices).clone();
        let s = c.settings;
        let mut refresh = false;

        card(ui, tr("Lautsprecher (Master)"), |ui| {
            let current = s
                .output_device
                .as_ref()
                .and_then(|id| devices.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
                .unwrap_or_else(|| tr("Systemstandard").to_owned());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("device").selected_text(current).width(300.0).show_ui(ui, |ui| {
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
                if action(ui, tr("Testton"), true).on_hover_text(tr("Spielt einen kurzen Ton über die Lautsprecher (Master)")).clicked() {
                    out.push(SettingsAction::TestTone(true));
                }
                if action(ui, tr("Liste aktualisieren"), true).clicked() {
                    refresh = true;
                }
            });
            if let Some(name) = c.active_device {
                note(ui, &trf("Aktiv: {} · {} Hz · {} Kanäle", &[&name, &c.sample_rate, &c.channels]));
            }
            ui.add_space(4.0);
            let rate = c.sample_rate.max(1) as f32;
            let buf_label = |b: Option<u32>| match b {
                None => tr("Standard").to_owned(),
                Some(n) => trf("{} Samples ≈ {} ms", &[&n, &format!("{:.1}", n as f32 / rate * 1000.0)]),
            };
            field(ui, tr("Puffergröße"), tr("Kleiner Puffer: weniger Verzögerung beim Scratchen, aber mehr Rechenlast. Bei Knacksern größer wählen."), |ui| {
                egui::ComboBox::from_id_salt("buffer").selected_text(buf_label(s.buffer_frames)).width(180.0).show_ui(ui, |ui| {
                    for b in BUFFERS {
                        if ui.selectable_label(s.buffer_frames == b, buf_label(b)).clicked() {
                            out.push(SettingsAction::Buffer(b));
                        }
                    }
                });
            });
        });
        if refresh {
            self.devices = None;
        }

        card(ui, tr("Kopfhörer zum Vorhören (zweiter Ausgang)"), |ui| {
            let head_current = s
                .head_device
                .as_ref()
                .and_then(|id| devices.iter().find(|d| &d.id == id).map(|d| d.name.clone()))
                .unwrap_or_else(|| tr("Kein eigenes Gerät").to_owned());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("head-device").selected_text(head_current).width(300.0).show_ui(ui, |ui| {
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
                if action(ui, tr("Testton"), true).on_hover_text(tr("Spielt einen kurzen Ton auf dem Kopfhörer")).clicked() {
                    out.push(SettingsAction::TestTone(false));
                }
            });
            let text = match c.active_head {
                Some(name) => trf("{} spielt zusätzlich zu den Lautsprechern. Dort hörst du, was du vorhörst (VORHÖREN, 🎧).", &[&name]),
                None if s.head_device.is_some() => tr("Das gewählte Gerät ist nicht aktiv (beim Start nicht verbunden). Zum Einschalten in der Liste erneut wählen.").to_owned(),
                None => tr("Ohne eigenes Gerät läuft der Kopfhörer über „Split L/R“ oder „Ausg. 3/4“ im Mixer.").to_owned(),
            };
            note(ui, &text);
            // Häufigste Überraschung: CUE/MST nicht auf CUE, dann läuft die Master-Musik auch im Kopfhörer
            if c.active_head.is_some() && s.head_mix > 0.01 {
                let warn = trf("CUE/MST im Mixer steht auf {} % Master: Der Kopfhörer spielt auch die Musik der Lautsprecher. Nur Vorhören: unter HÖRER ganz nach links auf CUE.", &[&format!("{:.0}", s.head_mix * 100.0)]);
                ui.add(egui::Label::new(RichText::new(warn).size(12.0).color(theme::yellow())).wrap());
            }
            if c.active_head.is_some() {
                note(ui, tr("Zwei Geräte laufen nie exakt gleich schnell. FadeWerk DJ gleicht das automatisch aus; der Kopfhörer liegt dabei etwa 20 ms hinter den Lautsprechern."));
            }
        });

        card(ui, tr("Andere Geräusche"), |ui| {
            let (mute_label, mute_info) = if cfg!(target_os = "macos") {
                (tr("Systemtöne stumm schalten, solange FadeWerk DJ läuft"), tr("Hinweistöne des Systems und der Programme sind aus und kommen beim Beenden zurück."))
            } else {
                (
                    tr("Systemtöne und andere Programme stumm schalten, solange FadeWerk DJ läuft"),
                    tr("Auch Browser, Musik- und Chatprogramme sind stumm (wie im Lautstärkemixer) und kommen beim Beenden zurück. Neu gestartete Programme werden nach etwa 2 Sekunden stumm."),
                )
            };
            if switch(ui, mute_label, s.mute_others, true, "", mute_info).clicked() {
                out.push(SettingsAction::MuteOthers(!s.mute_others));
            }
            if cfg!(target_os = "macos") {
                let info = tr("Nur FadeWerk DJ spielt dann über das Master-Gerät, alle anderen Programme bleiben darauf stumm, auch Videocalls. Das Kopfhörergerät ist davon nicht betroffen.");
                if switch(ui, tr("Lautsprecher exklusiv belegen (andere Programme stumm)"), s.exclusive_output, true, "", info).clicked() {
                    out.push(SettingsAction::Exclusive(!s.exclusive_output));
                }
            }
            ui.add(
                egui::Label::new(
                    RichText::new(tr("Mitteilungen (Mail, Kalender, Chat) blendet der Fokus „Nicht stören“ des Systems aus; den kann FadeWerk DJ nicht selbst einschalten."))
                        .size(11.5)
                        .color(theme::faint()),
                )
                .wrap(),
            );
        });
    }
}

/// Eintrag der Seitenleiste: Symbol und Name, der gewählte mit Leuchtstreifen links und hellerer Fläche.
fn nav_entry(ui: &mut egui::Ui, page: Page, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(NAV_W, NAV_ROW_H), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 5.0, theme::row_selected());
        let bar = Rect::from_min_size(rect.left_top() + vec2(0.0, 6.0), vec2(3.0, rect.height() - 12.0));
        p.rect_filled(bar, 1.5, theme::select_accent());
    } else if resp.hovered() {
        p.rect_filled(rect, 5.0, theme::row_hover());
    }
    let color = if on { theme::ink() } else { theme::dim() };
    let icon = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(18.0, 18.0));
    paint_page_icon(p, icon, page, color);
    p.text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, page.name(), egui::FontId::proportional(14.5), color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Selbst gezeichnete Symbole der Seitenleiste im Linienstil der Pad-Symbole.
fn paint_page_icon(p: &egui::Painter, r: Rect, page: Page, color: Color32) {
    let st = Stroke::new(1.6, color);
    let c = r.center();
    let at = |x: f32, y: f32| -> Pos2 { pos2(r.left() + x * r.width(), r.top() + y * r.height()) };
    match page {
        // Drei Schieberegler
        Page::General => {
            for (k, knob) in [(0.2, 0.3), (0.5, 0.7), (0.8, 0.45)] {
                p.line_segment([at(k, 0.05), at(k, 0.95)], st);
                p.rect_filled(Rect::from_center_size(at(k, knob), vec2(6.0, 3.5)), 1.0, color);
            }
        }
        // Lautsprecher mit Schallbögen
        Page::Audio => {
            let body = vec![at(0.05, 0.35), at(0.3, 0.35), at(0.55, 0.1), at(0.55, 0.9), at(0.3, 0.65), at(0.05, 0.65)];
            p.add(egui::Shape::closed_line(body, st));
            for rr in [0.28, 0.45] {
                let pts: Vec<Pos2> = (0..=8).map(|i| {
                    let a = -0.8 + 1.6 * i as f32 / 8.0;
                    at(0.5, 0.5) + vec2(a.cos(), a.sin()) * rr * r.width()
                }).collect();
                p.add(egui::Shape::line(pts, st));
            }
        }
        // Note
        Page::Music => {
            p.circle_filled(at(0.3, 0.78), r.width() * 0.16, color);
            p.line_segment([at(0.44, 0.78), at(0.44, 0.08)], st);
            p.line_segment([at(0.44, 0.08), at(0.85, 0.25)], st);
            p.line_segment([at(0.85, 0.25), at(0.85, 0.42)], st);
        }
        // Vier Pads
        Page::Effects => {
            for (x, y) in [(0.0, 0.0), (0.55, 0.0), (0.0, 0.55), (0.55, 0.55)] {
                p.rect_stroke(Rect::from_min_size(at(x, y), r.size() * 0.45), 2.0, st, egui::StrokeKind::Inside);
            }
        }
        // Bildschirm mit Fuß
        Page::Display => {
            p.rect_stroke(Rect::from_min_max(at(0.0, 0.08), at(1.0, 0.72)), 2.0, st, egui::StrokeKind::Inside);
            p.line_segment([at(0.5, 0.72), at(0.5, 0.9)], st);
            p.line_segment([at(0.28, 0.92), at(0.72, 0.92)], st);
        }
        // Info
        Page::About => {
            p.circle_stroke(c, r.width() * 0.46, st);
            p.circle_filled(at(0.5, 0.28), 1.3, color);
            p.line_segment([at(0.5, 0.42), at(0.5, 0.76)], Stroke::new(1.9, color));
        }
    }
}

/// Abschnitt als abgesetzte Fläche, oben der Titel in Großbuchstaben (wie MIXER und ORDNER).
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card_with(ui, title, theme::dim(), add)
}

fn card_with<R>(ui: &mut egui::Ui, title: &str, title_color: Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let fill = theme::mix(theme::panel(), theme::ground(), 0.5);
    let r = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, theme::line()))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(title.to_uppercase()).font(crate::fonts::bold(11.5)).extra_letter_spacing(1.0).color(title_color));
            ui.add_space(2.0);
            add(ui)
        })
        .inner;
    ui.add_space(CARD_GAP);
    r
}

/// Kleiner grauer Hinweis, umbrechend.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::dim())).wrap());
}

/// ⓘ mit der ausführlichen Erklärung als Tooltip.
fn info_mark(ui: &mut egui::Ui, text: &str) {
    let (rect, resp) = ui.allocate_exact_size(vec2(15.0, 15.0), Sense::hover());
    let color = if resp.hovered() { theme::ink() } else { theme::faint() };
    let p = ui.painter();
    p.circle_stroke(rect.center(), 6.5, Stroke::new(1.2, color));
    p.circle_filled(rect.center() + vec2(0.0, -3.2), 1.0, color);
    p.line_segment([rect.center() + vec2(0.0, -0.8), rect.center() + vec2(0.0, 3.6)], Stroke::new(1.5, color));
    resp.on_hover_ui(|ui| {
        ui.set_max_width(320.0);
        ui.label(text);
    });
}

/// Schalter-Zeile: links der Text (darunter `sub`, falls nicht leer, und ⓘ mit `info`), rechts ein LED-Taster
/// AN/AUS wie an den Playern. Auch ein Klick auf den Text schaltet um. `clicked()` = umschalten.
fn switch(ui: &mut egui::Ui, label: &str, on: bool, enabled: bool, sub: &str, info: &str) -> egui::Response {
    let w = ui.available_width();
    let text_w = w - SWITCH.x - 12.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let text = ui
            .allocate_ui_with_layout(vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let color = if enabled { theme::ink() } else { theme::faint() };
                    ui.add(egui::Label::new(RichText::new(label).size(14.0).color(color)).selectable(false));
                    if !info.is_empty() {
                        info_mark(ui, info);
                    }
                });
                if !sub.is_empty() {
                    ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(theme::dim())).wrap().selectable(false));
                }
            })
            .response;
        let led = if on { tr("An") } else { tr("Aus") }.to_uppercase();
        let b = crate::hw::button(ui, SWITCH, &led, crate::fonts::bold(10.5), crate::hw::Look::Color(theme::play()), on, enabled);
        let text_click = enabled && ui.interact(text.rect, ui.id().with(("schalter", label)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        if text_click {
            let mut b = b;
            b.flags.insert(egui::response::Flags::CLICKED);
            b
        } else {
            b
        }
    })
    .inner
}

/// Beschriftung in fester Breite (mit ⓘ, falls `info` nicht leer), daneben die Bedienelemente.
fn field(ui: &mut egui::Ui, label: &str, info: &str, add: impl FnOnce(&mut egui::Ui)) {
    let row = ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(FIELD_W, BTN_H), Sense::hover());
        add(ui);
        rect
    });
    // Erst nach den Bedienelementen beschriften: so steht der Text auf der Mitte der fertigen Zeile
    let rect = row.inner;
    let y = row.response.rect.center().y;
    let g = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(14.0), theme::ink());
    let w = g.size().x;
    ui.painter().galley(pos2(rect.left(), y - g.size().y / 2.0), g, theme::ink());
    if !info.is_empty() {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(pos2(rect.left() + w + 6.0, y - 7.5), vec2(15.0, 15.0))));
        info_mark(&mut child, info);
    }
}

/// Hardware-Taste in Textbreite (wie in den Kopfzeilen der App).
fn action(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    action_look(ui, label, crate::hw::Look::Plain, enabled)
}

fn action_look(ui: &mut egui::Ui, label: &str, look: crate::hw::Look, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::ink()).size().x + 22.0;
    let colored = matches!(look, crate::hw::Look::Color(_));
    crate::hw::button(ui, vec2(w.max(BTN_H + 6.0), BTN_H), label, font, look, colored, enabled)
}

/// Reihe von Segment-Tasten, die gewählte leuchtet. Liefert den angeklickten Wert; Einträge mit `false` sind
/// gesperrt (etwa „Eigene“, das sich nur aus den Haken ergibt).
fn segmented<T: Copy + PartialEq>(ui: &mut egui::Ui, current: T, options: &[(T, &str, bool)]) -> Option<T> {
    let font = egui::FontId::proportional(13.0);
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for &(value, label, enabled) in options {
            let on = value == current;
            let w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), theme::ink()).size().x + 24.0;
            let look = if on { crate::hw::Look::Color(theme::select_accent()) } else { crate::hw::Look::Plain };
            if crate::hw::button(ui, vec2(w, BTN_H), label, font.clone(), look, on, enabled || on).clicked() && !on {
                picked = Some(value);
            }
        }
    });
    picked
}

/// Sprache, Ruhezustand, Updates.
fn general_page(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    card(ui, tr("Sprache der Oberfläche"), |ui| {
        let options: Vec<(Lang, &str, bool)> = Lang::ALL.iter().map(|&l| (l, l.native_name(), true)).collect();
        if let Some(l) = segmented(ui, c.settings.language, &options) {
            out.push(SettingsAction::Language(l));
        }
    });

    card(ui, tr("Ruhezustand"), |ui| {
        let info = tr("Bildschirm und Rechner bleiben an, solange ein Player oder das Vorhören läuft. Danach gilt wieder die Einstellung des Systems.");
        if switch(ui, tr("Ruhezustand verhindern, solange Musik spielt"), c.settings.keep_awake, true, "", info).clicked() {
            out.push(SettingsAction::KeepAwake(!c.settings.keep_awake));
        }
    });

    card(ui, tr("Updates"), |ui| {
        let check = c.settings.update_check.unwrap_or(false);
        let info = tr("Gesendet wird nur die Frage nach der neuesten Version, keine Daten über Titel oder Ordner. GitHub sieht dabei wie bei jedem Webseitenaufruf deine IP-Adresse.");
        if switch(ui, tr("Beim Start bei GitHub nach einer neuen Version suchen"), check, true, "", info).clicked() {
            out.push(SettingsAction::UpdateCheck(!check));
        }
        ui.horizontal(|ui| {
            if action(ui, tr("Jetzt nach Updates suchen"), true).clicked() {
                out.push(SettingsAction::CheckUpdateNow);
            }
            if crate::self_update::asset_suffix().is_some() {
                info_mark(ui, tr("Eine neue Version installiert sich per Knopf selbst; die App startet danach neu. Auf dem Mac nur, wenn sie mit demselben Zertifikat signiert ist."));
            }
        });
    });
}

/// Mix aufnehmen (Format, Ordner) und die Historie der gespielten Titel.
fn recording_cards(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    card(ui, tr("Mix aufnehmen"), |ui| {
        note(ui, tr("Die Taste REC neben MASTER nimmt den Mix auf, wie er über Master klingt (vor der Master-Lautstärke)."));
        ui.add_space(2.0);
        field(ui, tr("Format"), "", |ui| {
            use crate::recorder::RecordFormat;
            let options = [(RecordFormat::Wav, tr("WAV (verlustfrei, 24 Bit)"), true), (RecordFormat::Mp3, tr("MP3 (320 kbit/s)"), true)];
            if let Some(f) = segmented(ui, c.settings.record_format, &options) {
                out.push(SettingsAction::RecordFormat(f));
            }
        });
        let dir = c.settings.record_dir.clone().unwrap_or_else(crate::recorder::default_dir);
        // Im Benutzerordner mit „~“ abkürzen, damit der Pfad in eine Zeile passt
        let shown = match dirs::home_dir().and_then(|h| dir.strip_prefix(&h).ok().map(|r| r.to_path_buf())) {
            Some(rest) => format!("~/{}", rest.display()),
            None => dir.display().to_string(),
        };
        field(ui, tr("Ordner"), "", |ui| {
            ui.add(egui::Label::new(RichText::new(shown).size(12.5).color(theme::ink())).truncate());
        });
        ui.horizontal(|ui| {
            ui.add_space(FIELD_W + ui.spacing().item_spacing.x);
            if action(ui, tr("Ordner wählen …"), true).clicked() {
                out.push(SettingsAction::RecordPickDir);
            }
            if c.settings.record_dir.is_some() && action(ui, tr("Standardordner"), true).clicked() {
                out.push(SettingsAction::RecordDirDefault);
            }
            if action(ui, crate::browser::reveal_label(), true).clicked() {
                out.push(SettingsAction::RecordRevealDir);
            }
        });
    });

    card(ui, tr("Historie"), |ui| {
        let info = tr("Ein Titel zählt, sobald er 30 Sekunden über Master zu hören war. Zu finden im Ordnerbrowser unter „Historie“.");
        if switch(ui, tr("Gespielte Titel aufzeichnen"), c.settings.record_history, true, "", info).clicked() {
            out.push(SettingsAction::RecordHistory(!c.settings.record_history));
        }
    });
}

/// Skins, Stahl, kompakte Ansicht und Zoom.
fn look_page(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    card(ui, tr("Skin"), |ui| {
        let tile_w = (ui.available_width() - 10.0) / 2.0;
        egui::Grid::new("skins").num_columns(2).spacing(vec2(10.0, 10.0)).show(ui, |ui| {
            for (i, s) in SKINS.iter().enumerate() {
                let selected = c.settings.skin == s.id;
                let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, 104.0), Sense::click());
                let p = ui.painter();
                // Vorschau in den Farben des jeweiligen Skins
                p.rect_filled(rect, CornerRadius::same(6), s.ground);
                let border = if selected { Stroke::new(2.5, theme::select_accent()) } else { Stroke::new(1.0, theme::line()) };
                p.rect_stroke(rect, CornerRadius::same(6), border, egui::StrokeKind::Inside);
                let inner = rect.shrink(10.0);
                let panel = egui::Rect::from_min_size(inner.min, vec2(inner.width(), 32.0));
                p.rect_filled(panel, CornerRadius::same(4), s.panel);
                if theme::brushed() {
                    crate::hw::brushed_overlay(p, panel, 4.0, s.dark);
                } else if s.finish == theme::Finish::Alu && !theme::lite().flat {
                    crate::hw::grain_overlay(p, panel, 4.0, s.dark);
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
        ui.add_space(4.0);
        // Ohne Effekte (Leistung) gibt es keinen Stahl
        let flat = c.settings.lite.flat;
        let r = switch(ui, tr("Gebürsteter Stahl"), c.settings.brushed_steel && !flat, !flat, tr("Gehäuseflächen wirken wie gebürstetes Metall in den Farben des Skins."), "");
        if r.on_disabled_hover_text(tr("Unter Leistung ist „Ohne Schatten und Leuchten“ eingeschaltet.")).clicked() {
            out.push(SettingsAction::Brushed(!c.settings.brushed_steel));
        }
    });

    card(ui, tr("Ansicht"), |ui| {
        let info = tr("Für kleine Bildschirme wie Netbooks: flache Player ohne Teller, die Ordner gleiten bei Bedarf von links herein (⌘B bzw. Strg+B). Automatisch schaltet sie nur auf kleinen Bildschirmen ein.");
        field(ui, tr("Kompakte Ansicht"), info, |ui| {
            let options = [(None, tr("Automatisch"), true), (Some(true), tr("An"), true), (Some(false), tr("Aus"), true)];
            if let Some(v) = segmented(ui, c.settings.compact, &options) {
                out.push(SettingsAction::Compact(v));
            }
        });
        let tip = if cfg!(target_os = "macos") { tr("Ganze Oberfläche kleiner oder größer, auch mit ⌘ + und ⌘ − (⌘ 0 = 100 %).") } else { tr("Ganze Oberfläche kleiner oder größer, auch mit Strg + und Strg − (Strg 0 = 100 %).") };
        field(ui, tr("Zoom"), tip, |ui| {
            let z = c.settings.ui_zoom;
            let font = egui::FontId::proportional(15.0);
            if crate::hw::button(ui, vec2(32.0, BTN_H), "−", font.clone(), crate::hw::Look::Plain, false, z > 0.71).clicked() {
                out.push(SettingsAction::Zoom(z - 0.1));
            }
            ui.label(RichText::new(format!("{:.0} %", z * 100.0)).font(crate::fonts::lcd(15.0)).color(theme::ink()));
            if crate::hw::button(ui, vec2(32.0, BTN_H), "+", font.clone(), crate::hw::Look::Plain, false, z < 1.49).clicked() {
                out.push(SettingsAction::Zoom(z + 0.1));
            }
            if action(ui, "100 %", (z - 1.0).abs() > 0.01).clicked() {
                out.push(SettingsAction::Zoom(1.0));
            }
        });
    });
}

/// Leistung: Voreinstellung (volle Optik, sparsam, eigene) und die einzelnen Sparmaßnahmen zum An- und Abwählen.
fn performance_page(ui: &mut egui::Ui, c: &SettingsCtx<'_>, out: &mut Vec<SettingsAction>) {
    use theme::Lite;
    let lite = c.settings.lite;
    card(ui, tr("Voreinstellung"), |ui| {
        note(ui, tr("Für ältere oder langsame Rechner. Jeder Punkt spart Rechenzeit, der Ton bleibt gleich."));
        // „Eigene“ ergibt sich aus den Schaltern darunter, anklicken lässt es sich nicht
        let custom = lite != Lite::NONE && lite != Lite::ALL;
        let current = if custom { None } else { Some(lite) };
        let options = [(Some(Lite::NONE), tr("Volle Optik"), true), (Some(Lite::ALL), tr("Sparsam"), true), (None, tr("Eigene"), false)];
        if let Some(Some(p)) = segmented(ui, current, &options) {
            out.push(SettingsAction::Lite(p));
        }
    });
    let part = |ui: &mut egui::Ui, out: &mut Vec<SettingsAction>, label: &str, sub: &str, pick: fn(&mut Lite) -> &mut bool| {
        let mut parts = lite;
        let on = *pick(&mut parts);
        if switch(ui, label, on, true, sub, "").clicked() {
            *pick(&mut parts) = !on;
            out.push(SettingsAction::Lite(parts));
        }
    };
    // Zwei Spalten: links was gezeichnet wird, rechts Rechenzeit und Grafik
    let gap = CARD_GAP;
    let col_w = (ui.available_width() - gap) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(vec2(col_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
            ui.set_width(col_w);
            card(ui, tr("Weniger zeichnen"), |ui| {
                part(ui, out, tr("Einfache Formen"), tr("Eckige Flächen, Kreise aus weniger Punkten. Spart am meisten."), |l| &mut l.shapes);
                part(ui, out, tr("Kantenglättung aus"), tr("Halbiert die Zeichenarbeit, Kanten wirken gezackt."), |l| &mut l.no_feathering);
                part(ui, out, tr("Ohne Schatten und Leuchten"), tr("Auch ohne Verläufe, Stahl und Aluminium."), |l| &mut l.flat);
                part(ui, out, tr("Grobe Wellenformen"), tr("Halb so viele Spalten, kaum zu sehen."), |l| &mut l.coarse_waves);
                part(ui, out, tr("Pads ohne Symbole"), tr("Nur Namen, Pegel als Balken."), |l| &mut l.simple_pads);
                // Die kompakte Ansicht hat keine Teller: Schalter aus und gesperrt, die Einstellung selbst bleibt
                let platters = c.settings.platters && !c.compact;
                let r = switch(ui, tr("Plattenspieler zeigen"), platters, !c.compact, tr("Ohne Teller mehr Platz, Scratchen geht in der Wellenform."), "");
                if r.on_disabled_hover_text(tr("In der kompakten Ansicht gibt es keine Plattenteller.")).clicked() {
                    out.push(SettingsAction::Platters(!platters));
                }
            });
        });
        ui.allocate_ui_with_layout(vec2(col_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
            ui.set_width(col_w);
            card(ui, tr("Weniger rechnen"), |ui| {
                part(ui, out, tr("Frequenzbänder aus"), tr("MAIN zeigt nur den Pegel, entlastet den Audio-Thread."), |l| &mut l.no_bands);
                part(ui, out, tr("Bildrate begrenzen"), tr("Höchstens 30 Bilder pro Sekunde, im Stillstand eines, ohne Animationen."), |l| &mut l.frame_limit);
            });
            card(ui, tr("Grafik"), |ui| {
                let sub = tr("Falls das Fenster nicht zuverlässig startet (etwa in virtuellen Maschinen). Wirkt nach einem Neustart.");
                if switch(ui, tr("Grafik über OpenGL"), c.settings.opengl, true, sub, "").clicked() {
                    out.push(SettingsAction::OpenGl(!c.settings.opengl));
                }
            });
        });
    });
}
