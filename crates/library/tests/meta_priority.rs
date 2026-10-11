// SPDX-License-Identifier: GPL-3.0-or-later
//! Tags sichtbarer Zeilen (`request_meta_now`) kommen vor allen wartenden Anfragen dran,
//! auch wenn eine Suche zuvor Tausende Titel angefordert hat.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fw_library::{Library, Response};

#[test]
fn visible_rows_jump_the_queue() {
    let dir = std::env::temp_dir().join(format!("fw-meta-prio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = |i: usize| -> PathBuf {
        let p = dir.join(format!("Interpret_Titel {i}.mp3"));
        std::fs::write(&p, b"kein echtes mp3").unwrap();
        p
    };
    let queued: Vec<PathBuf> = (0..5000).map(file).collect();
    let visible = file(99_999);

    let lib = Library::start(Arc::new(|| {}));
    for p in &queued {
        lib.request_meta(p.clone());
    }
    lib.request_meta_now(visible.clone());

    // Die Tag-Threads haben höchstens ein paar Anfragen schon in Arbeit; danach kommt die sichtbare Zeile
    let mut position = None;
    let mut n = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while position.is_none() && std::time::Instant::now() < deadline {
        for resp in lib.poll() {
            if let Response::Meta { path, .. } = resp {
                if path == visible {
                    position = Some(n);
                }
                n += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    std::fs::remove_dir_all(&dir).unwrap();
    let position = position.expect("sichtbare Zeile nie gelesen");
    assert!(position < 50, "sichtbare Zeile erst als {position}. Antwort");
}
