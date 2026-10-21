// SPDX-License-Identifier: GPL-3.0-or-later
//! Echtzeit-Wächter: Während `Engine::process` darf kein Speicher angefordert oder freigegeben werden
//! (das kann im Audio-Thread blockieren und hörbare Aussetzer verursachen). Ein zählender Allokator
//! schlägt Alarm, sobald es doch passiert.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fw_engine::{Command, Curve, Engine, HeadMode, TrackAudio, PREVIEW, head_link};

struct Watch;

static EVENTS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn note() {
    if ARMED.with(|a| a.get()) {
        EVENTS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Watch {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static WATCH: Watch = Watch;

fn tone(rate: u32, secs: f32) -> Arc<TrackAudio> {
    let n = (rate as f32 * secs) as usize;
    Arc::new(TrackAudio::new(rate, (0..n).map(|i| { let v = (i as f32 * 0.05).sin() * 0.5; [v, v] }).collect()))
}

/// Rechnet `blocks` Blöcke und zählt dabei Speicheraktionen im „Audio-Thread“.
fn process(engine: &mut Engine, out: &mut [f32], channels: usize, blocks: usize) -> usize {
    let before = EVENTS.load(Ordering::Relaxed);
    for _ in 0..blocks {
        ARMED.with(|a| a.set(true));
        engine.process(out, channels);
        ARMED.with(|a| a.set(false));
    }
    EVENTS.load(Ordering::Relaxed) - before
}

#[test]
fn audio_thread_never_allocates() {
    // Gegenprobe: der Wächter bemerkt eine Speicheranforderung
    let before = EVENTS.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    let v: Vec<u8> = std::hint::black_box(Vec::with_capacity(64));
    ARMED.with(|a| a.set(false));
    drop(v);
    assert!(EVENTS.load(Ordering::Relaxed) > before, "Wächter zählt nicht");

    let channels = 4;
    let (mut engine, mut ctl) = Engine::new(48_000, channels);
    let (sink, _receiver) = head_link(48_000, 44_100, 2048);
    let mut out = vec![0.0f32; 512 * channels];
    let mut check = |engine: &mut Engine, ctl: &mut fw_engine::Controller, what: &str, cmds: Vec<Command>| {
        for c in cmds {
            ctl.send(c);
        }
        let n = process(engine, &mut out, channels, 40);
        // Ausgeworfene Titel gibt die Oberfläche frei, außerhalb des Audio-Threads
        ctl.collect_garbage();
        assert_eq!(n, 0, "{what}: {n} Speicheraktionen im Audio-Thread");
    };

    check(&mut engine, &mut ctl, "Laden und Abspielen", vec![
        Command::SetHeadSink(Some(sink)),
        Command::SetHeadMode(HeadMode::Device),
        Command::Load { deck: 0, id: 1, track: tone(44_100, 3.0), keep_playing: false, start_secs: 0.0 },
        Command::Load { deck: 1, id: 2, track: tone(48_000, 3.0), keep_playing: false, start_secs: 0.5 },
        Command::Load { deck: PREVIEW, id: 3, track: tone(22_050, 3.0), keep_playing: false, start_secs: 0.0 },
        Command::Play { deck: 0 },
        Command::Play { deck: 1 },
        Command::Play { deck: PREVIEW },
        Command::SetPfl { deck: 1, on: true },
    ]);
    check(&mut engine, &mut ctl, "Mixer, EQ, Kurve", vec![
        Command::SetCrossfader(0.3),
        Command::SetCurve(Curve::Custom([0.9, 0.7, 0.4, 0.1])),
        Command::SetSplitFade(true),
        Command::SetEq { deck: 0, band: 0, gain: 0.0 },
        Command::SetEq { deck: 1, band: 2, gain: 1.5 },
        Command::SetFilter { deck: 0, value: -0.6 },
        Command::SetFilter { deck: 1, value: 0.8 },
        Command::SetRecording(true),
        Command::SetHeadMode(HeadMode::Split),
    ]);
    check(&mut engine, &mut ctl, "Scratch", vec![
        Command::ScratchBegin { deck: 0 },
        Command::ScratchTo { deck: 0, secs: 0.2 },
        Command::ScratchTo { deck: 0, secs: 1.5 },
    ]);
    check(&mut engine, &mut ctl, "Loslassen mit Schwung", vec![Command::ScratchEnd { deck: 0 }]);
    check(&mut engine, &mut ctl, "Loop, Reverse, Repeat", vec![
        Command::SetLoop { deck: 1, range: Some((0.5, 0.8)) },
        Command::SetReverse { deck: 0, on: true },
        Command::SetRepeatTrack { deck: 1, on: true },
        Command::SetRepeatFade(1.0),
    ]);
    check(&mut engine, &mut ctl, "Effekt-Pads", vec![
        Command::LoadFx { slot: 0, sound: tone(44_100, 0.5) },
        Command::LoadFx { slot: 7, sound: tone(48_000, 0.5) },
        Command::TriggerFx { slot: 0, to_master: true },
        Command::TriggerFx { slot: 7, to_master: false },
        Command::TestTone { to_master: true },
        Command::TestTone { to_master: false },
    ]);
    check(&mut engine, &mut ctl, "Neuer Titel ersetzt den alten", vec![
        Command::Load { deck: 1, id: 4, track: tone(44_100, 0.2), keep_playing: true, start_secs: 0.0 },
        Command::LoadFx { slot: 0, sound: tone(44_100, 0.3) },
    ]);
    check(&mut engine, &mut ctl, "Auswerfen und Titelende", vec![
        Command::Eject { deck: 0 },
        Command::SetLoop { deck: 1, range: None },
        Command::SetRepeatTrack { deck: 1, on: false },
        Command::Stop { deck: PREVIEW },
    ]);
}
