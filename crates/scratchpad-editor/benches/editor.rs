//! Editing engine benchmarks (PLAN §38): opening, inserting into and deleting from large documents, and
//! searching. Inputs are generated, so results are reproducible without fixture files.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use scratchpad_editor::search::{CaseSensitivity, find_all};
use scratchpad_editor::{ByteOffset, Editor, Motion};

const KB: usize = 1024;
const MB: usize = 1024 * KB;

/// Markdown-like text of roughly `size` bytes with a mix of headings, list items, prose and some non-ASCII.
fn markdown(size: usize) -> String {
    let blocks = [
        "## Section heading\n\n",
        "- a list item with **bold** text and a [link](https://example.com)\n",
        "Plain prose that goes on for a while, much like a real note would, with café and 日本語 mixed in.\n",
        "\n",
        "```rust\nfn main() { println!(\"hello\"); }\n```\n",
    ];
    let mut text = String::with_capacity(size + 128);
    for block in blocks.iter().cycle() {
        if text.len() >= size {
            break;
        }
        text.push_str(block);
    }
    text
}

fn open(c: &mut Criterion) {
    let mut group = c.benchmark_group("open");
    for (name, size) in [
        ("1KB", KB),
        ("100KB", 100 * KB),
        ("1MB", MB),
        ("10MB", 10 * MB),
    ] {
        let text = markdown(size);
        group.bench_function(name, |b| b.iter(|| Editor::from_text(black_box(&text))));
    }
    let crlf = markdown(MB).replace('\n', "\r\n");
    group.bench_function("1MB_crlf", |b| {
        b.iter(|| Editor::from_text(black_box(&crlf)))
    });
    group.finish();
}

/// One keystroke at a given position of a 10 MB document, including history recording.
fn insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_10MB");
    let base = Editor::from_text(&markdown(10 * MB));
    for (name, fraction) in [("beginning", 0.0), ("middle", 0.5), ("end", 1.0)] {
        let mut editor = base.clone();
        let offset = ByteOffset((editor.buffer().len() as f64 * fraction) as usize);
        editor.move_to(offset, false);
        group.bench_function(name, |b| b.iter(|| editor.insert_text(black_box("x"))));
    }
    group.finish();
}

fn delete(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_10MB");
    let base = Editor::from_text(&markdown(10 * MB));
    let middle = base.buffer().len() / 2;
    for (name, len) in [("1KB", KB), ("1MB", MB)] {
        let range = ByteOffset(middle)..ByteOffset(middle + len);
        // Cloning a rope is O(1), so each iteration deletes from a fresh copy of the document.
        group.bench_function(name, |b| {
            b.iter_batched(
                || base.clone(),
                |mut editor| {
                    editor.replace_range(range.clone(), "");
                    editor
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

/// Cursor motions that walk a whole 1 MB line: vertical movement counts grapheme columns from the line start.
/// Flags are the worst case, as each one's extent depends on every regional indicator before it.
fn long_line_motion(c: &mut Criterion) {
    let mut group = c.benchmark_group("motion_1MB_line");
    for (name, unit) in [("ascii", "a"), ("flags", "🇩🇪")] {
        let line = unit.repeat(MB / unit.len());
        let mut base = Editor::from_text(&format!("{line}\n{line}"));
        base.move_cursor(Motion::LineEnd, false);
        group.bench_function(format!("down_{name}"), |b| {
            b.iter_batched(
                || base.clone(),
                |mut editor| editor.move_cursor(Motion::Down, false),
                BatchSize::SmallInput,
            )
        });
        group.bench_function(format!("word_left_{name}"), |b| {
            b.iter_batched(
                || base.clone(),
                |mut editor| editor.move_cursor(Motion::WordLeft, false),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn search(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_1MB");
    let editor = Editor::from_text(&markdown(MB));
    let text = editor.buffer().normalized_text();
    group.bench_function("rare_insensitive", |b| {
        b.iter(|| {
            find_all(
                black_box(&text),
                "NotInTheText",
                CaseSensitivity::Insensitive,
            )
        })
    });
    group.bench_function("common_insensitive", |b| {
        b.iter(|| find_all(black_box(&text), "Bold", CaseSensitivity::Insensitive))
    });
    group.bench_function("common_sensitive", |b| {
        b.iter(|| find_all(black_box(&text), "bold", CaseSensitivity::Sensitive))
    });
    group.bench_function("buffer_to_string", |b| {
        b.iter(|| black_box(editor.buffer()).normalized_text())
    });
    group.finish();
}

criterion_group!(benches, open, insert, delete, long_line_motion, search);
criterion_main!(benches);
