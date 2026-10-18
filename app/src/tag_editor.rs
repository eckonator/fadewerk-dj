// SPDX-License-Identifier: GPL-3.0-or-later
//! Fenster „Titelinfo & Tags“: zeigt Dateiangaben und lässt die Tags eines Titels bearbeiten.
//! Gespeichert wird in einem eigenen Thread, weil das Umschreiben großer Dateien dauern kann.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, Align2, Frame, Margin, RichText, vec2};
use fw_library::{FileInfo, TagFields, TrackMeta, read_cover, read_tags, write_tags_with_cover};

use crate::online_tags::{self, Candidate, Details, Progress};

use crate::deck_ui::fmt_time;
use crate::i18n::{Lang, lang, tr};
use crate::{fonts, theme};

pub enum TagEditorEvent {
    Close,
    /// Tags wurden in die Datei geschrieben; Anzeigen neu einlesen.
    Saved(PathBuf),
    Reveal(PathBuf),
    /// Der Nutzer hat der Online-Erkennung zugestimmt (Einstellung merken).
    AllowOnline,
}

/// Felder, die die Online-Erkennung vorschlagen kann (Reihenfolge der Vergleichstabelle).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    Track,
    Genre,
    Cover,
}

const FIELDS: [Field; 8] = [Field::Title, Field::Artist, Field::Album, Field::AlbumArtist, Field::Year, Field::Track, Field::Genre, Field::Cover];

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Title => tr("Titel"),
            Field::Artist => tr("Interpret"),
            Field::Album => tr("Album"),
            Field::AlbumArtist => tr("Album-Interpret"),
            Field::Year => tr("Jahr"),
            Field::Track => tr("Track"),
            Field::Genre => tr("Genre"),
            Field::Cover => tr("Cover"),
        }
    }

    fn slot(self, f: &mut TagFields) -> Option<&mut String> {
        Some(match self {
            Field::Title => &mut f.title,
            Field::Artist => &mut f.artist,
            Field::Album => &mut f.album,
            Field::AlbumArtist => &mut f.album_artist,
            Field::Year => &mut f.year,
            Field::Track => &mut f.track,
            Field::Genre => &mut f.genre,
            Field::Cover => return None,
        })
    }
}

/// Zustand der Online-Erkennung im Fenster.
#[derive(Default)]
struct Online {
    /// Wartet auf die Zustimmung des Nutzers.
    asking: bool,
    search: Option<Receiver<Progress>>,
    status: &'static str,
    result: Option<Result<Vec<Candidate>, &'static str>>,
    chosen: usize,
    album: usize,
    /// Für welchen Vorschlag/welches Album Genre und Cover geladen sind bzw. werden.
    details_for: Option<(usize, usize)>,
    details_rx: Option<Receiver<Details>>,
    details: Details,
    cover_tex: Option<egui::TextureHandle>,
    /// Häkchen je Feld (`FIELDS`), neu gesetzt bei jedem Wechsel des Vorschlags.
    take: [bool; FIELDS.len()],
    take_for: Option<(usize, usize, bool)>,
}

pub struct TagEditor {
    path: PathBuf,
    fields: TagFields,
    original: TagFields,
    info: Option<FileInfo>,
    error: Option<String>,
    saving: Option<Receiver<Result<(), String>>>,
    online: Option<Online>,
    /// Cover aus der Online-Erkennung, wird beim Speichern geschrieben.
    new_cover: Option<Vec<u8>>,
    /// Vorhandenes Cover als Bild, einmal gelesen, sobald die Erkennung es zum Vergleich braucht.
    old_cover: Option<Option<egui::TextureHandle>>,
}

impl TagEditor {
    pub fn open(path: PathBuf) -> Self {
        let (fields, info, error) = match read_tags(&path) {
            Ok((f, i)) => (f, Some(i), None),
            Err(e) => (TagFields::default(), None, Some(format!("{e:#}"))),
        };
        Self { path, original: fields.clone(), fields, info, error, saving: None, online: None, new_cover: None, old_cover: None }
    }

    pub fn start_search(&mut self, ctx: &egui::Context) {
        let mut artist = self.fields.artist.trim().to_owned();
        let mut title = self.fields.title.trim().to_owned();
        if title.is_empty() {
            let guess = TrackMeta::from_filename(&self.path);
            (artist, title) = (guess.artist, guess.title);
        }
        self.online = Some(Online {
            search: Some(online_tags::search(self.path.clone(), artist, title, ctx.clone())),
            status: "Berechnet Fingerabdruck …",
            ..Default::default()
        });
    }

    /// Holt neue Nachrichten der Erkennung ab und lädt Genre/Cover, sobald ein Album gewählt ist.
    fn poll_online(&mut self, ctx: &egui::Context) {
        let Some(on) = self.online.as_mut() else { return };
        if let Some(rx) = &on.search {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Progress::Status(s) => on.status = s,
                    Progress::Done(r) => {
                        on.result = Some(r);
                        on.search = None;
                        break;
                    }
                }
            }
        }
        if let Some(rx) = &on.details_rx
            && let Ok(d) = rx.try_recv()
        {
            on.cover_tex = d.cover.as_deref().and_then(|b| texture(ctx, "online-cover", b));
            on.details = d;
            on.details_rx = None;
        }
        let Some(Ok(list)) = &on.result else { return };
        let Some(c) = list.get(on.chosen) else { return };
        let Some(album) = c.albums.get(on.album) else { return };
        if on.details_for != Some((on.chosen, on.album)) {
            on.details_for = Some((on.chosen, on.album));
            on.details = Details::default();
            on.cover_tex = None;
            on.details_rx = Some(online_tags::details(album.clone(), c.artist_ids.clone(), ctx.clone()));
        }
        if self.old_cover.is_none() {
            self.old_cover = Some(read_cover(&self.path).map(|c| {
                let img = egui::ColorImage::from_rgba_unmultiplied([c.size as usize; 2], &c.rgba);
                ctx.load_texture("old-cover", img, egui::TextureOptions::LINEAR)
            }));
        }
    }

    /// Vorgeschlagener Wert eines Feldes (Cover: leer = keins, „1“ = vorhanden).
    fn proposal(on: &Online, field: Field) -> Option<String> {
        let c = on.result.as_ref()?.as_ref().ok()?.get(on.chosen)?;
        let album = c.albums.get(on.album);
        Some(match field {
            Field::Title => c.title.clone(),
            Field::Artist => c.artist.clone(),
            Field::Album => album?.title.clone(),
            Field::AlbumArtist => album?.album_artist.clone(),
            Field::Year => c.year.clone(),
            Field::Track => album?.track.clone(),
            Field::Genre => on.details.genre.clone(),
            Field::Cover => if on.details.cover.is_some() { "1".into() } else { String::new() },
        })
    }

    /// Übernimmt die angehakten Vorschläge in die Eingabefelder; geschrieben wird erst mit „Speichern“.
    fn apply_online(&mut self) {
        let Some(on) = self.online.take() else { return };
        for (i, field) in FIELDS.into_iter().enumerate() {
            if !on.take[i] {
                continue;
            }
            let Some(value) = Self::proposal(&on, field) else { continue };
            match field.slot(&mut self.fields) {
                Some(slot) => *slot = value,
                None => self.new_cover = on.details.cover.clone(),
            }
        }
    }

    fn save(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (path, fields, cover) = (self.path.clone(), self.fields.clone(), self.new_cover.clone());
        std::thread::Builder::new()
            .name("fw-tag-write".into())
            .spawn(move || {
                let _ = tx.send(write_tags_with_cover(&path, &fields, cover.as_deref()).map_err(|e| format!("{e:#}")));
            })
            .ok();
        self.saving = Some(rx);
        self.error = None;
    }

    /// `online_allowed`: der Nutzer hat der Online-Erkennung schon zugestimmt.
    pub fn show(&mut self, ctx: &egui::Context, online_allowed: bool) -> Option<TagEditorEvent> {
        let mut event = None;
        self.poll_online(ctx);
        if let Some(rx) = &self.saving {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.saving = None;
                    self.original = self.fields.clone();
                    self.new_cover = None;
                    self.old_cover = None;
                    event = Some(TagEditorEvent::Saved(self.path.clone()));
                }
                Ok(Err(e)) => {
                    self.saving = None;
                    self.error = Some(e);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
                Err(TryRecvError::Disconnected) => self.saving = None,
            }
        }
        let changed = self.fields != self.original || self.new_cover.is_some();
        let busy = self.saving.is_some();
        let mut open = true;
        egui::Window::new(tr("Titelinfo & Tags"))
            .id(egui::Id::new("tag-editor"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .frame(Frame::window(&ctx.global_style()).fill(theme::panel()).inner_margin(Margin::same(16)))
            .show(ctx, |ui| {
                ui.set_width(480.0);
                let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(name).font(fonts::bold(16.0)));
                ui.add(egui::Label::new(RichText::new(self.path.display().to_string()).monospace().size(11.0).color(theme::dim())).truncate());
                if let Some(info) = &self.info {
                    ui.label(RichText::new(describe(info)).size(12.5).color(theme::dim()));
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                let readonly = self.info.is_none() || busy;
                egui::Grid::new("tag-felder").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
                    let f = &mut self.fields;
                    for (label, value) in [
                        ("Titel", &mut f.title),
                        ("Interpret", &mut f.artist),
                        ("Album", &mut f.album),
                        ("Album-Interpret", &mut f.album_artist),
                        ("Genre", &mut f.genre),
                        ("Jahr", &mut f.year),
                        ("Track", &mut f.track),
                        ("BPM", &mut f.bpm),
                    ] {
                        ui.label(RichText::new(tr(label)).color(theme::dim()));
                        ui.add_enabled(!readonly, egui::TextEdit::singleline(value).desired_width(360.0));
                        ui.end_row();
                    }
                    ui.label(RichText::new(tr("Kommentar")).color(theme::dim()));
                    ui.add_enabled(!readonly, egui::TextEdit::multiline(&mut f.comment).desired_rows(2).desired_width(360.0));
                    ui.end_row();
                });

                if self.new_cover.is_some() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(tr("Neues Cover wird beim Speichern in die Datei geschrieben.")).size(12.5).color(theme::dim()));
                }
                if self.online.is_some() {
                    ui.add_space(6.0);
                    ui.separator();
                    if let Some(e) = self.online_ui(ui, ctx) {
                        event = Some(e);
                    }
                }

                if let Some(e) = &self.error {
                    ui.add_space(4.0);
                    ui.colored_label(theme::cue(), e);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let save = ui.add_enabled(changed && !readonly, egui::Button::new(RichText::new(tr("Speichern")).strong()));
                    if save.on_hover_text(tr("Schreibt die Tags direkt in die Musikdatei")).clicked() {
                        self.save();
                    }
                    if ui.add_enabled(changed && !busy, egui::Button::new(tr("Zurücksetzen"))).clicked() {
                        self.fields = self.original.clone();
                        self.new_cover = None;
                    }
                    let searching = self.online.as_ref().is_some_and(|o| o.search.is_some());
                    let online = ui
                        .add_enabled(!readonly && !searching, egui::Button::new(tr("Online suchen")))
                        .on_hover_text(tr("Erkennt den Titel am Klang (AcoustID) und schlägt Tags aus MusicBrainz vor"));
                    if online.clicked() {
                        if online_allowed {
                            self.start_search(ctx);
                        } else {
                            self.online = Some(Online { asking: true, ..Default::default() });
                        }
                    }
                    if busy {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new(tr("Speichert …")).color(theme::dim()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr("Schließen")).clicked() {
                            event = Some(TagEditorEvent::Close);
                        }
                        if ui.button(crate::browser::reveal_label()).clicked() {
                            event = Some(TagEditorEvent::Reveal(self.path.clone()));
                        }
                    });
                });
            });
        if !open {
            event = Some(TagEditorEvent::Close);
        }
        event
    }

    /// Bereich der Online-Erkennung unter den Feldern: Zustimmung, Fortschritt, Vorschläge und Vergleich.
    fn online_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<TagEditorEvent> {
        let mut event = None;
        let mut close = false;
        let mut apply = false;
        let mut search = false;
        let current = self.fields.clone();
        let has_new_cover = self.new_cover.is_some();
        let old_cover = self.old_cover.clone().flatten();
        let on = self.online.as_mut()?;
        ui.label(RichText::new(tr("Online erkennen")).font(fonts::bold(14.0)));
        if on.asking {
            ui.label(RichText::new(tr("FadeWerk DJ berechnet aus dem Titel einen akustischen Fingerabdruck und schickt ihn mit der Länge an AcoustID; ohne Treffer gehen Interpret und Titel an MusicBrainz. Die Musikdatei selbst bleibt auf deinem Rechner. Die Dienste sehen dabei deine IP-Adresse.")).size(12.5).color(theme::dim()));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new(tr("Erlauben und suchen")).strong()).clicked() {
                    event = Some(TagEditorEvent::AllowOnline);
                    search = true;
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    close = true;
                }
            });
        } else if on.search.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(tr(on.status)).color(theme::dim()));
            });
        } else {
            match &on.result {
                Some(Err(e)) => {
                    ui.colored_label(theme::cue(), tr(e));
                }
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(tr("Nichts gefunden. Remixes, Edits und seltene Aufnahmen kennen die Dienste oft nicht."));
                }
                Some(Ok(list)) => {
                    let list = list.clone();
                    egui::ScrollArea::vertical().id_salt("online-vorschlaege").max_height(110.0).min_scrolled_height(0.0).show(ui, |ui| {
                        for (i, c) in list.iter().enumerate() {
                            let source = if c.by_sound { tr("Klang") } else { tr("Text") };
                            let text = format!("{} – {}   {} %  ·  {}", c.artist, c.title, (c.score * 100.0).round(), source);
                            if ui.add(egui::Button::selectable(on.chosen == i, text).truncate()).clicked() && on.chosen != i {
                                on.chosen = i;
                                on.album = 0;
                            }
                        }
                    });
                    let c = &list[on.chosen.min(list.len() - 1)];
                    if !c.albums.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("Album")).color(theme::dim()));
                            let name = |a: &online_tags::AlbumChoice| {
                                let mut s = format!("{}  ({}", a.title, kind_name(&a.kind));
                                if !a.year.is_empty() {
                                    s += &format!(", {}", a.year);
                                }
                                s + ")"
                            };
                            let selected = c.albums.get(on.album).map(name).unwrap_or_default();
                            egui::ComboBox::from_id_salt("online-album").selected_text(selected).width(380.0).truncate().show_ui(ui, |ui| {
                                for (i, a) in c.albums.iter().enumerate() {
                                    ui.selectable_value(&mut on.album, i, name(a));
                                }
                            });
                        });
                    }
                    // Häkchen vorbelegen nur für leere Felder (ergänzen); Abweichungen stehen fett da und
                    // lassen sich von Hand anhaken. Genres bei MusicBrainz sind oft eigenwillig.
                    let loading = on.details_rx.is_some();
                    let key = (on.chosen, on.album, loading);
                    if on.take_for != Some(key) {
                        on.take_for = Some(key);
                        for (i, f) in FIELDS.into_iter().enumerate() {
                            let new = Self::proposal(on, f).unwrap_or_default();
                            let old = match f {
                                Field::Cover => if old_cover.is_some() || has_new_cover { "1".into() } else { String::new() },
                                _ => f.slot(&mut current.clone()).map(|s| s.clone()).unwrap_or_default(),
                            };
                            on.take[i] = !new.is_empty() && old.trim().is_empty();
                        }
                    }
                    ui.add_space(4.0);
                    egui::Grid::new("online-vergleich").num_columns(3).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                        ui.label("");
                        ui.label(RichText::new(tr("jetzt")).size(12.0).color(theme::dim()));
                        ui.label(RichText::new(tr("neu")).size(12.0).color(theme::dim()));
                        ui.end_row();
                        for (i, f) in FIELDS.into_iter().enumerate() {
                            let new = Self::proposal(on, f).unwrap_or_default();
                            let pending = matches!(f, Field::Genre | Field::Cover) && loading;
                            let old = f.slot(&mut current.clone()).map(|s| s.clone()).unwrap_or_default();
                            ui.add_enabled(!new.is_empty(), egui::Checkbox::new(&mut on.take[i], f.label()));
                            if f == Field::Cover {
                                cover_cell(ui, old_cover.as_ref(), if has_new_cover { tr("neues gewählt") } else { tr("keins") });
                                if pending {
                                    ui.add(egui::Spinner::new().size(12.0));
                                } else {
                                    cover_cell(ui, on.cover_tex.as_ref(), tr("keins gefunden"));
                                }
                            } else {
                                cell(ui, egui::Label::new(RichText::new(&old).color(theme::dim())).truncate());
                                if pending {
                                    ui.add(egui::Spinner::new().size(12.0));
                                } else {
                                    let text = RichText::new(&new);
                                    let text = if new != old && !new.is_empty() { text.strong() } else { text.color(theme::dim()) };
                                    cell(ui, egui::Label::new(text).truncate());
                                }
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let any = on.take.iter().any(|t| *t);
                        if ui.add_enabled(any, egui::Button::new(RichText::new(tr("Übernehmen")).strong())).on_hover_text(tr("Trägt die angehakten Werte in die Felder ein; geschrieben wird erst mit „Speichern“")).clicked() {
                            apply = true;
                        }
                        if ui.button(tr("Verwerfen")).clicked() {
                            close = true;
                        }
                        ui.label(RichText::new(tr("Daten: MusicBrainz, AcoustID, Cover Art Archive")).size(11.0).color(theme::dim()));
                    });
                    return finish_online(self, ctx, event, close, apply, search);
                }
                None => {}
            }
            if ui.button(tr("Schließen")).clicked() {
                close = true;
            }
        }
        finish_online(self, ctx, event, close, apply, search)
    }
}

fn finish_online(ed: &mut TagEditor, ctx: &egui::Context, event: Option<TagEditorEvent>, close: bool, apply: bool, search: bool) -> Option<TagEditorEvent> {
    if search {
        ed.start_search(ctx);
    } else if apply {
        ed.apply_online();
    } else if close {
        ed.online = None;
    }
    event
}

/// Linksbündige Zelle fester Breite in der Vergleichstabelle (abgeschnitten mit „…“).
fn cell(ui: &mut egui::Ui, label: egui::Label) {
    ui.allocate_ui_with_layout(vec2(150.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(150.0);
        ui.add(label);
    });
}

/// Kleines Coverbild in der Vergleichstabelle, sonst ein Wort.
fn cover_cell(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, none: &str) {
    match tex {
        Some(t) => {
            ui.add(egui::Image::new(t).fit_to_exact_size(vec2(48.0, 48.0)).corner_radius(3.0));
        }
        None => {
            ui.label(RichText::new(none).size(12.0).color(theme::dim()));
        }
    }
}

fn texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
    let c = fw_library::meta::decode_cover(bytes).ok()?;
    let img = egui::ColorImage::from_rgba_unmultiplied([c.size as usize; 2], &c.rgba);
    Some(ctx.load_texture(name, img, egui::TextureOptions::LINEAR))
}

/// Art eines Albums bei MusicBrainz in der Sprache der Oberfläche.
fn kind_name(kind: &str) -> &str {
    match kind {
        "Album" => tr("Album"),
        "Single" => tr("Single"),
        "EP" => tr("EP"),
        "Compilation" => tr("Zusammenstellung"),
        "Live" => tr("Live"),
        "Soundtrack" => tr("Soundtrack"),
        "Remix" => tr("Remix"),
        "DJ-mix" => tr("DJ-Mix"),
        "Other" => tr("Sonstiges"),
        other => other,
    }
}

/// Eine Zeile mit den technischen Angaben, z. B. „Mpeg · ID3v2 · 3:20 · 320 kbit/s · 44,1 kHz · Stereo · 7,6 MB“.
fn describe(i: &FileInfo) -> String {
    let num = |v: f64, digits: usize| {
        let s = format!("{v:.digits$}");
        if lang() == Lang::De { s.replace('.', ",") } else { s }
    };
    // Namen aus lofty lesbar machen („Mpeg“ → „MP3“, „Id3v2“ → „ID3v2“)
    let pretty = |s: &str| match s {
        "Mpeg" => "MP3".to_owned(),
        "Mp4" => "MP4/M4A".to_owned(),
        "Vorbis" => "Ogg Vorbis".to_owned(),
        "Id3v1" | "Id3v2" => s.replace("Id3", "ID3"),
        "VorbisComments" => "Vorbis Comments".to_owned(),
        "Mp4Ilst" => "MP4".to_owned(),
        "RiffInfo" => "RIFF INFO".to_owned(),
        "AiffText" => "AIFF".to_owned(),
        other => other.to_uppercase(),
    };
    let mut parts = vec![pretty(&i.format), pretty(&i.tag_format), fmt_time(i.duration_secs)];
    if let Some(b) = i.bitrate_kbps.filter(|b| *b > 0) {
        parts.push(format!("{b} kbit/s"));
    }
    if let Some(r) = i.sample_rate {
        parts.push(format!("{} kHz", num(r as f64 / 1000.0, 1)));
    }
    if let Some(d) = i.bit_depth {
        parts.push(format!("{d} Bit"));
    }
    match i.channels {
        Some(1) => parts.push(tr("Mono").into()),
        Some(2) => parts.push(tr("Stereo").into()),
        Some(n) => parts.push(format!("{n} ch")),
        None => {}
    }
    parts.push(format!("{} MB", num(i.size_bytes as f64 / 1_000_000.0, 1)));
    parts.join(" · ")
}
