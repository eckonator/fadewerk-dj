// SPDX-License-Identifier: GPL-3.0-or-later
//! „Harmonisch“: kommende Titel beider Playlists so ordnen, dass jeder hörbare Übergang passt.
//!
//! Auto-Fade spielt abwechselnd aus A und B, gehört wird also A1 → B1 → A2 → B2. Ausgehend vom Titel auf
//! Sendung wird reihum aus der Playlist, die als Nächste dran ist, der passendste Titel gewählt (gierig,
//! Kosten siehe [`cost`]). Titel bleiben in ihrer Playlist; ist eine leer, spielt die andere allein weiter.

use fw_library::Camelot;

/// Was für die Wahl zählt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Track {
    pub key: Option<Camelot>,
    pub bpm: Option<f32>,
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
        (Some(x), Some(y)) if x > 0.0 && y > 0.0 => {
            let pct = [1.0f32, 2.0, 0.5].iter().map(|f| ((y * f) / x - 1.0).abs() * 100.0).fold(f32::MAX, f32::min);
            // 8 % Unterschied wiegen etwa so viel wie ein Schritt auf dem Rad
            pct / 8.0
        }
        _ => 0.5,
    };
    key + tempo
}

/// Neue Reihenfolge der kommenden Titel beider Playlists (Indizes in `lists[0]` bzw. `lists[1]`).
/// `start` = Titel auf Sendung (ohne: der erste gewählte Titel ist der beste Anfang), `first` = welche
/// Playlist als Nächste spielt.
pub fn plan(start: Option<Track>, first: usize, lists: [&[Track]; 2]) -> [Vec<usize>; 2] {
    let mut left: [Vec<usize>; 2] = [(0..lists[0].len()).collect(), (0..lists[1].len()).collect()];
    let mut out: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    let mut cur = start;
    let mut turn = first.min(1);
    while !left[0].is_empty() || !left[1].is_empty() {
        if left[turn].is_empty() {
            turn = 1 - turn;
        }
        let pick = match cur {
            // Ohne Ausgangstitel: der mit bekannter Tonart, der am besten zu den übrigen passt, sonst der erste
            None => left[turn].iter().position(|&i| lists[turn][i].key.is_some()).unwrap_or(0),
            Some(c) => {
                let mut best = (f32::MAX, 0);
                for (pos, &i) in left[turn].iter().enumerate() {
                    let k = cost(&c, &lists[turn][i]);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, bpm: f32) -> Track {
        Track { key: fw_library::parse_key(key), bpm: Some(bpm) }
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
        assert!(cost(&a, &Track { key: None, bpm: Some(124.0) }) > cost(&a, &t("2B", 124.0)));
    }

    #[test]
    fn alternates_between_the_playlists_and_chains_neighbours() {
        let a = [t("3A", 120.0), t("8A", 120.0), t("10A", 120.0)];
        let b = [t("10A", 120.0), t("9A", 120.0), t("4A", 120.0)];
        // Auf Sendung 8A aus Playlist A, als Nächstes spielt B
        let [oa, ob] = plan(Some(t("8A", 120.0)), 1, [&a, &b]);
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
        let a = [Track { key: None, bpm: Some(120.0) }, t("8A", 120.0)];
        let b = [t("9A", 120.0)];
        let [oa, ob] = plan(Some(t("8A", 120.0)), 0, [&a, &b]);
        assert_eq!(oa, [1, 0], "ohne Tonart ans Ende");
        assert_eq!(ob, [0]);
        let [oa, ob] = plan(None, 0, [&[], &[]]);
        assert!(oa.is_empty() && ob.is_empty());
    }
}
