//! Editing engine benchmarks (PLAN §38): opening, inserting into and deleting from large documents, searching,
//! and Markdown parsing and styling. Inputs are generated, so results are reproducible without fixture files.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use scratchpad_editor::markdown::MarkdownState;
use scratchpad_editor::search::{CaseSensitivity, find_all};
use scratchpad_editor::{ByteOffset, Editor, Motion, Selection};

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

/// What a save or crash recovery snapshot of a 10 MB document costs: serializing the text (what the writer does
/// off the UI thread, and the UI thread did before ADR 0111) versus taking a snapshot on the UI thread, which
/// makes the next keystroke copy the rope nodes it touches.
fn save(c: &mut Criterion) {
    let mut group = c.benchmark_group("save_10MB");
    let mut editor = Editor::from_text(&markdown(10 * MB));
    group.bench_function("to_text", |b| b.iter(|| black_box(&editor).to_text()));
    group.bench_function("snapshot", |b| {
        b.iter(|| black_box(editor.buffer()).snapshot())
    });
    group.bench_function("snapshot_and_keystroke", |b| {
        b.iter_with_large_drop(|| {
            let snapshot = editor.buffer().snapshot();
            editor.insert_text(black_box("x"));
            snapshot
        })
    });
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

/// Splitting a document into block regions (what opening costs) and parsing all of them (the worst case for
/// a renderer, which normally parses only visible regions).
fn markdown_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_parse");
    for (name, size) in [("100KB", 100 * KB), ("1MB", MB), ("10MB", 10 * MB)] {
        let editor = Editor::from_text(&markdown(size));
        let buffer = editor.buffer();
        group.bench_function(format!("scan_{name}"), |b| {
            b.iter(|| MarkdownState::new(black_box(buffer)))
        });
        group.bench_function(format!("full_{name}"), |b| {
            b.iter(|| {
                MarkdownState::new(black_box(buffer))
                    .decorations(buffer, ByteOffset(0)..buffer.end())
            })
        });
    }
    group.finish();
}

/// Keystrokes in a parsed 10 MB document, each followed by what the view does next: styling the edited line,
/// which syncs the Markdown state and reparses the edited region. Each iteration types a char and deletes it
/// again (two keystrokes), so the document does not grow during the measurement.
fn markdown_keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("markdown_two_keystrokes_10MB");
    let base = Editor::from_text(&markdown(10 * MB));
    for (name, fraction) in [("beginning", 0.0), ("middle", 0.5), ("end", 1.0)] {
        let mut editor = base.clone();
        let offset = ByteOffset((editor.buffer().len() as f64 * fraction) as usize);
        editor.move_to(offset, false);
        let mut markdown = MarkdownState::new(editor.buffer());
        markdown.decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end());
        let line = editor.buffer().line_of(offset);
        group.bench_function(name, |b| {
            b.iter(|| {
                editor.insert_text(black_box("x"));
                markdown.styled_lines(editor.buffer(), line..line + 1, Some(editor.selection()));
                editor.backspace();
                markdown.styled_lines(editor.buffer(), line..line + 1, Some(editor.selection()))
            })
        });
    }
    group.finish();
}

/// Styling a screenful of lines in the middle of a parsed 1 MB document, as the view does every frame.
fn markdown_styled_screen(c: &mut Criterion) {
    let editor = Editor::from_text(&markdown(MB));
    let buffer = editor.buffer();
    let mut markdown = MarkdownState::new(buffer);
    markdown.decorations(buffer, ByteOffset(0)..buffer.end());
    let first = buffer.line_count() / 2;
    let selection = Selection::cursor(buffer.line_start(first + 10));
    c.bench_function("markdown_styled_lines_50", |b| {
        b.iter(|| markdown.styled_lines(buffer, black_box(first..first + 50), Some(selection)))
    });
}

/// Styling one 96 KB line of 16k bold words: the cost must grow linearly with the spans on a line.
fn markdown_styled_long_line(c: &mut Criterion) {
    let editor = Editor::from_text(&"**a** ".repeat(16_000));
    let buffer = editor.buffer();
    let mut markdown = MarkdownState::new(buffer);
    c.bench_function("markdown_styled_long_line", |b| {
        b.iter(|| {
            markdown.styled_lines(
                buffer,
                black_box(0..1),
                Some(Selection::cursor(ByteOffset(0))),
            )
        })
    });
}

criterion_group!(
    benches,
    open,
    insert,
    delete,
    save,
    long_line_motion,
    search,
    markdown_parse,
    markdown_keystroke,
    markdown_styled_screen,
    markdown_styled_long_line
);
criterion_main!(benches);
