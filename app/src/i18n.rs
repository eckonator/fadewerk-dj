// SPDX-License-Identifier: GPL-3.0-or-later
//! Sprachen der Oberfläche.
//!
//! Der deutsche Text im Code ist zugleich der Schlüssel: `tr("Leeren")` liefert auf Englisch "Clear".
//! Platzhalter `{}` werden mit `trf` der Reihe nach ersetzt. Fehlt eine Übersetzung, bleibt der deutsche Text.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    De,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::De, Lang::En];

    /// Name der Sprache in ihrer eigenen Schreibweise.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::De => "Deutsch",
            Lang::En => "English",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::En as u8 { Lang::En } else { Lang::De }
}

/// Übersetzt einen festen Text.
pub fn tr(de: &'static str) -> &'static str {
    match lang() {
        Lang::De => de,
        Lang::En => en(de).unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                log::debug!("Übersetzung fehlt: {de:?}");
            }
            de
        }),
    }
}

/// Übersetzt einen Text mit `{}`-Platzhaltern und setzt die Werte der Reihe nach ein.
pub fn trf(de: &'static str, args: &[&dyn Display]) -> String {
    let template = tr(de);
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = args.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// „12 Titel · 41:20“ mit richtiger Einzahl auf Englisch.
pub fn count_tracks(n: usize, time: &str) -> String {
    if lang() == Lang::En && n == 1 {
        format!("1 track · {time}")
    } else {
        trf("{} Titel · {}", &[&n, &time])
    }
}

fn en(de: &str) -> Option<&'static str> {
    Some(match de {
        // Player
        "Kein Titel geladen" => "No track loaded",
        "Doppelklick in der Playlist lädt einen Titel" => "Double-click a playlist entry to load it",
        "Lädt …" => "Loading …",
        "Auswerfen: Player leeren" => "Eject: empty this deck",
        "GESPIELT" => "ELAPSED",
        "REST" => "REMAIN",
        "AUS" => "OFF",
        "TITEL" => "TRACK",
        "LISTE" => "LIST",
        "VORHÖREN" => "PRE-LISTEN",
        "Auf dem Kopfhörer vorhören, unabhängig vom Crossfader" => "Pre-listen on headphones, independent of the crossfader",
        "Tempo an den anderen Player angleichen" => "Match tempo to the other deck",
        "Loop über {} Beats" => "Loop over {} beats",
        "Rechtsklick oder Doppelklick setzt auf 0" => "Right-click or double-click resets to 0",
        "Festhalten hält an, ziehen scratcht, tippen startet oder pausiert" => "Hold to stop, drag to scratch, tap to play or pause",
        "Teller ziehen = scratchen" => "Drag the platter to scratch",
        "Zurück: Titelanfang, in den ersten 3 s voriger Titel" => "Back: start of track, within the first 3 s the previous track",
        "Stopp: Teller läuft aus, dann zurück zum Cue-Punkt" => "Stop: platter spins down, then returns to the cue point",
        "Nächster Titel der Playlist" => "Next track in the playlist",
        "Wiederholen: aus, Titel, ganze Playlist" => "Repeat: off, track, whole playlist",
        // Mixer
        "MIXER" => "MIXER",
        "MASTER" => "MASTER",
        "KOPFHÖRER" => "HEADPHONES",
        "Aus" => "Off",
        "Ausg. 3/4" => "Out 3/4",
        "Kein Kopfhörersignal" => "No headphone signal",
        "Eine Soundkarte: links Master, rechts Kopfhörer" => "One sound card: master left, headphones right",
        "Mehrkanal-Interface: Kopfhörer auf Ausgang 3/4" => "Multi-channel interface: headphones on outputs 3/4",
        "Mischung Vorhören ↔ Master" => "Mix cue ↔ master",
        "PEGEL" => "LEVEL",
        "Keine Audio-Ausgabe gefunden" => "No audio output found",
        "Ausgabe: {}" => "Output: {}",
        // Auto-Fade
        "AUTO-FADE" => "AUTO-FADE",
        "SEK." => "SEC.",
        "Überblendzeit" => "Fade time",
        "Weich" => "Smooth",
        "Linear" => "Linear",
        "Hart" => "Sharp",
        "Linien ziehen formt die Blendkurve, Rechtsklick setzt sie zurück" => "Drag the lines to shape the fade curve, right-click resets it",
        "Crossfader" => "Crossfader",
        "FADEN" => "FADE NOW",
        "MOTOR · ANLAUF & AUSLAUF" => "MOTOR · START & STOP",
        "ANLAUF " => "START  ",
        "AUSLAUF" => "STOP   ",
        "Blende von {} auf {}, noch {} s" => "Fading {} to {}, {} s left",
        "Auto-Fade aus, Crossfader per Hand" => "Auto-fade off, crossfader by hand",
        "Player {} starten, dann blendet Auto-Fade selbst" => "Start deck {}, auto-fade takes over from there",
        "Repeat oder Loop in {}: Auto-Fade wartet" => "Repeat or loop on {}: auto-fade waits",
        "Auto-Fade zu {} in {}" => "Auto-fade to {} in {}",
        // Playlists
        "PLAYLIST {}" => "PLAYLIST {}",
        "{} Titel · {}" => "{} tracks · {}",
        "Leeren" => "Clear",
        "Zufällige Reihenfolge" => "Shuffle",
        "Mischt die kommenden Titel. Der geladene und die schon gespielten bleiben an ihrem Platz."
            => "Shuffles the upcoming tracks. The loaded track and those already played stay where they are.",
        "Leer. Titel aus dem Ordner hierher ziehen." => "Empty. Drag tracks here from the folder below.",
        "Aus der Playlist entfernen" => "Remove from playlist",
        "In Player {} laden" => "Load into deck {}",
        // Ordnerbrowser
        "ORDNER" => "FOLDERS",
        "Ordner hinzufügen …" => "Add folder …",
        "Suchen (mit Unterordnern)" => "Search (incl. subfolders)",
        "UNTERORDNER" => "SUBFOLDER",
        "Suche läuft …" => "Searching …",
        "Durchsucht Unterordner …" => "Searching subfolders …",
        "Keine Audiodateien in diesem Ordner und seinen Unterordnern." => "No audio files in this folder or its subfolders.",
        "Nur die ersten {} Titel. Für mehr einen Unterordner wählen." => "Only the first {} tracks. Pick a subfolder for more.",
        "{} von {} Titeln" => "{} of {} tracks",
        "⏺ Live gelesen, keine Datenbank" => "⏺ Read live, no database",
        "Ordner in Playlist {}" => "Folder to playlist {}",
        "In Playlist {}" => "To playlist {}",
        "Keine Audiodateien in diesem Ordner. Links einen Unterordner wählen." => "No audio files in this folder. Pick a subfolder on the left.",
        "INTERPRET" => "ARTIST",
        "ZEIT" => "TIME",
        "Auf dem Kopfhörer vorhören" => "Pre-listen on headphones",
        "STOPP" => "STOP",
        "Vorhören beenden" => "Stop pre-listening",
        "IM KOPFHÖRER" => "IN HEADPHONES",
        "Musik" => "Music",
        "Benutzerordner" => "Home",
        "Laufwerke" => "Drives",
        "{} lässt sich nicht lesen: {}" => "Cannot read {}: {}",
        "Musikordner hinzufügen" => "Add music folder",
        "Aus der Liste entfernen" => "Remove from list",
        "HI" => "HI",
        "MID" => "MID",
        "LOW" => "LOW",
        "Band ganz herausnehmen" => "Remove this band completely",
        "Beim Überblenden kommt der neue Titel ohne Bässe herein, in der Mitte tauschen die Bässe. Klingt sauberer als ein reiner Lautstärke-Fade."
            => "While fading, the new track comes in without bass; halfway through the basses swap. Sounds cleaner than a plain volume fade.",
        "Auswahl in Playlist {}" => "Selection to playlist {}",
        "Auswahl ({} Titel) in Playlist {}" => "Selection ({} tracks) to playlist {}",
        "· {} ausgewählt" => "· {} selected",
        "{} Ordner" => "{} folders",
        "{} Ordner ausgewählt" => "{} folders selected",
        "{} Titel ausgewählt" => "{} tracks selected",
        "An Playlist {} anhängen" => "Append to playlist {}",
        "Gemischt an Playlist {} anhängen" => "Append shuffled to playlist {}",
        "Auf Playlist A und B verteilen" => "Spread across playlists A and B",
        "Abwechselnd ein Titel nach A, der nächste nach B. Passt zum Auto-Fade."
            => "Alternating: one track to A, the next to B. Made for auto-fade.",
        "Als Nächstes in Playlist {}" => "Play next in playlist {}",
        "Direkt hinter den geladenen Titel" => "Right after the loaded track",
        "Spielt gerade. Erst stoppen." => "Playing right now. Stop it first.",
        "Vorhören" => "Pre-listen",
        "Im Finder zeigen" => "Show in Finder",
        "Im Explorer zeigen" => "Show in Explorer",
        "Im Dateimanager zeigen" => "Show in file manager",
        "{} Titel als Nächstes in Playlist {}" => "{} tracks up next in playlist {}",
        "{} Titel auf Playlist A und B verteilt" => "{} tracks spread across playlists A and B",
        "Lässt sich nicht anzeigen: {}" => "Cannot show: {}",
        // Hinweise
        "{} Titel in Playlist {}" => "{} tracks added to playlist {}",
        "„{}“ lässt sich nicht abspielen" => "“{}” cannot be played",
        "Das Ausgabegerät hat nur {} Kanäle" => "The output device has only {} channels",
        "Der Titel ist in Player {} geladen und bleibt in dessen Playlist" => "This track is loaded in deck {} and stays in its playlist",
        "Für Sync brauchen beide Player einen Titel mit BPM" => "Sync needs a track with BPM on both decks",
        "Kopfhörer: links Master, rechts Vorhören" => "Headphones: master left, cue right",
        "Player {} ist gerade zu hören. Erst stoppen oder wegblenden." => "Deck {} is on air. Stop it or fade away first.",
        "Player {} spielt gerade. Erst stoppen." => "Deck {} is playing. Stop it first.",
        "Playlist {}: kein weiterer Titel" => "Playlist {}: no further track",
        "Playlist {} hat keinen nächsten Titel" => "Playlist {} has no next track",
        "Vorhören auf dem rechten Kanal (Split L/R)" => "Cue on the right channel (split L/R)",
        "Zu weit auseinander ({} ↔ {} BPM), Pitch reicht nur ±{} %" => "Too far apart ({} ↔ {} BPM), pitch only covers ±{} %",
        "Kein Ton: {}" => "No sound: {}",
        "{} Titel aus den Playlists gibt es nicht mehr" => "{} playlist tracks no longer exist",
        "Audiogerät gewechselt: {}" => "Audio device changed: {}",
        // Einstellungen
        "Einstellungen" => "Settings",
        "Einstellungen (⌘,)" => "Settings (⌘,)",
        "Audio" => "Audio",
        "Sprache" => "Language",
        "Aussehen" => "Appearance",
        "Ausgabegerät" => "Output device",
        "Systemstandard" => "System default",
        "Puffergröße" => "Buffer size",
        "Standard" => "Default",
        "{} Samples ≈ {} ms" => "{} samples ≈ {} ms",
        "Kleiner Puffer: weniger Verzögerung beim Scratchen, aber mehr Rechenlast. Bei Knacksern größer wählen."
            => "Smaller buffer: less delay when scratching, but more CPU load. Choose larger if you hear clicks.",
        "Liste aktualisieren" => "Refresh list",
        "Aktiv: {} · {} Hz · {} Kanäle" => "Active: {} · {} Hz · {} channels",
        "Lautsprecher (Master)" => "Speakers (master)",
        "Kopfhörer" => "Headphones",
        "Kein eigenes Gerät" => "No separate device",
        "Ohne eigenes Gerät läuft der Kopfhörer über „Split L/R“ oder „Ausg. 3/4“ im Mixer."
            => "Without a separate device, headphones use “Split L/R” or “Out 3/4” in the mixer.",
        "Zwei Geräte laufen nie exakt gleich schnell. FadeWerk DJ gleicht das automatisch aus; der Kopfhörer liegt dabei etwa 20 ms hinter den Lautsprechern."
            => "Two devices never run at exactly the same speed. FadeWerk DJ compensates automatically; the headphones lag the speakers by about 20 ms.",
        "Kopfhörer aktiv: {} · {} Hz" => "Headphones active: {} · {} Hz",
        "Gerät" => "Device",
        "Eigenes Kopfhörergerät aus den Einstellungen" => "Separate headphone device from the settings",
        "Kopfhörergerät nicht verfügbar: {}" => "Headphone device unavailable: {}",
        "Das Kopfhörergerät ist dasselbe wie die Lautsprecher. Bitte ein anderes wählen oder im Mixer Split L/R nutzen."
            => "The headphone device is the same as the speakers. Pick another one or use split L/R in the mixer.",
        "Vorhören über das Kopfhörergerät" => "Pre-listening on the headphone device",
        "Sprache der Oberfläche" => "Interface language",
        "Skin" => "Skin",
        "Gebürsteter Stahl" => "Brushed steel",
        "Über" => "About",
        "Version {}" => "Version {}",
        "DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade." => "DJ player with two decks, two playlists and auto-fade.",
        "FadeWerk DJ ist freie Software: Du kannst es unter den Bedingungen der GNU General Public License, Version 3 oder (nach deiner Wahl) jeder späteren Version, weitergeben und verändern."
            => "FadeWerk DJ is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License, version 3 or (at your option) any later version.",
        "Es wird OHNE JEDE GEWÄHRLEISTUNG bereitgestellt, auch ohne die implizite Gewährleistung der Marktfähigkeit oder Eignung für einen bestimmten Zweck."
            => "It is provided WITHOUT ANY WARRANTY, without even the implied warranty of merchantability or fitness for a particular purpose.",
        "Quellcode:" => "Source code:",
        "Verwendete Fremdbestandteile" => "Third-party components",
        "Schriften Chakra Petch und DSEG (SIL Open Font License 1.1), egui und eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0) und weitere Rust-Bibliotheken. Die vollständige Liste liegt jedem Release als THIRD-PARTY-LICENSES.yml bei."
            => "Fonts Chakra Petch and DSEG (SIL Open Font License 1.1), egui and eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0) and other Rust libraries. The full list ships with every release as THIRD-PARTY-LICENSES.yml.",
        "Lizenztexte" => "License texts",
        "Gehäuseflächen wirken wie gebürstetes Metall in den Farben des Skins." => "Panels look like brushed metal in the skin's colours.",
        "Einstellungen werden automatisch gespeichert." => "Settings are saved automatically.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_in_order() {
        set_lang(Lang::De);
        assert_eq!(trf("Auto-Fade zu {} in {}", &[&"B", &"0:12"]), "Auto-Fade zu B in 0:12");
        set_lang(Lang::En);
        assert_eq!(trf("Auto-Fade zu {} in {}", &[&"B", &"0:12"]), "Auto-fade to B in 0:12");
        assert_eq!(tr("Leeren"), "Clear");
        assert_eq!(tr("gibt es nicht"), "gibt es nicht");
        set_lang(Lang::De);
    }
}
