// SPDX-License-Identifier: GPL-3.0-or-later
//! Systemwerte für die Statuszeile: CPU-Last, Arbeitsspeicher, Temperatur und Akku.
//!
//! Gelesen im Thread `fw-sysmon` alle `INTERVAL`, nur was die Statuszeile gerade zeigt. Die Temperatur gibt es
//! nicht überall (Windows meist nur mit Adminrechten); ob sie lesbar ist, steht nach der ersten Abfrage fest.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(2);

/// Was gelesen werden soll (Bitmaske in `Monitor::wanted`).
pub const CPU: u8 = 1;
pub const RAM: u8 = 2;
pub const TEMP: u8 = 4;
pub const BATTERY: u8 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Battery {
    /// Ladestand 0 bis 100.
    pub percent: f32,
    /// Hängt am Netz (lädt oder voll).
    pub plugged: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// CPU-Last aller Kerne, 0 bis 100.
    pub cpu: Option<f32>,
    /// Belegter Arbeitsspeicher, 0 bis 100.
    pub ram: Option<f32>,
    /// Temperatur des Prozessors in °C.
    pub temp: Option<f32>,
    /// `None` = noch nicht gefragt, `Some(false)` = das System liefert keine.
    pub temp_available: Option<bool>,
    /// `None` = kein Akku (oder nicht lesbar).
    pub battery: Option<Battery>,
    /// `None` = noch nicht gefragt, `Some(false)` = kein Akku.
    pub battery_available: Option<bool>,
}

pub struct Monitor {
    stats: Arc<Mutex<Stats>>,
    wanted: Arc<AtomicU8>,
}

impl Monitor {
    pub fn start(repaint: impl Fn() + Send + 'static) -> Self {
        let stats = Arc::new(Mutex::new(Stats::default()));
        let wanted = Arc::new(AtomicU8::new(0));
        let (s, w) = (Arc::clone(&stats), Arc::clone(&wanted));
        let spawned = std::thread::Builder::new().name("fw-sysmon".into()).spawn(move || run(&s, &w, repaint));
        if let Err(e) = spawned {
            log::warn!("Systemwerte: Thread startet nicht: {e}");
        }
        Monitor { stats, wanted }
    }

    /// Welche Werte die Statuszeile zeigt (`CPU | RAM | …`); Temperatur und Akku werden für die Einstellungen
    /// einmal gefragt, auch wenn sie aus sind.
    pub fn set_wanted(&self, mask: u8) {
        self.wanted.store(mask, Ordering::Relaxed);
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn run(stats: &Mutex<Stats>, wanted: &AtomicU8, repaint: impl Fn()) {
    let mut sys = sysinfo::System::new().map_err(|e| log::warn!("Systemwerte nicht lesbar: {e}")).ok();
    // Sensorliste einmal anlegen (`None` = noch nicht versucht, `Some(None)` = gibt es nicht)
    let mut comps: Option<Option<sysinfo::Components>> = None;
    let batteries = starship_battery::Manager::new().ok();
    let mut first = true;
    loop {
        let mask = wanted.load(Ordering::Relaxed);
        let mut next = stats.lock().map(|s| s.clone()).unwrap_or_default();
        next.cpu = sys.as_mut().filter(|_| mask & CPU != 0).map(|sys| {
            sys.refresh_cpu_usage();
            sys.global_cpu_usage()
        });
        next.ram = sys.as_mut().filter(|_| mask & RAM != 0).map(|sys| {
            sys.refresh_memory();
            let total = sys.total_memory().max(1) as f32;
            100.0 * sys.used_memory() as f32 / total
        });
        // Beim ersten Mal immer, damit die Einstellungen wissen, ob es eine Temperatur gibt
        if mask & TEMP != 0 || first {
            let c = comps.get_or_insert_with(|| sysinfo::Components::new_with_refreshed_list().ok());
            next.temp = c.as_mut().and_then(|c| {
                c.refresh(false);
                cpu_temperature(c)
            });
            next.temp_available = Some(next.temp.is_some() || next.temp_available == Some(true));
            if mask & TEMP == 0 {
                next.temp = None;
            }
        } else {
            next.temp = None;
        }
        if mask & BATTERY != 0 || first {
            let b = read_battery(batteries.as_ref());
            next.battery_available = Some(b.is_some());
            next.battery = if mask & BATTERY != 0 { b } else { None };
        } else {
            next.battery = None;
        }
        first = false;
        if let Ok(mut s) = stats.lock() {
            *s = next;
        }
        repaint();
        std::thread::sleep(INTERVAL);
    }
}

/// Temperatur des Prozessors: Sensoren mit CPU-Namen bevorzugt (je nach System „CPU“, „Package“, „Tdie“,
/// „PMU tdie“, „coretemp“, „k10temp“), sonst der heißeste Sensor. Unsinnige Werte fallen weg.
fn cpu_temperature(c: &sysinfo::Components) -> Option<f32> {
    let valid = |t: f32| t.is_finite() && (5.0..130.0).contains(&t);
    let all: Vec<(String, f32)> = c.iter().filter_map(|x| Some((x.label().to_lowercase(), x.temperature().filter(|&t| valid(t))?))).collect();
    let cpu_like = |l: &str| ["cpu", "package", "tdie", "tctl", "coretemp", "k10temp", "core ", "soc"].iter().any(|k| l.contains(k));
    let pick = |it: &mut dyn Iterator<Item = f32>| it.fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.max(t))));
    pick(&mut all.iter().filter(|(l, _)| cpu_like(l)).map(|(_, t)| *t)).or_else(|| pick(&mut all.iter().map(|(_, t)| *t)))
}

fn read_battery(m: Option<&starship_battery::Manager>) -> Option<Battery> {
    use starship_battery::State;
    let b = m?.batteries().ok()?.flatten().next()?;
    let percent = b.state_of_charge().get::<starship_battery::units::ratio::percent>();
    let plugged = matches!(b.state(), State::Charging | State::Full | State::Paused);
    percent.is_finite().then_some(Battery { percent: percent.clamp(0.0, 100.0), plugged })
}
