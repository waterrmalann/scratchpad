//! Coarse performance guard for note search (PLAN §55): fails when it gets several times slower than
//! measured (docs/performance.md, ADR 0114). Limits sit 10x or more above the criterion numbers so a loaded
//! machine does not fail them; `cargo bench -p scratchpad-core` gives the real numbers.
//!
//! Timings mean nothing in debug builds, so this runs with `--release` only:
//! `cargo test --release -p scratchpad-editor -p scratchpad-core --test perf`.

use std::time::{Duration, Instant};

use scratchpad_core::{NoteSearch, NoteStore};

/// 1,000 notes of about 3 KB of words, every 50th with the phrase searched for.
fn thousand_notes(store: &NoteStore) {
    let words = [
        "alpha", "harbour", "kettle", "lantern", "meadow", "quartz", "ribbon", "willow",
    ];
    for i in 0..1_000 {
        let note = store.create(Some(&format!("Note {i}"))).unwrap();
        let mut text = format!("# Note {i}\n\n");
        for w in 0..400 {
            text.push_str(words[(i * 7 + w * 3) % words.len()]);
            text.push(if w % 14 == 13 { '\n' } else { ' ' });
        }
        if i % 50 == 0 {
            text.push_str("\nthe webhook gateway needs benchmarking\n");
        }
        store.save(&note.path, &text).unwrap();
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let result = f();
    (result, start.elapsed())
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn searching_1000_notes() {
    let dir = tempfile::tempdir().unwrap();
    let store = NoteStore::open(dir.path()).unwrap();
    thousand_notes(&store);
    let notes = store.list().unwrap();
    let mut search = NoteSearch::new();

    // Measured ~134 ms: reads every file (from the OS cache).
    let (hits, cold) = timed(|| search.search(&store, &notes, "webhook gateway"));
    assert_eq!(hits.len(), 20);
    assert_under("the first search", cold, Duration::from_secs(2));

    // Measured 3-5 ms: each further keystroke of the query.
    let mut warm: Vec<Duration> = (0..11)
        .map(|_| timed(|| search.search(&store, &notes, "webhook gateway")).1)
        .collect();
    warm.sort();
    assert_under(
        "a warm search",
        warm[warm.len() / 2],
        Duration::from_millis(50),
    );
}

fn assert_under(what: &str, time: Duration, limit: Duration) {
    // Shown with `--nocapture`, so the numbers are visible long before they reach the limit.
    eprintln!("{what}: {time:?} (limit {limit:?})");
    assert!(time < limit, "{what} took {time:?}, limit {limit:?}");
}
