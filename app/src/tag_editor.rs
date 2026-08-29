// SPDX-License-Identifier: GPL-3.0-or-later
//! Fenster „Titelinfo & Tags“: zeigt Dateiangaben und lässt die Tags eines Titels bearbeiten.
//! Gespeichert wird in einem eigenen Thread, weil das Umschreiben großer Dateien dauern kann.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{self, Align2, Frame, Margin, RichText, vec2};
use fw_library::{FileInfo, TagFields, read_tags, write_tags};

use crate::deck_ui::fmt_time;
use crate::i18n::{Lang, lang, tr};
use crate::{fonts, theme};

pub enum TagEditorEvent {
    Close,
    /// Tags wurden in die Datei geschrieben; Anzeigen neu einlesen.
    Saved(PathBuf),
    Reveal(PathBuf),
}

pub struct TagEditor {
    path: PathBuf,
    fields: TagFields,
    original: TagFields,
    info: Option<FileInfo>,
    error: Option<String>,
    saving: Option<Receiver<Result<(), String>>>,
}

impl TagEditor {
    pub fn open(path: PathBuf) -> Self {
        let (fields, info, error) = match read_tags(&path) {
            Ok((f, i)) => (f, Some(i), None),
            Err(e) => (TagFields::default(), None, Some(format!("{e:#}"))),
        };
        Self { path, original: fields.clone(), fields, info, error, saving: None }
    }

    fn save(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (path, fields) = (self.path.clone(), self.fields.clone());
        std::thread::Builder::new()
            .name("fw-tag-write".into())
            .spawn(move || {
                let _ = tx.send(write_tags(&path, &fields).map_err(|e| format!("{e:#}")));
            })
            .ok();
        self.saving = Some(rx);
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<TagEditorEvent> {
        let mut event = None;
        if let Some(rx) = &self.saving {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.saving = None;
                    self.original = self.fields.clone();
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
        let changed = self.fields != self.original;
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
