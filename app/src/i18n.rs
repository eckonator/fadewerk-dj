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
    // Ohne Buchstaben (leer, „#“, Zahlen) gibt es nichts zu übersetzen
    if !de.chars().any(char::is_alphabetic) {
        return de;
    }
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
        "Klick = Play/Pause, Rechtsklick = Stopp, festhalten, ziehen oder Mausrad = scratchen" => "Click to play or pause, right-click to stop, hold, drag or use the mouse wheel to scratch",
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
        "Links (CUE): nur was du vorhörst · rechts (MST): die Musik der Lautsprecher" => "Left (CUE): only what you pre-listen to · right (MST): the music on the speakers",
        "PEGEL" => "LEVEL",
        "Keine Audio-Ausgabe gefunden" => "No audio output found",
        // Auto-Fade
        "AUTO-FADE" => "AUTO-FADE",
        "SEK." => "SEC.",
        "Überblendzeit" => "Fade time",
        "Zuklappen" => "Collapse",
        "Klicken, um die Zeit einzugeben" => "Click to type in the time",
        "Anlaufzeit bei Play" => "Start-up time on play",
        "Auslaufzeit bei Pause und Stopp" => "Run-down time on pause and stop",
        "ANLAUF" => "START",
        "AUSLAUF" => "STOP",
        "Alle eigenen Dateien, Namen und Symbole entfernen?" => "Remove all your own files, names and symbols?",
        "Ja, zurücksetzen" => "Yes, reset",
        "Alle Effekte zurücksetzen" => "Reset all effects",
        "Alle Pads wieder mit den mitgelieferten Effekten, Namen und Symbolen belegen" => "Give every pad its built-in effect, name and symbol again",
        "HÖRER" => "PHONES",
        "Kopfhörer · zuklappen" => "Headphones · collapse",
        "Kopfhörer · aufklappen" => "Headphones · expand",
        "PADS" => "PADS",
        "HORN" => "HORN",
        "SIRENE" => "SIREN",
        "LASER" => "LASER",
        "BOOM" => "BOOM",
        "RISER" => "RISER",
        "DROP" => "DROP",
        "REWIND" => "REWIND",
        "BREMSE" => "BRAKE",
        "Lautstärke der Effekte" => "Effects volume",
        "Taste {} · Rechtsklick: im Kopfhörer vorhören" => "Key {} · right-click: pre-listen in the headphones",
        "Effekt-Pads" => "Effect pads",
        "Jedes Pad spielt einen mitgelieferten Effekt oder eine eigene Datei (MP3, WAV, FLAC, OGG, M4A). Auslösen mit Klick oder den Tasten 1 bis 8, Rechtsklick auf ein Pad hört es im Kopfhörer vor." => "Each pad plays a built-in effect or your own file (MP3, WAV, FLAC, OGG, M4A). Trigger it with a click or the keys 1 to 8; right-click a pad to pre-listen in the headphones.",
        "Mitgeliefert" => "Built-in",
        "Datei …" => "File …",
        "Audiodateien" => "Audio files",
        "Mitgelieferten Effekt, Namen und Symbol wiederherstellen" => "Restore the built-in effect, name and symbol",
        "Symbol wählen" => "Choose a symbol",
        "Symbol für Pad {}" => "Symbol for pad {}",
        "Horn" => "Horn",
        "Sirene" => "Siren",
        "Blitz" => "Lightning",
        "Explosion" => "Explosion",
        "Anstieg" => "Rise",
        "Abfall" => "Fall",
        "Rückspulen" => "Rewind",
        "Glocke" => "Bell",
        "Note" => "Note",
        "Lautsprecher" => "Speaker",
        "Mikrofon" => "Microphone",
        "Stern" => "Star",
        "Herz" => "Heart",
        "Bombe" => "Bomb",
        "Welle" => "Wave",
        "Im Kopfhörer anhören" => "Listen in the headphones",
        "Effekt für Pad {}" => "Effect for pad {}",
        "Effekt {}: Datei nicht lesbar, es klingt der mitgelieferte Effekt" => "Effect {}: file not readable, the built-in effect plays instead",
        "KURVE" => "CURVE",
        "Aufklappen" => "Expand",
        "MOTOR" => "MOTOR",
        "Wie schnell die Teller bei Play anlaufen und bei Pause oder Stopp auslaufen. Im Kopfhörer starten und halten sie sofort." => "How fast the platters start on play and run down on pause or stop. In the headphones they start and stop instantly.",
        "Rückwärts: der Motor dreht die Platte andersherum" => "Reverse: the motor turns the record the other way",
        "Klicken, um die Überblendzeit einzugeben" => "Click to type in the fade time",
        "Weich" => "Smooth",
        "Linear" => "Linear",
        "Hart" => "Sharp",
        "Linien ziehen formt die Blendkurve, Rechtsklick setzt sie zurück" => "Drag the lines to shape the fade curve, right-click resets it",
        "Crossfader" => "Crossfader",
        "FADE" => "FADE",
        "MOTOR · ANLAUF & AUSLAUF" => "MOTOR · START & STOP",
        "Blende von {} auf {}, noch {} s" => "Fading {} to {}, {} s left",
        "Auto-Fade aus, Crossfader per Hand" => "Auto-fade off, crossfader by hand",
        "Player {} starten, dann blendet Auto-Fade selbst" => "Start deck {}, auto-fade takes over from there",
        "Repeat oder Loop in {}: Auto-Fade wartet" => "Repeat or loop on {}: auto-fade waits",
        "Auto-Fade zu {} in {}" => "Auto-fade to {} in {}",
        // Playlists
        "PLAYLIST {}" => "PLAYLIST {}",
        "{} Titel · {}" => "{} tracks · {}",
        "Leeren" => "Clear",
        "Mischt die kommenden Titel. Der geladene und die schon gespielten bleiben an ihrem Platz."
            => "Shuffles the upcoming tracks. The loaded track and those already played stay where they are.",
        "FadeWerk: zeigt die Wellenform jedes Titels. Hier legst du fest, wie jeder Titel ein- und ausgeblendet wird."
            => "FadeWerk: shows the waveform of every track. This is where you set how each track fades in and out.",
        "Fade-Bereiche dieses Titels entfernen" => "Remove this track's fade ranges",
        "Alle Fade-Bereiche entfernen …" => "Remove all fade ranges …",
        "Ein- und Ausblendbereiche aller Titel in beiden Playlists entfernen?" => "Remove the fade-in and fade-out ranges of all tracks in both playlists?",
        "Ja, alle entfernen" => "Yes, remove all",
        "Alle Fade-Bereiche entfernt" => "All fade ranges removed",
        "Wellenform wird berechnet …" => "Calculating waveform …",
        "nicht gesetzt (halbe Überblendzeit)" => "not set (half the fade time)",
        "harter Einstieg bei {}" => "hard start at {}",
        "harter Schluss bei {}" => "hard end at {}",
        "{} bis {} · {} s" => "{} to {} · {} s",
        "EINBLENDEN" => "FADE IN",
        "AUSBLENDEN" => "FADE OUT",
        "Ziehen = Bereich · Klick = harter Schnitt · Rechtsklick = löschen"
            => "Drag = range · click = hard cut · right-click = remove",
        "ZURÜCKSETZEN" => "RESET",
        "Schließt den Editor (auch mit Esc oder Rechtsklick neben die Ausschnitte)" => "Closes the editor (also with Esc or a right-click beside the views)",
        "FERTIG" => "DONE",
        "ÜBERGANG" => "TRANSITION",
        "Eigene Fade-Bereiche" => "Custom fade ranges",
        "Einblenden" => "Fade in",
        "Ausblenden" => "Fade out",
        "harter Schnitt bei {}" => "hard cut at {}",
        "BEREICH" => "RANGE",
        "von „{}“ ({})" => "from “{}” ({})",
        "zu „{}“ ({})" => "to “{}” ({})",
        "Einblenden allein auf dem Kopfhörer vorhören" => "Preview the fade-in alone on the headphones",
        "Ausblenden allein auf dem Kopfhörer vorhören" => "Preview the fade-out alone on the headphones",
        "Übergang von „{}“ (Playlist {}) auf dem Kopfhörer vorhören" => "Preview the transition from “{}” (playlist {}) on the headphones",
        "Übergang zu „{}“ (Playlist {}) auf dem Kopfhörer vorhören" => "Preview the transition to “{}” (playlist {}) on the headphones",
        "Davor kommt kein Titel aus der anderen Playlist" => "No track from the other playlist comes before this one",
        "Danach kommt kein Titel aus der anderen Playlist" => "No track from the other playlist comes after this one",
        "Vorhören nicht möglich, der Titel lässt sich nicht lesen" => "Cannot preview, the track cannot be read",
        "Leer. Titel aus dem Ordner hierher ziehen." => "Empty. Drag tracks here from the folder below.",
        "Aus der Playlist entfernen" => "Remove from playlist",
        "In Player {} laden" => "Load into deck {}",
        // Ordnerbrowser
        "ORDNER" => "FOLDERS",
        "Ordner hinzufügen …" => "Add folder …",
        "Suchen" => "Search",
        "UNTERORDNER" => "SUBFOLDER",
        "Suche läuft …" => "Searching …",
        "Durchsucht Unterordner …" => "Searching subfolders …",
        "Keine Audiodateien in diesem Ordner und seinen Unterordnern." => "No audio files in this folder or its subfolders.",
        "Nur die ersten {} Titel. Für mehr einen Unterordner wählen." => "Only the first {} tracks. Pick a subfolder for more.",
        "{} von {} Titeln" => "{} of {} tracks",
        "⏺ Live gelesen, keine Datenbank" => "⏺ Read live, no database",
        "Ordner in Playlist {}" => "Folder to playlist {}",
        "In Playlist {}" => "To playlist {}",
        "INTERPRET" => "ARTIST",
        "ZEIT" => "TIME",
        "Auf dem Kopfhörer vorhören" => "Pre-listen on headphones",
        "Vorhören beenden" => "Stop pre-listening",
        "IM KOPFHÖRER" => "IN HEADPHONES",
        "Musik" => "Music",
        "Benutzerordner" => "Home",
        "Laufwerke" => "Drives",
        "{} lässt sich nicht lesen: {}" => "Cannot read {}: {}",
        "Musikordner hinzufügen" => "Add music folder",
        "Aus der Liste entfernen" => "Remove from list",
        "Als Nächstes spielen" => "Play next",
        "Mit Unterordnern" => "Include subfolders",
        "Auch alle Unterordner durchsuchen. Bei Laufwerken und dem Benutzerordner ist das anfangs aus, damit nicht die ganze Platte durchsucht wird. Systemordner werden immer ausgelassen."
            => "Also search all subfolders. Off at first for drives and the home folder so the whole disk isn't scanned. System folders are always skipped.",
        "Stopp" => "Stop",
        "Suche anhalten, gefundene Titel bleiben" => "Stop searching, tracks found so far stay",
        "Suche angehalten" => "Search stopped",
        "Keine Audiodateien direkt in diesem Ordner. „Mit Unterordnern“ durchsucht auch die Ordner darin."
            => "No audio files directly in this folder. “Include subfolders” also searches the folders inside.",
        "Kopfhörer: {}" => "Headphones: {}",
        "MST" => "MST",
        "CUE" => "CUE",
        "Split L/R" => "Split L/R",
        "Titelinfo & Tags" => "Track info & tags",
        "Titelinfo & Tags …" => "Track info & tags …",
        "Titel" => "Title",
        "Interpret" => "Artist",
        "Album" => "Album",
        "Album-Interpret" => "Album artist",
        "Genre" => "Genre",
        "Jahr" => "Year",
        "Track" => "Track",
        "BPM" => "BPM",
        "Kommentar" => "Comment",
        "Unterordner" => "Subfolder",
        "Ordner" => "Folder",
        "Erweiterte Suche" => "Advanced search",
        "Erweiterte Suche (⇧⌘F)" => "Advanced search (⇧⌘F)",
        "Erweiterte Suche (Strg+Umschalt+F)" => "Advanced search (Ctrl+Shift+F)",
        "von" => "from",
        "bis" => "to",
        "Sucht in den gewählten Ordnern. Titel und Interpret stammen aus den Tags, fehlen sie, aus dem Dateinamen („Interpret - Titel“). Bei Jahr und BPM fallen Titel ohne Angabe heraus; BPM kennt FadeWerk DJ aus den Tags oder nach dem Laden in einen Player." => "Searches the selected folders. Title and artist come from the tags, or from the file name (“Artist - Title”) if there are none. For year and BPM, tracks without a value are left out; FadeWerk DJ knows the BPM from the tags or after loading a track into a deck.",
        "Alles löschen" => "Clear all",
        "Fertig" => "Done",
        "FILTER" => "FILTER",
        "Filter entfernen" => "Remove filter",
        "Alle entfernen" => "Remove all",
        "Keine Titel passen zur Suche." => "No tracks match the search.",
        "Suchfilter merken" => "Remember search filters",
        "Doppelte Titel ausblenden" => "Hide duplicate tracks",
        "Liegt derselbe Titel mehrfach in den Ordnern (gleicher Interpret und Titel, fast gleich lang), zeigt die Suche nur das erste Exemplar." => "If the same track is in your folders more than once (same artist and title, nearly the same length), the search shows only the first copy.",
        "· {} doppelt" => "· {} duplicates",
        "Kopien desselben Titels sind ausgeblendet. Abschalten in der erweiterten Suche (Lupe)." => "Copies of the same track are hidden. Turn this off in the advanced search (magnifier).",
        "Wartende BPM-Analysen verwerfen" => "Discard pending BPM analyses",
        "Im Hintergrund für alle Titel ohne BPM-Angabe im Tag. Die Werte werden gemerkt." => {
            "In the background for all tracks without a BPM tag. The values are remembered."
        }
        "Fehlende BPM ermitteln" => "Detect missing BPM",
        "Tempo im Hintergrund neu ermitteln, auch wenn eine BPM-Angabe im Tag steht. Der Wert wird gemerkt." => {
            "Detect the tempo again in the background, even if the tag has a BPM value. The value is remembered."
        }
        "BPM analysieren" => "Analyse BPM",
        "BPM ermitteln" => "Detect BPM",
        "Alle BPM sind bekannt" => "All BPM values are known",
        "Fehlende BPM der Titel in der Liste im Hintergrund ermitteln (BPM aus den Tags werden übernommen)" => {
            "Detect missing BPM of the tracks in the list in the background (BPM from tags is used as is)"
        }
        "Fehlende BPM der ausgewählten Titel im Hintergrund ermitteln (BPM aus den Tags werden übernommen)" => {
            "Detect missing BPM of the selected tracks in the background (BPM from tags is used as is)"
        }
        "BPM-Analyse {} von {}" => "BPM analysis {} of {}",
        "Die Suchfilter bleiben nach dem Schließen des Fensters und nach einem Neustart erhalten." => {
            "The search filters stay after closing the window and after a restart."
        }
        "Bearbeiten …" => "Edit …",
        "Dateiname" => "File name",
        "Format" => "Format",
        "Zeit" => "Time",
        "SPALTEN" => "COLUMNS",
        "Spalten anzeigen" => "Show columns",
        "Ursprüngliche Reihenfolge" => "Original order",
        "Aufsteigend sortieren" => "Sort ascending",
        "Absteigend sortieren" => "Sort descending",
        "Sortierung aufheben" => "Remove sorting",
        "In Playlist {} verschieben" => "Move to playlist {}",
        "Wiederherstellen" => "Restore",
        "Playlists A und B wieder so füllen, wie an diesem Tag gespielt wurde. Ersetzt die jetzigen Playlists; ein gerade laufender Titel bleibt." => "Fill playlists A and B again the way this day was played. Replaces the current playlists; a track that is playing stays.",
        "Keine der Dateien dieses Tages ist zu finden" => "None of this day's files can be found",
        "Wiederhergestellt: {} Titel in Playlist A, {} in B" => "Restored: {} tracks in playlist A, {} in B",
        "Tonarm ziehen = an diese Stelle springen" => "Drag the tonearm = jump to that spot",
        // Historie
        "Historie" => "History",
        "Gespielte Titel, Tag für Tag" => "Played tracks, day by day",
        "Historie importieren …" => "Import history …",
        "Historie importieren" => "Import history",
        "Historie exportieren" => "Export history",
        "Historie oder Playlist" => "History or playlist",
        "Importiert …" => "Importing …",
        "noch leer" => "empty so far",
        "Noch nichts gespielt. Hier landen die gespielten Titel, Tag für Tag der Reihe nach: ein Titel zählt, sobald er 30 Sekunden über Master zu hören war." => "Nothing played yet. Played tracks end up here, day by day in order: a track counts once it has been audible on master for 30 seconds.",
        "Links einen Tag wählen. Ein Titel zählt, sobald er 30 Sekunden über Master zu hören war; Titel bis 6 Uhr früh gehören zum Vortag." => "Pick a day on the left. A track counts once it has been audible on master for 30 seconds; tracks until 6 am belong to the day before.",
        "Rechtsklick auf einen Tag: umbenennen (z. B. „Omas Geburtstag“), exportieren oder löschen." => "Right-click a day to rename it (e.g. “Grandma’s birthday”), export or delete it.",
        "FadeWerk-DJ-Historie (.json) oder Playlist (.m3u, .m3u8)" => "FadeWerk DJ history (.json) or playlist (.m3u, .m3u8)",
        "Gespielt" => "Played",
        "{} Titel" => "{} tracks",
        "{} nicht gefunden" => "{} not found",
        "Diese Dateien gibt es nicht mehr an ihrem Platz. Sie bleiben in der Historie, lassen sich aber nicht abspielen." => "These files are no longer where they were. They stay in the history but can't be played.",
        "Datei nicht gefunden: {}" => "File not found: {}",
        "An diesem Tag ist nichts mehr eingetragen." => "Nothing left on this day.",
        "Aus der Historie entfernen" => "Remove from history",
        "{} Einträge aus der Historie entfernen" => "Remove {} entries from history",
        "{} Einträge aus der Historie entfernt" => "{} entries removed from history",
        "Alle in Playlist {}" => "All to playlist {}",
        "Umbenennen …" => "Rename …",
        "Eigener Name für diesen Tag, z. B. „Omas Geburtstag“" => "Your own name for this day, e.g. “Grandma’s birthday”",
        "Name, z. B. Omas Geburtstag" => "Name, e.g. Grandma's birthday",
        "Exportieren" => "Export",
        "Als Playlist (M3U8) …" => "As playlist (M3U8) …",
        "Als Tabelle (CSV) …" => "As table (CSV) …",
        "Als FadeWerk-DJ-Historie …" => "As FadeWerk DJ history …",
        "Lässt sich in fast jedem Player öffnen" => "Opens in almost any player",
        "Uhrzeit, Interpret, Titel, Album, Länge – etwa für Excel oder eine Musikfolge-Meldung" => "Time, artist, title, album, length – e.g. for a spreadsheet or a music usage report",
        "Mit allen Angaben, zum Sichern oder für einen anderen Rechner" => "With all details, as a backup or for another computer",
        "Tag löschen …" => "Delete day …",
        "Tag löschen?" => "Delete day?",
        "{} Titel. Das Löschen lässt sich nicht rückgängig machen; die Musikdateien bleiben, wo sie sind." => "{} tracks. This can't be undone; the music files stay where they are.",
        "Löschen" => "Delete",
        "Abbrechen" => "Cancel",
        "Playlist als Historie importieren" => "Import playlist as history",
        "{} Titel gelesen" => "{} tracks read",
        "{} davon in deinen Musikordnern wiedergefunden" => "{} of them found again in your music folders",
        "{} nicht gefunden (bleiben ausgegraut in der Liste)" => "{} not found (shown greyed out)",
        "Datum" => "Date",
        "Name" => "Name",
        "Format JJJJ-MM-TT" => "Format YYYY-MM-DD",
        "Gibt es den Tag schon, kommen die Titel dazu." => "If the day already exists, the tracks are added to it.",
        "Importieren" => "Import",
        "Historie {} importiert: {} Titel" => "History {} imported: {} tracks",
        "Historie {}: {} neue Titel" => "History {}: {} new tracks",
        " ({} wiedergefunden)" => " ({} found again)",
        "Historie {} gelöscht" => "History {} deleted",
        "Import fehlgeschlagen: {}" => "Import failed: {}",
        "Export fehlgeschlagen: {}" => "Export failed: {}",
        "Exportiert: {}" => "Exported: {}",
        "FadeWerk-DJ-Historie" => "FadeWerk DJ history",
        "Tabelle" => "Table",
        "Playlist" => "Playlist",
        "Nr." => "No.",
        "Uhrzeit" => "Time",
        "Länge" => "Length",
        "Player" => "Deck",
        "Datei" => "File",
        "Gespielte Titel aufzeichnen" => "Record played tracks",
        "Ein Titel zählt, sobald er 30 Sekunden über Master zu hören war. Zu finden im Ordnerbrowser unter „Historie“." => "A track counts once it has been audible on master for 30 seconds. Find it in the folder browser under “History”.",
        "Spaltenbreiten zurücksetzen" => "Reset column widths",
        "Spalten auswählen (auch mit Rechtsklick auf den Tabellenkopf)" => "Choose columns (also by right-clicking the table header)",
        "Speichern" => "Save",
        "Schreibt die Tags direkt in die Musikdatei" => "Writes the tags straight into the music file",
        "Zurücksetzen" => "Revert",
        "Speichert …" => "Saving …",
        "Schließen" => "Close",
        "Mono" => "Mono",
        "Stereo" => "Stereo",
        "Tags gespeichert" => "Tags saved",
        "Ordner neu einlesen (⌘R)" => "Re-read folders (⌘R)",
        "Ordner neu einlesen (F5)" => "Re-read folders (F5)",
        "HI" => "HI",
        "MID" => "MID",
        "LOW" => "LOW",
        "Band ganz herausnehmen" => "Remove this band completely",
        "Beim Überblenden kommt der neue Titel ohne Bässe herein, in der Mitte tauschen die Bässe. Klingt sauberer als ein reiner Lautstärke-Fade."
            => "While fading, the new track comes in without bass; halfway through the basses swap. Sounds cleaner than a plain volume fade.",
        "Beim Titelwechsel bekommt der neue Titel das Tempo des laufenden. Danach läuft sein Pitch langsam auf 0 % zurück (0,1 % pro Sekunde)."
            => "On a track change the new track takes over the tempo of the playing one. Afterwards its pitch slowly returns to 0 % (0.1 % per second).",
        "Auto-Sync steuert den Pitch; Pitch anfassen übernimmt von Hand" => "Auto sync controls the pitch; touching the pitch takes over manually",
        "Auto-Sync: {} ↔ {} BPM liegen zu weit auseinander" => "Auto sync: {} ↔ {} BPM are too far apart",
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
        "Titel nicht geladen: Player {} spielt gerade. Erst stoppen." => "Track not loaded: deck {} is playing. Stop it first.",
        "kein Treffer" => "no match",
        "Playlist {} ist durch, {} spielt allein weiter" => "Playlist {} is finished, {} plays on alone",
        "Springt zum passenden Titel. Enter: nächster Treffer, ⇧Enter: voriger, Esc: leeren" => "Jumps to the matching track. Enter: next match, ⇧Enter: previous, Esc: clear",
        "Playlist {}: kein weiterer Titel" => "Playlist {}: no further track",
        "Playlist {} hat keinen nächsten Titel" => "Playlist {} has no next track",
        "Zu weit auseinander ({} ↔ {} BPM), Pitch reicht nur ±{} %" => "Too far apart ({} ↔ {} BPM), pitch only covers ±{} %",
        "Kein Ton: {}" => "No sound: {}",
        "{} Titel aus den Playlists gibt es nicht mehr" => "{} playlist tracks no longer exist",
        "Audiogerät gewechselt: {}" => "Audio device changed: {}",
        // Einstellungen
        "Einstellungen" => "Settings",
        "Einstellungen (⌘,)" => "Settings (⌘,)",
        "Vollbild (⌃⌘F)" => "Full screen (⌃⌘F)",
        "Vollbild (F11)" => "Full screen (F11)",
        "Plattenspieler zeigen" => "Show turntables",
        "Ohne Plattenteller bekommen die Bedienelemente mehr Platz. Scratchen geht weiter in der Wellenform." => "Without platters the controls get more room. You can still scratch in the waveform.",
        "Ordner unter die Player setzen" => "Move folders below the decks",
        "Caches" => "Caches",
        "Gemerkte Tags, Ordnerlisten und Wellenformen beschleunigen große Sammlungen und Netzlaufwerke. Leeren ist ungefährlich: Alles wird bei Bedarf neu gelesen." => "Remembered tags, folder lists and waveforms speed up large collections and network drives. Clearing them is safe: everything is read again when needed.",
        "Tags {} · Ordnerlisten {} · Wellenformen {} · ermittelte BPM und Tonarten {}" => "Tags {} · folder lists {} · waveforms {} · detected BPM and keys {}",
        "Wird gemessen …" => "Measuring …",
        "Auch ermittelte BPM und Tonarten löschen" => "Also delete detected BPM and keys",
        "Die Tempo-Erkennung dauert lange; ohne Häkchen bleiben ermittelte BPM erhalten." => "Tempo detection takes a long time; without this box, detected BPM are kept.",
        "Caches wirklich leeren?" => "Really clear the caches?",
        "Ja, leeren" => "Yes, clear",
        "Caches leeren …" => "Clear caches …",
        "Löscht alle Einstellungen, Playlists, die Historie, eigene Effekt-Pads und alle Caches. Deine Musikdateien bleiben unberührt. Danach startet FadeWerk DJ neu wie beim ersten Mal." => "Deletes all settings, playlists, the history, custom effect pads and all caches. Your music files stay untouched. FadeWerk DJ then restarts as if for the first time.",
        "Wirklich alles löschen? Das lässt sich nicht rückgängig machen." => "Really delete everything? This cannot be undone.",
        "FadeWerk DJ zurücksetzen …" => "Reset FadeWerk DJ …",
        "Gesperrt, solange ein Player spielt" => "Locked while a deck is playing",
        "FadeWerk DJ wurde zurückgesetzt." => "FadeWerk DJ has been reset.",
        "Caches geleert." => "Caches cleared.",
        "Entwicklerlauf: es wird nichts gelöscht." => "Developer run: nothing is deleted.",
        "Zurückgesetzt. Bitte FadeWerk DJ neu starten." => "Reset done. Please restart FadeWerk DJ.",
        "Über FadeWerk DJ" => "About FadeWerk DJ",
        "Prüft auf Änderungen …" => "Checking for changes …",
        "Gerechnet mit der Titelzahl der letzten Suche." => "Based on the number of tracks found last time.",
        "Geschätzt nach der Ordnerstruktur, bei sehr unterschiedlich großen Unterordnern ungenau." => "Estimated from the folder structure; less accurate if subfolders differ greatly in size.",
        "noch ca. {} s" => "about {} s left",
        "noch ca. {} Min." => "about {} min left",
        "Ordner in Spalten zeigen" => "Show folders in columns",
        "Ordner als Baum zeigen" => "Show folders as a tree",
        "Ordner rechts neben die Player setzen (für breite Monitore)" => "Move folders to the right of the decks (for wide monitors)",
        "Ordner rechts neben die Player setzen – dafür ist das Fenster zu schmal" => "Move folders to the right of the decks – the window is too narrow for that",
        "Vollbild beenden (⌃⌘F)" => "Exit full screen (⌃⌘F)",
        "Vollbild beenden (F11)" => "Exit full screen (F11)",
        "Audio" => "Audio",
        "Sprache" => "Language",
        "Aussehen" => "Appearance",
        "Systemstandard" => "System default",
        "Puffergröße" => "Buffer size",
        "Standard" => "Default",
        "{} Samples ≈ {} ms" => "{} samples ≈ {} ms",
        "Kleiner Puffer: weniger Verzögerung beim Scratchen, aber mehr Rechenlast. Bei Knacksern größer wählen."
            => "Smaller buffer: less delay when scratching, but more CPU load. Choose larger if you hear clicks.",
        "Liste aktualisieren" => "Refresh list",
        "Nur ganze Wörter" => "Whole words only",
        "„Cro“ findet dann „Cro“ und „Cro & Bausa“, aber nicht „Crosby“ oder „Macro“. Gilt für alle Textfelder dieses Fensters." => "“Cro” then finds “Cro” and “Cro & Bausa”, but not “Crosby” or “Macro”. Applies to all text fields in this window.",
        "Testton" => "Test tone",
        "Ordnet die kommenden Titel beider Playlists so, dass jeder Übergang im Wechsel A – B – A passt: zuerst nach Tonart, dann nach Tempo. Fehlende Tonarten werden vorher ermittelt." => "Orders the upcoming tracks of both playlists so that every transition in the A – B – A alternation fits: by key first, then by tempo. Missing keys are detected first.",
        "Analysiert …" => "Analyzing …",
        "Sortieren" => "Sort",
        "Harmonisch (Tonart, Tempo)" => "Harmonic (key, tempo)",
        "Zufällig" => "Shuffle",
        "Stellt die Reihenfolge von vor dem letzten Sortieren wieder her." => "Restores the order from before the last sort.",
        "Fehlende Tonarten werden ermittelt, danach wird sortiert." => "Missing keys are being detected, then the tracks are sorted.",
        "Rückgängig" => "Undo",
        "Kommende Titel harmonisch sortiert" => "Upcoming tracks sorted harmonically",
        "Reihenfolge wiederhergestellt" => "Order restored",
        "Ermittle {} fehlende Tonarten, danach wird sortiert …" => "Detecting {} missing keys, then sorting …",
        "Harmonisch sortiert; {} Titel ohne Tonart stehen am Ende" => "Sorted harmonically; {} tracks without a key are at the end",
        "Tonart" => "Key",
        "PASST" => "MATCH",
        "TONART" => "KEY",
        "Tonart unbekannt (im Tag steht keine, die Analyse läuft noch oder war unsicher)" => "Key unknown (none in the tag, analysis still running or uncertain)",
        "Klingt in {}, original {} (Pitch verschiebt die Tonhöhe um {} Halbton)" => "Sounds in {}, originally {} (pitch shifts it by {} semitone)",
        "Tonart {} (Camelot)" => "Key {} (Camelot)",
        "Analysieren" => "Analyze",
        "Analysieren (BPM, Tonart)" => "Analyze (BPM, key)",
        "BPM und Tonart sind überall bekannt" => "BPM and key are known for all tracks",
        "Fehlende BPM und Tonarten der Titel in der Liste im Hintergrund ermitteln (Angaben aus den Tags werden übernommen)" => "Detect missing BPM and keys of the tracks in the list in the background (values from the tags are used)",
        "Fehlende BPM und Tonarten der ausgewählten Titel im Hintergrund ermitteln (Angaben aus den Tags werden übernommen)" => "Detect missing BPM and keys of the selected tracks in the background (values from the tags are used)",
        "Im Hintergrund für alle Titel, denen BPM oder Tonart fehlt (im Tag oder schon ermittelt). Die Werte werden gemerkt." => "In the background for all tracks missing BPM or key (in the tag or already detected). The values are remembered.",
        "Fehlende BPM und Tonarten ermitteln" => "Detect missing BPM and keys",
        "Tempo und Tonart im Hintergrund neu ermitteln, auch wenn Angaben im Tag stehen. Die Werte werden gemerkt." => "Detect tempo and key again in the background, even if the tag has them. The values are remembered.",
        "Wartende Analysen verwerfen" => "Discard waiting analyses",
        "Analyse {} von {}" => "Analysis {} of {}",
        "Andere Geräusche" => "Other sounds",
        "Systemtöne stumm schalten, solange FadeWerk DJ läuft" => "Mute system sounds while FadeWerk DJ is running",
        "Systemtöne und andere Programme stumm schalten, solange FadeWerk DJ läuft" => "Mute system sounds and other programs while FadeWerk DJ is running",
        "Hinweistöne des Systems und der Programme sind aus und kommen beim Beenden zurück." => "Alert sounds of the system and of programs are off and come back when you quit.",
        "Auch Browser, Musik- und Chatprogramme sind stumm (wie im Lautstärkemixer) und kommen beim Beenden zurück. Neu gestartete Programme werden nach etwa 2 Sekunden stumm." => "Browsers, music and chat programs are muted as well (as in the volume mixer) and come back when you quit. Newly started programs are muted after about 2 seconds.",
        "Lautsprecher exklusiv belegen (andere Programme stumm)" => "Use the speakers exclusively (other programs muted)",
        "Nur FadeWerk DJ spielt dann über das Master-Gerät, alle anderen Programme bleiben darauf stumm, auch Videocalls. Das Kopfhörergerät ist davon nicht betroffen." => "Only FadeWerk DJ then plays on the master device; all other programs stay silent on it, video calls included. The headphone device is not affected.",
        "Mitteilungen (Mail, Kalender, Chat) blendet der Fokus „Nicht stören“ des Systems aus; den kann FadeWerk DJ nicht selbst einschalten." => "Notifications (mail, calendar, chat) are hidden by the system's “Do Not Disturb” focus; FadeWerk DJ cannot switch it on itself.",
        "Ausgabegerät lässt sich nicht exklusiv belegen: {}" => "The output device cannot be used exclusively: {}",
        "FadeWerk DJ – Ordner" => "FadeWerk DJ – Folders",
        "Ordner zurück ins Hauptfenster holen" => "Bring the folders back into the main window",
        "Ordner in ein eigenes Fenster ausklinken (etwa für einen zweiten Monitor)" => "Detach the folders into a window of their own (e.g. for a second monitor)",
        "Ordner über die Titel setzen" => "Put the folders above the tracks",
        "Ordner links neben die Titel setzen" => "Put the folders to the left of the tracks",
        "Kopfhörer zum Vorhören (zweiter Ausgang)" => "Headphones for pre-listening (second output)",
        "{} spielt zusätzlich zu den Lautsprechern. Dort hörst du, was du vorhörst (VORHÖREN, 🎧)." => "{} plays in addition to the speakers. There you hear what you pre-listen to (PRE-LISTEN, 🎧).",
        "CUE/MST im Mixer steht auf {} % Master: Der Kopfhörer spielt auch die Musik der Lautsprecher. Nur Vorhören: unter HÖRER ganz nach links auf CUE." => "CUE/MST in the mixer is at {} % master: the headphones also play the music from the speakers. Pre-listening only: under PHONES, all the way left to CUE.",
        "Das gewählte Gerät ist nicht aktiv (beim Start nicht verbunden). Zum Einschalten in der Liste erneut wählen." => "The chosen device is not active (it was not connected at start). Choose it again in the list to switch it on.",
        "Spielt einen kurzen Ton über die Lautsprecher (Master)" => "Plays a short tone on the speakers (master)",
        "Spielt einen kurzen Ton auf dem Kopfhörer" => "Plays a short tone on the headphones",
        "Aktiv: {} · {} Hz · {} Kanäle" => "Active: {} · {} Hz · {} channels",
        "Lautsprecher (Master)" => "Speakers (master)",
        "Kopfhörer" => "Headphones",
        "Kein eigenes Gerät" => "No separate device",
        "Ohne eigenes Gerät läuft der Kopfhörer über „Split L/R“ oder „Ausg. 3/4“ im Mixer."
            => "Without a separate device, headphones use “Split L/R” or “Out 3/4” in the mixer.",
        "Zwei Geräte laufen nie exakt gleich schnell. FadeWerk DJ gleicht das automatisch aus; der Kopfhörer liegt dabei etwa 20 ms hinter den Lautsprechern."
            => "Two devices never run at exactly the same speed. FadeWerk DJ compensates automatically; the headphones lag the speakers by about 20 ms.",
        "Gerät" => "Device",
        "Eigenes Kopfhörergerät aus den Einstellungen" => "Separate headphone device from the settings",
        "Kopfhörergerät nicht verfügbar: {}" => "Headphone device unavailable: {}",
        "Das Kopfhörergerät ist dasselbe wie die Lautsprecher. Bitte ein anderes wählen oder im Mixer Split L/R nutzen."
            => "The headphone device is the same as the speakers. Pick another one or use split L/R in the mixer.",
        "Vorhören über das Kopfhörergerät" => "Pre-listening on the headphone device",
        "Sprache der Oberfläche" => "Interface language",
        "Skin" => "Skin",
        "Gebürsteter Stahl" => "Brushed steel",
        "Ruhezustand" => "Sleep",
        "Allgemein" => "General",
        "Ruhezustand verhindern, solange Musik spielt" => "Prevent sleep while music is playing",
        "Bildschirm und Rechner bleiben an, solange ein Player oder das Vorhören läuft. Danach gilt wieder die Einstellung des Systems." => "Display and computer stay on while a deck or pre-listen is playing. Afterwards the system setting applies again.",
        "Musik spielt" => "Music is playing",
        "Sparsame Darstellung (für ältere oder langsame Rechner)" => "Lightweight display (for older or slower computers)",
        "Ohne Schatten, Verläufe, Leuchten und Frequenzbänder, höchstens 30 Bilder pro Sekunde. Spart Rechenzeit, der Ton bleibt gleich." => "No shadows, gradients, glow or frequency bands, at most 30 frames per second. Saves processing power; the sound stays the same.",
        "Über" => "About",
        "Version {}" => "Version {}",
        "DJ-Player mit zwei Decks, zwei Playlists und Auto-Fade." => "DJ player with two decks, two playlists and auto-fade.",
        "FadeWerk DJ ist freie Software: Du kannst es unter den Bedingungen der GNU General Public License, Version 3 oder (nach deiner Wahl) jeder späteren Version, weitergeben und verändern."
            => "FadeWerk DJ is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License, version 3 or (at your option) any later version.",
        "Es wird OHNE JEDE GEWÄHRLEISTUNG bereitgestellt, auch ohne die implizite Gewährleistung der Marktfähigkeit oder Eignung für einen bestimmten Zweck."
            => "It is provided WITHOUT ANY WARRANTY, without even the implied warranty of merchantability or fitness for a particular purpose.",
        "Quellcode:" => "Source code:",
        "Verwendete Fremdbestandteile" => "Third-party components",
        "Schriften Chakra Petch und DSEG (SIL Open Font License 1.1), egui und eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0), rusty-chromaprint (MIT) und weitere Rust-Bibliotheken. Titelerkennung über AcoustID und MusicBrainz (Daten CC0), Cover über das Cover Art Archive. Die vollständige Liste liegt jedem Release als THIRD-PARTY-LICENSES.yml bei."
            => "Fonts Chakra Petch and DSEG (SIL Open Font License 1.1), egui and eframe (MIT/Apache-2.0), cpal (Apache-2.0), Symphonia (MPL-2.0), lofty (MIT/Apache-2.0), rusty-chromaprint (MIT) and other Rust libraries. Track recognition via AcoustID and MusicBrainz (data CC0), covers via the Cover Art Archive. The full list ships with every release as THIRD-PARTY-LICENSES.yml.",
        // Online-Erkennung (Tag-Fenster)
        "Titel online erkennen" => "Recognize tracks online",
        "„Online suchen“ im Tag-Fenster erlauben" => "Allow “Search online” in the tag window",
        "Nur auf Knopfdruck: Ein akustischer Fingerabdruck und die Länge gehen an AcoustID, ohne Treffer Interpret und Titel an MusicBrainz, für das Cover die Album-Kennung an das Cover Art Archive. Die Musikdatei bleibt auf deinem Rechner."
            => "Only on request: an acoustic fingerprint and the length go to AcoustID, without a match artist and title go to MusicBrainz, and the album ID goes to the Cover Art Archive for the cover. The music file stays on your computer.",
        "Cover" => "Cover",
        "Neues Cover wird beim Speichern in die Datei geschrieben." => "The new cover will be written to the file when you save.",
        "Online suchen" => "Search online",
        "Erkennt den Titel am Klang (AcoustID) und schlägt Tags aus MusicBrainz vor" => "Recognizes the track by its sound (AcoustID) and suggests tags from MusicBrainz",
        "Online erkennen" => "Recognize online",
        "FadeWerk DJ berechnet aus dem Titel einen akustischen Fingerabdruck und schickt ihn mit der Länge an AcoustID; ohne Treffer gehen Interpret und Titel an MusicBrainz. Die Musikdatei selbst bleibt auf deinem Rechner. Die Dienste sehen dabei deine IP-Adresse."
            => "FadeWerk DJ computes an acoustic fingerprint of the track and sends it with the length to AcoustID; without a match, artist and title go to MusicBrainz. The music file itself stays on your computer. The services see your IP address.",
        "Erlauben und suchen" => "Allow and search",
        "Nichts gefunden. Remixes, Edits und seltene Aufnahmen kennen die Dienste oft nicht." => "Nothing found. Remixes, edits and rare recordings are often unknown to these services.",
        "Klang" => "sound",
        "Text" => "text",
        "jetzt" => "now",
        "neu" => "new",
        "neues gewählt" => "new one chosen",
        "keins" => "none",
        "keins gefunden" => "none found",
        "Übernehmen" => "Apply",
        "Trägt die angehakten Werte in die Felder ein; geschrieben wird erst mit „Speichern“" => "Fills the checked values into the fields; nothing is written until you click “Save”",
        "Verwerfen" => "Discard",
        "Daten: MusicBrainz, AcoustID, Cover Art Archive" => "Data: MusicBrainz, AcoustID, Cover Art Archive",
        "Single" => "Single",
        "EP" => "EP",
        "Zusammenstellung" => "Compilation",
        "Live" => "Live",
        "Soundtrack" => "Soundtrack",
        "Remix" => "Remix",
        "DJ-Mix" => "DJ mix",
        "Sonstiges" => "Other",
        // Tags online ergänzen (ganze Ordner)
        "Tags der Titel in der Liste online ergänzen (erst Übersicht, dann schreiben)" => "Complete the tags of the tracks in the list online (overview first, then write)",
        "Tags der ausgewählten Titel online ergänzen (erst Übersicht, dann schreiben)" => "Complete the tags of the selected tracks online (overview first, then write)",
        "Tags online" => "Tags online",
        "Erkennt alle Titel am Klang und ergänzt leere oder offensichtlich falsche Tags. Vorher siehst du eine Übersicht." => "Recognizes all tracks by their sound and fills in empty or obviously wrong tags. You see an overview first.",
        "Tags online ergänzen …" => "Complete tags online …",
        "Tags online ergänzen" => "Complete tags online",
        "Für jeden Titel wird ein akustischer Fingerabdruck berechnet und mit der Länge an AcoustID geschickt; Genres kommen von MusicBrainz, Cover aus dem Cover Art Archive. Die Musikdateien bleiben auf deinem Rechner. Geschrieben wird erst, wenn du es bestätigst."
            => "For each track an acoustic fingerprint is computed and sent with the length to AcoustID; genres come from MusicBrainz, covers from the Cover Art Archive. The music files stay on your computer. Nothing is written until you confirm.",
        "Erkennung starten" => "Start recognition",
        "Erlauben und starten" => "Allow and start",
        "Sicher ({})" => "Certain ({})",
        "Unsicher ({})" => "Uncertain ({})",
        "Nicht gefunden ({})" => "Not found ({})",
        "Übersprungen ({})" => "Skipped ({})",
        "Treffer passt zu Länge und vorhandenen Angaben. Ergänzt werden nur leere oder offensichtlich falsche Felder." => "The match fits the length and the existing tags. Only empty or obviously wrong fields are filled in.",
        "Nicht eindeutig (z. B. Remix, B-Seite oder Angaben passen nicht). Mit „Prüfen …“ im Tag-Fenster ansehen." => "Not unambiguous (e.g. remix, B-side or the tags don't fit). Use “Review …” to look at it in the tag window.",
        "Die Dienste kennen diese Aufnahmen nicht. Hier ändert sich nichts." => "The services don't know these recordings. Nothing changes here.",
        "Schon früher erkannt (MusicBrainz-Kennung in der Datei) oder nicht lesbar." => "Recognized before (MusicBrainz ID in the file) or not readable.",
        "Geht, sobald die Erkennung fertig ist oder gestoppt wurde" => "Possible once the recognition has finished or was stopped",
        "Schreibt die angehakten Titel; die alten Werte merkt sich ein Journal zum Rückgängigmachen" => "Writes the checked tracks; a journal keeps the old values so you can undo",
        "Rückgängig machen" => "Undo",
        "Setzt die damals geschriebenen Felder zurück, sofern sie seitdem niemand geändert hat, und entfernt ergänzte Cover" => "Restores the fields written back then, unless they were changed since, and removes added covers",
        "schon erkannt" => "recognized before",
        "nicht gefunden" => "not found",
        "Prüfen …" => "Review …",
        "erledigt" => "done",
        "Fehler" => "Error",
        "spielt gerade" => "playing",
        "lädt …" => "loading …",
        "nur Kennung" => "ID only",
        "schon erkannt, Cover fehlt" => "recognized before, cover missing",
        "{} Titel zurückgesetzt." => "{} tracks restored.",
        "{} Titel geschrieben, {} Fehler." => "{} tracks written, {} errors.",
        "{} Titel geschrieben." => "{} tracks written.",
        "  · noch ca. {} s" => "  · about {} s left",
        "  · noch ca. {} min" => "  · about {} min left",
        "Angehakte schreiben ({})" => "Write checked ({})",
        "Letzter Durchlauf: {} · {} Titel" => "Last run: {} · {} tracks",
        "Trefferquote unter 90 %" => "match below 90 %",
        "Länge weicht ab" => "length differs",
        "Tags nennen etwas anderes" => "tags say something else",
        "Dateiname nennt etwas anderes" => "file name says something else",
        "anderer Titel genauso wahrscheinlich" => "another track equally likely",
        "Lädt Genres und Cover …" => "Loading genres and covers …",
        "Berechnet Fingerabdruck …" => "Computing fingerprint …",
        "Fragt AcoustID …" => "Asking AcoustID …",
        "Sucht bei MusicBrainz …" => "Searching MusicBrainz …",
        "Keine Verbindung zum Erkennungsdienst. Ist der Rechner online?" => "No connection to the recognition service. Is the computer online?",
        "MusicBrainz ist gerade überlastet. Bitte gleich noch einmal versuchen." => "MusicBrainz is busy right now. Please try again in a moment.",
        "Lizenztexte" => "License texts",
        "Gehäuseflächen wirken wie gebürstetes Metall in den Farben des Skins." => "Panels look like brushed metal in the skin's colours.",
        "Grafik über OpenGL" => "Graphics via OpenGL",
        "Zoom" => "Zoom",
        "Ganze Oberfläche kleiner oder größer, auch mit ⌘ + und ⌘ − (⌘ 0 = 100 %)." => "Make the whole interface smaller or larger, also with ⌘ + and ⌘ − (⌘ 0 = 100 %).",
        "Ganze Oberfläche kleiner oder größer, auch mit Strg + und Strg − (Strg 0 = 100 %)." => "Make the whole interface smaller or larger, also with Ctrl + and Ctrl − (Ctrl 0 = 100 %).",
        "Ziehen ändert die Höhe der Player, Doppelklick stellt sie wieder automatisch ein" => "Drag to change the height of the players, double-click to make it automatic again",
        "Für Rechner, auf denen das Fenster nicht zuverlässig startet (etwa virtuelle Maschinen). Wirkt nach einem Neustart." => "For computers where the window does not start reliably (such as virtual machines). Takes effect after a restart.",
        "Logdatei:" => "Log file:",
        "Updates" => "Updates",
        "Beim Start bei GitHub nach einer neuen Version suchen" => "Check GitHub for a new version at startup",
        "Gesendet wird nur die Frage nach der neuesten Version, keine Daten über Titel oder Ordner. GitHub sieht dabei wie bei jedem Webseitenaufruf deine IP-Adresse." => {
            "Only the request for the latest version is sent, no data about tracks or folders. As with any website visit, GitHub sees your IP address."
        }
        "Beim Start nach neuen Versionen suchen?" => "Check for new versions at startup?",
        "Dabei fragt FadeWerk DJ nur bei GitHub nach der neuesten Version – ohne Daten über deine Musik." => {
            "FadeWerk DJ only asks GitHub for the latest version – without any data about your music."
        }
        "Ja, prüfen" => "Yes, check",
        "Nein" => "No",
        "FadeWerk DJ {} ist verfügbar (installiert: {})." => "FadeWerk DJ {} is available (installed: {}).",
        "Download-Seite öffnen" => "Open download page",
        "FadeWerk DJ wurde auf Version {} aktualisiert." => "FadeWerk DJ was updated to version {}.",
        "FadeWerk DJ {} ist die neueste Version." => "FadeWerk DJ {} is the latest version.",
        "GitHub ist nicht erreichbar, bitte später noch einmal versuchen." => "GitHub is not reachable, please try again later.",
        "Lade FadeWerk DJ {} …" => "Downloading FadeWerk DJ {} …",
        "Installiere FadeWerk DJ {} – die App startet gleich neu …" => "Installing FadeWerk DJ {} – the app restarts in a moment …",
        "Update fehlgeschlagen: {}" => "Update failed: {}",
        "Die App startet dabei neu. Erst, wenn keine Musik mehr läuft." => "The app restarts for this. Possible once no music is playing.",
        "Jetzt aktualisieren" => "Update now",
        "Jetzt nach Updates suchen" => "Check for updates now",
        "Eine neue Version installiert sich per Knopf selbst; die App startet danach neu. Auf dem Mac nur, wenn sie mit demselben Zertifikat signiert ist." => "A new version installs itself at the push of a button; the app restarts afterwards. On the Mac only if it is signed with the same certificate.",
        "Später" => "Later",
        "Logdatei nicht gefunden: {}" => "Log file not found: {}",
        "Einzelheiten stehen in der Logdatei:\n{}" => "Details are in the log file:\n{}",
        "FadeWerk DJ wurde wegen eines Fehlers beendet:\n\n{}\n({})" => "FadeWerk DJ was closed because of an error:\n\n{}\n({})",
        "Das Fenster ließ sich nicht öffnen:\n{}" => "The window could not be opened:\n{}",
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

    /// Liest die String-Literale aller `tr("…")`- und `trf("…", …)`-Aufrufe aus dem Quellcode.
    fn literal_keys(src: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for call in ["tr(\"", "trf(\""] {
            let mut rest = src;
            while let Some(i) = rest.find(call) {
                let before = rest[..i].chars().next_back();
                rest = &rest[i + call.len()..];
                // Nur echte Aufrufe, nicht etwa `attr("`
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut key = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => key.push('\n'),
                            Some('t') => key.push('\t'),
                            Some('\n') => {
                                // Zeilenfortsetzung: führende Leerzeichen der nächsten Zeile fallen weg
                                let tail = chars.as_str().trim_start();
                                chars = tail.chars();
                            }
                            Some(other) => key.push(other),
                            None => break,
                        },
                        c => key.push(c),
                    }
                }
                keys.push(key);
            }
        }
        keys
    }

    /// Jeder feste Oberflächentext im Code hat eine englische Übersetzung. Schlägt fehl, sobald ein neuer
    /// Text ohne Eintrag in `en()` dazukommt.
    #[test]
    fn every_literal_ui_text_has_an_english_translation() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut found = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for key in literal_keys(&src) {
                found += 1;
                if key.chars().any(char::is_alphabetic) && en(&key).is_none() {
                    missing.push(format!("{}: {key:?}", path.file_name().unwrap().to_string_lossy()));
                }
            }
        }
        // Schutz davor, dass der Test wegen eines Lesefehlers nichts mehr findet
        assert!(found > 150, "nur {found} Texte gefunden");
        assert!(missing.is_empty(), "Englische Übersetzung fehlt:\n{}", missing.join("\n"));
    }

    /// Texte, die über Tabellen statt als Literal in `tr()` laufen.
    #[test]
    fn listed_names_have_english_translations() {
        let names = crate::browser::TrackColumn::ALL
            .iter()
            .map(|c| c.name())
            .chain(crate::search::Field::ALL.iter().map(|f| f.name()))
            .chain(crate::fx::BUILTIN_NAMES)
            .chain(crate::fx_icons::FxIcon::ALL.iter().map(|i| i.name()))
            .chain(crate::mixer_ui::CenterTab::ALL.iter().map(|t| t.label()))
            .chain(["Titel"]);
        for n in names {
            assert!(en(n).is_some(), "Englische Übersetzung fehlt: {n:?}");
        }
    }
}
