//! Coarse performance guards for the typing path (PLAN §55): each fails when its operation gets several
//! times slower than measured (docs/performance.md, ADR 0114). Thresholds sit 10x or more above the
//! criterion numbers so a loaded machine does not fail them; `cargo bench` gives the real numbers.
//!
//! Timings mean nothing in debug builds, so these run with `--release` only:
//! `cargo test --release -p scratchpad-editor -p scratchpad-core --test perf`.

use std::time::{Duration, Instant};

use scratchpad_editor::markdown::MarkdownState;
use scratchpad_editor::search::{CaseSensitivity, adjust_matches, find_all};
use scratchpad_editor::{ByteOffset, Editor, Selection};

const MB: usize = 1024 * 1024;

/// Markdown of about `size` bytes: headings, lists with emphasis and links, prose, code blocks.
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

/// The median time of `runs` calls of `f`.
fn median(runs: usize, mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[runs / 2]
}

fn assert_under(what: &str, time: Duration, limit: Duration) {
    // Shown with `--nocapture`, so the numbers are visible long before they reach the limit.
    eprintln!("{what}: {time:?} (limit {limit:?})");
    assert!(time < limit, "{what} took {time:?}, limit {limit:?}");
}

/// A parsed 10 MB note with the cursor at `fraction` of it, as the view holds it after scrolling there.
fn parsed_10mb_note(fraction: f64) -> (Editor, MarkdownState) {
    let mut editor = Editor::from_text(&markdown(10 * MB));
    let offset = ByteOffset((editor.buffer().len() as f64 * fraction) as usize);
    editor.move_to(offset, false);
    let mut markdown = MarkdownState::new(editor.buffer());
    markdown.decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end());
    (editor, markdown)
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn a_keystroke_in_a_10mb_note_and_restyling_its_line() {
    // Measured 10-50 µs per keystroke (ADR 0041).
    for fraction in [0.0, 0.5, 1.0] {
        let (mut editor, mut markdown) = parsed_10mb_note(fraction);
        let line = editor.buffer().line_of(editor.selection().head);
        let time = median(51, || {
            editor.insert_text("x");
            markdown.styled_lines(editor.buffer(), line..line + 1, Some(editor.selection()));
        });
        assert_under("a keystroke", time, Duration::from_millis(1));
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn styling_a_screen_of_lines() {
    // Measured ~36 µs for 50 lines, which the view does every frame after an edit.
    let (editor, mut markdown) = parsed_10mb_note(0.5);
    let first = editor.buffer().line_count() / 2;
    let selection = Selection::cursor(editor.buffer().line_start(first + 10));
    let time = median(51, || {
        markdown.styled_lines(editor.buffer(), first..first + 50, Some(selection));
    });
    assert_under("styling 50 lines", time, Duration::from_millis(1));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn opening_a_10mb_note() {
    // Measured ~21 ms to load the buffer plus ~27 ms to scan its Markdown regions.
    let text = markdown(10 * MB);
    let time = median(5, || {
        let editor = Editor::from_text(&text);
        MarkdownState::new(editor.buffer());
    });
    assert_under("opening 10 MB", time, Duration::from_millis(500));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn a_recovery_snapshot_and_the_keystroke_after_it() {
    // Measured ~7 µs; copying the text instead costs ~17 ms (ADR 0111).
    let mut editor = Editor::from_text(&markdown(10 * MB));
    let mut snapshots = Vec::new();
    let time = median(51, || {
        snapshots.push(editor.buffer().snapshot());
        editor.insert_text("x");
    });
    assert_under("a snapshot and a keystroke", time, Duration::from_millis(1));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn keeping_a_million_find_matches_in_step_with_a_keystroke() {
    // Measured 0.7-1.2 ms with the edit at the start of the note, where every match moves.
    let mut editor = Editor::from_text(&markdown(10 * MB));
    let mut matches = find_all(
        &editor.buffer().normalized_text(),
        "e",
        CaseSensitivity::Insensitive,
    );
    assert!(matches.len() > 500_000, "{} matches", matches.len());
    let version = editor.buffer().version();
    editor.insert_text("x");
    let change = *editor
        .buffer()
        .changes_since(version)
        .unwrap()
        .next()
        .unwrap();
    // An insertion at the very start touches no match, so every run moves all of them again.
    let time = median(11, || adjust_matches(&mut matches, &change));
    assert_under("moving the matches", time, Duration::from_millis(20));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing limits are for release builds")]
fn shift_tab_on_a_whole_long_list() {
    // Each item's parent used to be looked up through every line above it: 8 s for 4,000 items.
    let list: String = (0..5_000).map(|i| format!("  - item {i}\n")).collect();
    let time = median(5, || {
        let mut editor = Editor::from_text(&list);
        editor.set_selection(Selection::new(ByteOffset(0), editor.buffer().end()));
        let mut markdown = MarkdownState::new(editor.buffer());
        assert!(editor.outdent_list_items(&mut markdown));
    });
    assert_under("Shift+Tab on 5,000 items", time, Duration::from_millis(300));
}
