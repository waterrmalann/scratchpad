//! Search over 1,000 generated notes of about 3 KB each (PLAN §38).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use scratchpad_core::{NoteSearch, NoteStore};

const NOTES: usize = 1_000;
const WORDS_PER_NOTE: usize = 400;

/// Deterministic pseudo-random numbers; a benchmark corpus must not change between runs.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

fn vocabulary(rng: &mut Lcg) -> Vec<String> {
    (0..2_000)
        .map(|_| {
            let len = 3 + rng.next(7);
            (0..len)
                .map(|_| char::from(b'a' + rng.next(26) as u8))
                .collect()
        })
        .collect()
}

/// Notes shaped like real ones: a heading, paragraphs of words, an occasional rare phrase.
fn generate_notes(store: &NoteStore) {
    let mut rng = Lcg(42);
    let words = vocabulary(&mut rng);
    for i in 0..NOTES {
        let note = store
            .create(Some(&format!("Note {i} {}", words[i])))
            .unwrap();
        let mut text = format!("# Note {i} {}\n\n", words[i]);
        for w in 0..WORDS_PER_NOTE {
            text.push_str(&words[rng.next(words.len() as u64) as usize]);
            text.push(if w % 14 == 13 { '\n' } else { ' ' });
        }
        if i % 50 == 0 {
            text.push_str("\nthe webhook gateway needs benchmarking\n");
        }
        store.save(&note.path, &text).unwrap();
    }
}

fn bench_search(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let store = NoteStore::open(dir.path()).unwrap();
    generate_notes(&store);
    let notes = store.list().unwrap();
    assert_eq!(notes.len(), NOTES);

    let mut group = c.benchmark_group("search_1000_notes");

    // Typing another character: the contents are already cached.
    let mut warm = NoteSearch::new();
    warm.search(&store, &notes, "gateway");
    group.bench_function("warm_rare_phrase", |b| {
        b.iter(|| black_box(warm.search(&store, &notes, black_box("webhook gateway"))));
    });
    group.bench_function("warm_common_word", |b| {
        b.iter(|| black_box(warm.search(&store, &notes, black_box("a"))));
    });
    group.bench_function("warm_no_match", |b| {
        b.iter(|| black_box(warm.search(&store, &notes, black_box("zzzzqx"))));
    });

    // The first search after startup reads every file (from the OS file cache).
    group.bench_function("cold_first_search", |b| {
        b.iter(|| {
            let mut cold = NoteSearch::new();
            black_box(cold.search(&store, &notes, black_box("webhook gateway")))
        });
    });
    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
