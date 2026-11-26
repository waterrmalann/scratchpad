//! Small things found rather than announced: Notepad's `.LOG` notes, the status bar's zoom extremes
//! and word count, and the shimmer when the last task is ticked.

mod common;

use common::{days_ago, editor_text, write_note};
use gpui::TestAppContext;

/// `2026-11-26 14:05`: a date, a space and a time.
fn is_stamp(line: &str) -> bool {
    let bytes = line.as_bytes();
    line.len() == 16
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b' ',
            13 => *b == b':',
            _ => b.is_ascii_digit(),
        })
}

#[gpui::test]
fn a_log_note_is_dated_when_writing_starts_at_its_end(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Journal", ".LOG\nfirst entry", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    // Opening it changes nothing; the cursor waits at the end.
    assert_eq!(editor_text(&root, cx), ".LOG\nfirst entry");

    cx.simulate_input("second");
    let text = editor_text(&root, cx);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text:?}");
    assert_eq!(&lines[..2], [".LOG", "first entry"]);
    assert!(is_stamp(lines[2]), "{text:?}");
    assert_eq!(lines[3], "second");

    // Once is enough: more typing adds no more stamps.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("more");
    assert_eq!(
        editor_text(&root, cx)
            .lines()
            .filter(|l| is_stamp(l))
            .count(),
        1
    );

    // The stamp is its own undo step, after the typing.
    cx.simulate_keystrokes("ctrl-z");
    while editor_text(&root, cx).lines().count() > 3 {
        cx.simulate_keystrokes("ctrl-z");
    }
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(editor_text(&root, cx), ".LOG\nfirst entry");
}

#[gpui::test]
fn a_log_is_not_dated_when_the_cursor_left_its_end(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(
        dir.path(),
        "Journal",
        ".LOG\nfirst entry\n",
        days_ago(0, 10),
    );
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    cx.simulate_keystrokes("ctrl-home down end");
    cx.simulate_input("!");
    assert_eq!(editor_text(&root, cx), ".LOG\nfirst entry!\n");
}

fn status(
    root: &gpui::Entity<scratchpad::AppWindow>,
    cx: &mut gpui::VisualTestContext,
) -> scratchpad::status_bar::Status {
    root.read_with(cx, |root, cx| {
        root.editor_pane()
            .read(cx)
            .status_bar()
            .read(cx)
            .status()
            .clone()
    })
}

#[gpui::test]
fn the_zoom_readout_has_an_ant_and_a_telescope_at_its_ends(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Note", "Title", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let editor = common::editor(&root, cx);
    for (percent, shown) in [
        (400, "400% 🔭"),
        (390, "390%"),
        (50, "50% 🐜"),
        (100, "100%"),
    ] {
        editor.update(cx, |editor, cx| editor.set_zoom_percent(percent, cx));
        cx.run_until_parked();
        assert_eq!(status(&root, cx).zoom, shown);
    }
}

#[gpui::test]
fn clicking_the_count_shows_words_then_reading_time(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let words = "word ".repeat(459);
    write_note(
        dir.path(),
        "Note",
        &format!("Title\n{words}"),
        days_ago(0, 10),
    );
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let characters = status(&root, cx).characters;
    assert!(characters.ends_with(" characters"), "{characters}");

    common::click("status-count", cx);
    assert_eq!(status(&root, cx).characters, "460 words");
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" more");
    assert_eq!(status(&root, cx).characters, "461 words");
    cx.simulate_keystrokes("shift-ctrl-left");
    assert_eq!(status(&root, cx).characters, "1 of 461 words");

    common::click("status-count", cx);
    assert_eq!(status(&root, cx).characters, "< 1 of 2 min read");
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(status(&root, cx).characters, "2 min read");

    common::click("status-count", cx);
    // Back to characters, five more of them.
    assert_eq!(status(&root, cx).characters, "2,306 characters");
    assert_eq!(characters, "2,301 characters");
}
