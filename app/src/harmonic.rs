// SPDX-License-Identifier: GPL-3.0-or-later
//! „Harmonisch“: kommende Titel beider Playlists so ordnen, dass jeder hörbare Übergang passt.
//!
//! Auto-Fade spielt abwechselnd aus A und B, gehört wird also A1 → B1 → A2 → B2. Ausgehend vom Titel auf
//! Sendung wird reihum aus der Playlist, die als Nächste dran ist, der passendste Titel gewählt (gierig,
//! Kosten siehe [`cost`] bzw. [`beat_cost`]). Titel bleiben in ihrer Playlist; ist eine leer, spielt die andere
//! allein weiter.
//!
//! „Im Takt“ ([`Mode::Beat`]) zählt zuerst, ob der Übergang im Takt laufen kann (Beatraster am Ende des alten und
//! am Anfang des neuen Titels, Tempo höchstens `BEAT_PCT` auseinander, wie `app::beatmatch`), dann der kleinste
//! Tempo-Sprung, die Tonart nur bei Gleichstand.

use fw_library::Camelot;

/// Was für die Wahl zählt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Track {
    pub key: Option<Camelot>,
    pub bpm: Option<f32>,
    /// Tempo laut Beatraster am Anfang bzw. am Ende (dort wird übergeblendet); `None` = kein sicheres Raster.
    pub grid_in: Option<f32>,
    pub grid_out: Option<f32>,
}

/// Wonach sortiert wird.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    /// Tonart zuerst, dann Tempo.
    Harmonic,
    /// Übergänge im Takt zuerst (Beatraster, Tempo), Tonart bei Gleichstand.
    Beat,
}

/// Höchste Tempoanpassung für einen Übergang im Takt (wie `app::beatmatch::BEAT_MAX_PCT`).
pub const BEAT_PCT: f32 = 6.0;

/// Tempounterschied in Prozent, auch halb/doppelt gerechnet (wie Auto-Sync es kann).
fn tempo_pct(x: f32, y: f32) -> f32 {
    [1.0f32, 2.0, 0.5].iter().map(|f| ((y * f) / x - 1.0).abs() * 100.0).fold(f32::MAX, f32::min)
}

/// Kann der Übergang von `a` nach `b` im Takt laufen?
pub fn on_beat(a: &Track, b: &Track) -> bool {
    matches!((a.grid_out, b.grid_in), (Some(x), Some(y)) if x > 0.0 && y > 0.0 && tempo_pct(x, y) <= BEAT_PCT)
}

/// Kosten für „im Takt“: im Takt möglich 0…1 (nach Tempo-Sprung) plus ein Zehntel der Tonart-Kosten; mit Raster,
/// aber zu weit auseinander ab 10; ohne Raster an der Stelle ab 20 (solche Titel rücken nach hinten).
pub fn beat_cost(a: &Track, b: &Track) -> f32 {
    let key = 0.1 * cost(&Track { bpm: None, ..*a }, &Track { bpm: None, ..*b }).min(20.0) / 20.0;
    match (a.grid_out, b.grid_in) {
        (Some(x), Some(y)) if x > 0.0 && y > 0.0 => {
            let pct = tempo_pct(x, y);
            if pct <= BEAT_PCT { pct / BEAT_PCT + key } else { 10.0 + pct / 8.0 + key }
        }
        _ => {
            let pct = match (a.grid_out.or(a.bpm), b.grid_in.or(b.bpm)) {
                (Some(x), Some(y)) if x > 0.0 && y > 0.0 => tempo_pct(x, y),
                _ => 50.0,
            };
            20.0 + pct / 8.0 + key
        }
    }
}

/// Kosten eines Übergangs von `a` nach `b`: zuerst die Tonart (gleich 0, Camelot-Nachbar oder Paralleltonart 1,
/// sonst 2 plus Abstand auf dem Rad), dann das Tempo (in Prozent, auch halb/doppelt, wie Auto-Sync es kann).
/// Ohne Tonart teuer, damit solche Titel ans Ende rücken.
pub fn cost(a: &Track, b: &Track) -> f32 {
    let key = match (a.key, b.key) {
        (Some(x), Some(y)) if x == y => 0.0,
        (Some(x), Some(y)) if x.compatible(y) => 1.0,
        (Some(x), Some(y)) => {
            let d = (x.number() as i32 - y.number() as i32).rem_euclid(12);
            2.0 + d.min(12 - d) as f32 + if x.is_minor() != y.is_minor() { 1.0 } else { 0.0 }
        }
        _ => 20.0,
    };
    let tempo = match (a.bpm, b.bpm) {
        // 8 % Unterschied wiegen etwa so viel wie ein Schritt auf dem Rad
        (Some(x), Some(y)) if x > 0.0 && y > 0.0 => tempo_pct(x, y) / 8.0,
        _ => 0.5,
    };
    key + tempo
}

/// Neue Reihenfolge der kommenden Titel beider Playlists (Indizes in `lists[0]` bzw. `lists[1]`).
/// `start` = Titel auf Sendung (ohne: der erste gewählte Titel ist der beste Anfang), `first` = welche
/// Playlist als Nächste spielt.
pub fn plan(start: Option<Track>, first: usize, lists: [&[Track]; 2], mode: Mode) -> [Vec<usize>; 2] {
    let price = |a: &Track, b: &Track| match mode {
        Mode::Harmonic => cost(a, b),
        Mode::Beat => beat_cost(a, b),
    };
    let mut left: [Vec<usize>; 2] = [(0..lists[0].len()).collect(), (0..lists[1].len()).collect()];
    let mut out: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    let mut cur = start;
    let mut turn = first.min(1);
    while !left[0].is_empty() || !left[1].is_empty() {
        if left[turn].is_empty() {
            turn = 1 - turn;
        }
        let pick = match cur {
            // Ohne Ausgangstitel: der erste mit bekannter Tonart bzw. (im Takt) mit Raster am Ende, sonst der erste
            None => left[turn]
                .iter()
                .position(|&i| match mode {
                    Mode::Harmonic => lists[turn][i].key.is_some(),
                    Mode::Beat => lists[turn][i].grid_out.is_some(),
                })
                .unwrap_or(0),
            Some(c) => {
                let mut best = (f32::MAX, 0);
                for (pos, &i) in left[turn].iter().enumerate() {
                    let k = price(&c, &lists[turn][i]);
                    // Bei Gleichstand die bisherige Reihenfolge behalten
                    if k < best.0 {
                        best = (k, pos);
                    }
                }
                best.1
            }
        };
        let i = left[turn].remove(pick);
        out[turn].push(i);
        cur = Some(lists[turn][i]);
        turn = 1 - turn;
    }
    out
}

/// Wie viele der gehörten Übergänge in dieser Reihenfolge im Takt laufen können, und wie viele es sind.
pub fn beat_matches(start: Option<Track>, first: usize, lists: [&[Track]; 2], order: &[Vec<usize>; 2]) -> (usize, usize) {
    let mut pos = [0usize; 2];
    let mut cur = start;
    let mut turn = first.min(1);
    let (mut ok, mut all) = (0, 0);
    while pos[0] < order[0].len() || pos[1] < order[1].len() {
        if pos[turn] >= order[turn].len() {
            turn = 1 - turn;
        }
        let t = lists[turn][order[turn][pos[turn]]];
        pos[turn] += 1;
        if let Some(c) = cur {
            all += 1;
            ok += usize::from(on_beat(&c, &t));
        }
        cur = Some(t);
        turn = 1 - turn;
    }
    (ok, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, bpm: f32) -> Track {
        Track { key: fw_library::parse_key(key), bpm: Some(bpm), ..Track::default() }
    }

    /// Titel mit Raster an beiden Enden.
    fn g(key: &str, bpm: f32) -> Track {
        Track { grid_in: Some(bpm), grid_out: Some(bpm), ..t(key, bpm) }
    }

    #[test]
    fn beat_mode_chains_tempos_and_puts_gridless_last() {
        let a = [g("1A", 140.0), Track { key: fw_library::parse_key("8A"), bpm: Some(120.0), ..Track::default() }, g("5B", 122.0)];
        let b = [g("12B", 160.0), g("8A", 121.0), g("3A", 141.0)];
        // Auf Sendung 120 BPM mit Raster aus A, als Nächstes spielt B
        let start = Some(g("8A", 120.0));
        let order = plan(start, 1, [&a, &b], Mode::Beat);
        let heard: Vec<f32> = (0..3).flat_map(|k| [b[order[1][k]].bpm.unwrap(), a[order[0][k]].bpm.unwrap()]).collect();
        assert_eq!(&heard[..2], &[121.0, 122.0], "erst die nahen Tempi");
        assert_eq!(order[0][2], 1, "Titel ohne Raster ans Ende");
        let (ok, all) = beat_matches(start, 1, [&a, &b], &order);
        assert_eq!(all, 6);
        // 120→121→122 und 141→140 im Takt; 122→141, 140→160 und der Titel ohne Raster nicht
        assert_eq!(ok, 3, "{ok} von {all}");
        // 70 und 140 passen zusammen
        assert!(on_beat(&g("1A", 70.0), &g("1A", 141.0)));
        assert!(!on_beat(&g("1A", 120.0), &g("1A", 130.0)));
    }

    #[test]
    fn costs_follow_the_wheel_and_the_tempo() {
        let a = t("8A", 124.0);
        assert!(cost(&a, &t("8A", 124.0)) < cost(&a, &t("9A", 124.0)));
        assert!(cost(&a, &t("9A", 124.0)) < cost(&a, &t("10A", 124.0)));
        assert!(cost(&a, &t("8B", 124.0)) < cost(&a, &t("3B", 124.0)));
        // Gleiche Tonart, aber 20 % schneller ist schlechter als ein Nachbar im gleichen Tempo
        assert!(cost(&a, &t("8A", 149.0)) > cost(&a, &t("9A", 124.0)));
        // Doppeltes Tempo gilt wie gleiches
        assert!(cost(&a, &t("8A", 248.0)) < 0.1);
        assert!(cost(&a, &Track { key: None, bpm: Some(124.0), ..Track::default() }) > cost(&a, &t("2B", 124.0)));
    }

    #[test]
    fn alternates_between_the_playlists_and_chains_neighbours() {
        let a = [t("3A", 120.0), t("8A", 120.0), t("10A", 120.0)];
        let b = [t("10A", 120.0), t("9A", 120.0), t("4A", 120.0)];
        // Auf Sendung 8A aus Playlist A, als Nächstes spielt B
        let [oa, ob] = plan(Some(t("8A", 120.0)), 1, [&a, &b], Mode::Harmonic);
        // B: 9A (Nachbar) → A: 8A oder 10A …
        assert_eq!(ob[0], 1, "9A passt zu 8A");
        let heard: Vec<String> = (0..3).flat_map(|k| [b[ob[k]].key.unwrap().label(), a[oa[k]].key.unwrap().label()]).collect();
        assert_eq!(heard, ["9A", "8A", "10A", "10A", "4A", "3A"]);
        // Jeder Index genau einmal
        let mut s = oa.clone();
        s.sort();
        assert_eq!(s, [0, 1, 2]);
    }

    #[test]
    fn unknown_keys_go_last_and_a_short_list_runs_out() {
        let a = [Track { key: None, bpm: Some(120.0), ..Track::default() }, t("8A", 120.0)];
        let b = [t("9A", 120.0)];
        let [oa, ob] = plan(Some(t("8A", 120.0)), 0, [&a, &b], Mode::Harmonic);
        assert_eq!(oa, [1, 0], "ohne Tonart ans Ende");
        assert_eq!(ob, [0]);
        let [oa, ob] = plan(None, 0, [&[], &[]], Mode::Harmonic);
        assert!(oa.is_empty() && ob.is_empty());
    }
}
