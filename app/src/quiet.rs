// SPDX-License-Identifier: GPL-3.0-or-later
//! Andere Geräusche dämpfen, solange FadeWerk DJ läuft (Einstellung „Andere Geräusche stumm schalten“).
//!
//! - Systemtöne: macOS Hinweislautstärke auf 0, Windows die Sitzung „Systemsounds“, Linux (GNOME) die
//!   Ereignisklänge.
//! - Andere Programme: Windows über die Sitzungen des Lautstärkemixers (WASAPI), Linux über `pactl`
//!   (PulseAudio/PipeWire). macOS kennt dafür keine Schnittstelle; dort hilft nur der Exklusivmodus
//!   ([`set_exclusive`]), der das Ausgabegerät für FadeWerk DJ allein belegt.
//!
//! Läuft im Thread `fw-quiet` und schaut alle [`REFRESH`] nach neuen Programmen. Was verändert wurde, steht
//! in `stumm.json` im Einstellungsordner; nach einem Absturz stellt der nächste Start es wieder her.
//! Windows und PulseAudio merken sich die Stummschaltung je Programm auch über dessen Neustart hinaus:
//! Programme, die beim Zurückstellen nicht laufen, bleiben vorgemerkt und werden beim nächsten Mal
//! zurückgestellt, sobald sie wieder Ton haben.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Wie oft nach neu aufgetauchten Programmen geschaut wird.
const REFRESH: Duration = Duration::from_secs(2);
const MARKER: &str = "stumm.json";

/// Was FadeWerk DJ verändert hat und zurückstellen muss.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Changes {
    /// macOS: Hinweislautstärke vorher (0 bis 100).
    alert_volume: Option<u32>,
    /// GNOME: Ereignisklänge waren an.
    event_sounds: Option<bool>,
    /// Windows/Linux: Kennungen der stumm geschalteten Programme (Windows `GetSessionIdentifier`,
    /// Linux `application.process.binary` bzw. Name).
    muted: Vec<String>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.alert_volume.is_none() && self.event_sounds.is_none() && self.muted.is_empty()
    }
}

enum Msg {
    Set(bool),
}

pub struct Quiet {
    tx: Option<Sender<Msg>>,
    handle: Option<JoinHandle<()>>,
}

impl Quiet {
    /// Startet den Thread; `dir` ist der Einstellungsordner (`None` = nichts merken, etwa bei Entwicklerläufen).
    /// Reste eines abgestürzten Laufs werden zuerst zurückgestellt.
    pub fn start(dir: Option<PathBuf>, on: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new().name("fw-quiet".into()).spawn(move || run(dir, on, rx)).ok();
        Self { tx: Some(tx), handle }
    }

    pub fn set(&self, on: bool) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Set(on));
        }
    }
}

impl Drop for Quiet {
    /// Beim Beenden alles zurückstellen (der Thread endet, sobald der Kanal zu ist).
    fn drop(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(dir: Option<PathBuf>, mut on: bool, rx: Receiver<Msg>) {
    let marker = dir.map(|d| d.join(MARKER));
    let mut changes: Changes = marker.as_ref().and_then(|m| std::fs::read_to_string(m).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if !changes.is_empty() {
        log::info!("Stummschaltung eines früheren Laufs wird zurückgestellt");
        restore(&mut changes);
    }
    loop {
        if on {
            engage(&mut changes);
        } else if !changes.muted.is_empty() {
            // Vorgemerkte Programme zurückstellen, sobald sie wieder Ton haben
            restore(&mut changes);
        }
        save(marker.as_ref(), &changes);
        match rx.recv_timeout(REFRESH) {
            Ok(Msg::Set(v)) => {
                if on && !v {
                    restore(&mut changes);
                }
                on = v;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    restore(&mut changes);
    save(marker.as_ref(), &changes);
}

fn save(marker: Option<&PathBuf>, changes: &Changes) {
    let Some(m) = marker else { return };
    if changes.is_empty() {
        let _ = std::fs::remove_file(m);
    } else if let Ok(text) = serde_json::to_string_pretty(changes) {
        let _ = crate::settings::write_atomic(m, &text);
    }
}

// ---------- macOS ----------

#[cfg(target_os = "macos")]
fn engage(c: &mut Changes) {
    if c.alert_volume.is_none()
        && let Some(v) = osascript("alert volume of (get volume settings)").and_then(|s| s.trim().parse::<u32>().ok())
        && v > 0
        && osascript("set volume alert volume 0").is_some()
    {
        c.alert_volume = Some(v);
    }
}

#[cfg(target_os = "macos")]
fn restore(c: &mut Changes) {
    if let Some(v) = c.alert_volume
        && osascript(&format!("set volume alert volume {v}")).is_some()
    {
        c.alert_volume = None;
    }
    c.event_sounds = None;
    c.muted.clear();
}

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> Option<String> {
    let out = std::process::Command::new("osascript").args(["-e", script]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Exklusivmodus: belegt das Ausgabegerät (`id` wie von cpal, `coreaudio:<UID>`) für diesen Prozess allein
/// („Hog Mode“), andere Programme bleiben darauf stumm. `false` gibt es frei. Endet der Prozess, gibt
/// macOS das Gerät von selbst frei.
#[cfg(target_os = "macos")]
pub fn set_exclusive(id: &str, on: bool) -> Result<(), String> {
    let uid = id.strip_prefix("coreaudio:").unwrap_or(id);
    let device = coreaudio::device_by_uid(uid).ok_or_else(|| format!("Gerät {uid} nicht gefunden"))?;
    let pid: i32 = if on { std::process::id() as i32 } else { -1 };
    // Nur wenn nötig: Setzen schaltet bei manchen Treibern hin und her
    let owner = coreaudio::hog_owner(device);
    if on && owner == Some(pid) || !on && owner != Some(std::process::id() as i32) {
        return Ok(());
    }
    coreaudio::set_hog(device, pid)?;
    if on && coreaudio::hog_owner(device) != Some(pid) {
        return Err("das Gerät lässt sich nicht exklusiv belegen".into());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn set_exclusive(_id: &str, _on: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod coreaudio {
    //! Die wenigen CoreAudio-Aufrufe für den Exklusivmodus, direkt gegen das Framework.
    use std::ffi::c_void;

    type ObjectId = u32;
    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }
    const SYSTEM_OBJECT: ObjectId = 1;
    const DEVICES: u32 = u32::from_be_bytes(*b"dev#");
    const DEVICE_UID: u32 = u32::from_be_bytes(*b"uid ");
    const HOG_MODE: u32 = u32::from_be_bytes(*b"oink");
    const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(id: ObjectId, addr: *const Address, qsize: u32, q: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(id: ObjectId, addr: *const Address, qsize: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
        fn AudioObjectSetPropertyData(id: ObjectId, addr: *const Address, qsize: u32, q: *const c_void, size: u32, data: *const c_void) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, len: isize, encoding: u32) -> u8;
        fn CFRelease(obj: *const c_void);
    }

    fn addr(selector: u32) -> Address {
        Address { selector, scope: SCOPE_GLOBAL, element: 0 }
    }

    pub fn device_by_uid(uid: &str) -> Option<ObjectId> {
        let a = addr(DEVICES);
        let mut size = 0u32;
        // SAFETY: gültige Adresse und Größenzeiger, keine Qualifier-Daten
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &a, 0, std::ptr::null(), &mut size) } != 0 {
            return None;
        }
        let mut ids = vec![0u32; size as usize / 4];
        // SAFETY: Puffer ist `size` Bytes groß
        if unsafe { AudioObjectGetPropertyData(SYSTEM_OBJECT, &a, 0, std::ptr::null(), &mut size, ids.as_mut_ptr().cast()) } != 0 {
            return None;
        }
        ids.truncate(size as usize / 4);
        ids.into_iter().find(|&d| device_uid(d).as_deref() == Some(uid))
    }

    fn device_uid(device: ObjectId) -> Option<String> {
        let a = addr(DEVICE_UID);
        let mut cf: *const c_void = std::ptr::null();
        let mut size = std::mem::size_of::<*const c_void>() as u32;
        // SAFETY: Ergebnis ist eine CFStringRef, die wir freigeben
        let status = unsafe { AudioObjectGetPropertyData(device, &a, 0, std::ptr::null(), &mut size, (&mut cf as *mut *const c_void).cast()) };
        if status != 0 || cf.is_null() {
            return None;
        }
        let mut buf = [0u8; 512];
        // SAFETY: Puffer mit Länge, `cf` ist gültig bis CFRelease
        let ok = unsafe { CFStringGetCString(cf, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        unsafe { CFRelease(cf) };
        if ok == 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    pub fn hog_owner(device: ObjectId) -> Option<i32> {
        let a = addr(HOG_MODE);
        let mut pid: i32 = -1;
        let mut size = 4u32;
        // SAFETY: vier Bytes für eine pid_t
        let status = unsafe { AudioObjectGetPropertyData(device, &a, 0, std::ptr::null(), &mut size, (&mut pid as *mut i32).cast()) };
        (status == 0 && pid != -1).then_some(pid)
    }

    pub fn set_hog(device: ObjectId, pid: i32) -> Result<(), String> {
        let a = addr(HOG_MODE);
        // SAFETY: vier Bytes für eine pid_t
        let status = unsafe { AudioObjectSetPropertyData(device, &a, 0, std::ptr::null(), 4, (&pid as *const i32).cast()) };
        if status == 0 { Ok(()) } else { Err(format!("CoreAudio-Fehler {status}")) }
    }
}

// ---------- Windows ----------

#[cfg(windows)]
fn engage(c: &mut Changes) {
    if let Err(e) = windows_sessions::mute_others(&mut c.muted) {
        log::debug!("Andere Programme stumm schalten: {e}");
    }
}

#[cfg(windows)]
fn restore(c: &mut Changes) {
    if let Err(e) = windows_sessions::unmute(&mut c.muted) {
        log::debug!("Stummschaltung zurückstellen: {e}");
    }
    c.alert_volume = None;
    c.event_sounds = None;
}

#[cfg(windows)]
mod windows_sessions {
    //! Sitzungen des Lautstärkemixers aller aktiven Ausgabegeräte (dazu gehört „Systemsounds“).
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};
    use windows::core::Interface;

    /// Alle Sitzungen außer der eigenen: (Kennung, Lautstärke-Schnittstelle).
    fn sessions() -> windows::core::Result<Vec<(String, ISimpleAudioVolume)>> {
        let own = std::process::id();
        let mut out = Vec::new();
        // SAFETY: COM-Aufrufe im eigenen Thread; Zeichenketten von COM werden mit CoTaskMemFree freigegeben
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let Ok(device) = devices.Item(i) else { continue };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let Ok(list) = manager.GetSessionEnumerator() else { continue };
                for j in 0..list.GetCount().unwrap_or(0) {
                    let Ok(session) = list.GetSession(j) else { continue };
                    let Ok(control) = session.cast::<IAudioSessionControl2>() else { continue };
                    if control.GetProcessId().is_ok_and(|pid| pid == own) {
                        continue;
                    }
                    let Ok(raw) = control.GetSessionIdentifier() else { continue };
                    let id = raw.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(raw.0 as *const _));
                    if let Ok(volume) = session.cast::<ISimpleAudioVolume>() {
                        out.push((id, volume));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn mute_others(muted: &mut Vec<String>) -> windows::core::Result<()> {
        for (id, volume) in sessions()? {
            // SAFETY: gültige Schnittstelle aus `sessions`
            unsafe {
                if !volume.GetMute()?.as_bool() && volume.SetMute(true, std::ptr::null()).is_ok() && !muted.contains(&id) {
                    muted.push(id);
                }
            }
        }
        Ok(())
    }

    /// Stellt die vorgemerkten Sitzungen zurück; nicht gefundene bleiben vorgemerkt.
    pub fn unmute(muted: &mut Vec<String>) -> windows::core::Result<()> {
        let mut done = Vec::new();
        for (id, volume) in sessions()? {
            if muted.contains(&id) {
                // SAFETY: gültige Schnittstelle aus `sessions`
                if unsafe { volume.SetMute(false, std::ptr::null()) }.is_ok() {
                    done.push(id);
                }
            }
        }
        muted.retain(|id| !done.contains(id));
        Ok(())
    }
}

// ---------- Linux ----------

#[cfg(target_os = "linux")]
fn engage(c: &mut Changes) {
    if c.event_sounds.is_none()
        && let Some(on) = gsettings_get()
        && on
        && run_ok("gsettings", &["set", "org.gnome.desktop.sound", "event-sounds", "false"])
    {
        c.event_sounds = Some(true);
    }
    let Some(list) = pactl_list() else { return };
    for input in parse_sink_inputs(&list, std::process::id()) {
        if !input.muted && run_ok("pactl", &["set-sink-input-mute", &input.index.to_string(), "1"]) && !c.muted.contains(&input.app) {
            c.muted.push(input.app);
        }
    }
}

#[cfg(target_os = "linux")]
fn restore(c: &mut Changes) {
    if c.event_sounds == Some(true) && run_ok("gsettings", &["set", "org.gnome.desktop.sound", "event-sounds", "true"]) {
        c.event_sounds = None;
    }
    c.alert_volume = None;
    let Some(list) = pactl_list() else { return };
    let mut done = Vec::new();
    for input in parse_sink_inputs(&list, std::process::id()) {
        if c.muted.contains(&input.app) && run_ok("pactl", &["set-sink-input-mute", &input.index.to_string(), "0"]) {
            done.push(input.app);
        }
    }
    c.muted.retain(|a| !done.contains(a));
}

#[cfg(target_os = "linux")]
fn pactl_list() -> Option<String> {
    let out = std::process::Command::new("pactl").env("LC_ALL", "C").args(["list", "sink-inputs"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn gsettings_get() -> Option<bool> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.sound", "event-sounds"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim() == "true")
}

#[cfg(target_os = "linux")]
fn run_ok(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program).args(args).output().is_ok_and(|o| o.status.success())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn engage(_c: &mut Changes) {}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn restore(c: &mut Changes) {
    *c = Changes::default();
}

/// Ein Tonstrom aus `pactl list sink-inputs` (mit `LC_ALL=C`).
#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct SinkInput {
    index: u32,
    muted: bool,
    /// Programm, an dem die Stummschaltung hängt (Programmdatei, sonst Name).
    app: String,
}

/// Tonströme anderer Programme (nicht `own_pid`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_sink_inputs(text: &str, own_pid: u32) -> Vec<SinkInput> {
    let mut out = Vec::new();
    for block in text.split("Sink Input #").skip(1) {
        let Some(index) = block.lines().next().and_then(|l| l.trim().parse::<u32>().ok()) else { continue };
        let prop = |key: &str| {
            block.lines().find_map(|l| {
                let (k, v) = l.trim().split_once(" = ")?;
                (k == key).then(|| v.trim_matches('"').to_owned())
            })
        };
        let muted = block.lines().any(|l| l.trim() == "Mute: yes");
        if prop("application.process.id").and_then(|p| p.parse::<u32>().ok()) == Some(own_pid) {
            continue;
        }
        let Some(app) = prop("application.process.binary").or_else(|| prop("application.name")) else { continue };
        out.push(SinkInput { index, muted, app });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_inputs_skip_own_process() {
        let text = r#"Sink Input #42
	Driver: protocol-native.c
	Mute: no
	Properties:
		application.name = "Firefox"
		application.process.id = "1000"
		application.process.binary = "firefox"

Sink Input #43
	Mute: yes
	Properties:
		application.name = "FadeWerk DJ"
		application.process.id = "7"

Sink Input #44
	Mute: yes
	Properties:
		application.name = "Spotify"
		application.process.id = "2000"
"#;
        let inputs = parse_sink_inputs(text, 7);
        assert_eq!(
            inputs,
            vec![SinkInput { index: 42, muted: false, app: "firefox".into() }, SinkInput { index: 44, muted: true, app: "Spotify".into() }]
        );
    }

    #[test]
    fn broken_marker_counts_as_nothing_changed() {
        let c: Changes = serde_json::from_str("{\"muted\": 5}").unwrap_or_default();
        assert!(c.is_empty());
        let c: Changes = serde_json::from_str("{\"alert_volume\": 80}").unwrap();
        assert_eq!(c.alert_volume, Some(80));
    }
}
