//! The Notepad-style status bar under the note (ADR 0137).

mod common;

use std::fs;

use common::{click, days_ago, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::actions::ToggleStatusBar;
use scratchpad::status_bar::Status;
use scratchpad_core::Config;

fn status(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Status {
    root.read_with(cx, |root, cx| {
        root.editor_pane()
            .read(cx)
            .status_bar()
            .read(cx)
            .status()
            .clone()
    })
}

fn left(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> String {
    let status = status(root, cx);
    format!("{} | {}", status.position, status.characters)
}

#[gpui::test]
fn the_caret_position_counts_characters_not_bytes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // "e" + a combining acute accent is one grapheme of two characters; the emoji is one
    // character of four bytes.
    write_note(
        dir.path(),
        "Note",
        "Title\nw\u{f6}rld \u{1F600}e\u{301}x",
        days_ago(0, 10),
    );
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    assert_eq!(left(&root, cx), "Ln 1, Col 1 | 16 characters");

    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(left(&root, cx), "Ln 2, Col 11 | 16 characters");
    cx.simulate_keystrokes("left");
    assert_eq!(left(&root, cx), "Ln 2, Col 10 | 16 characters");
    // Over the accented e: one step for the caret, two characters.
    cx.simulate_keystrokes("left");
    assert_eq!(left(&root, cx), "Ln 2, Col 8 | 16 characters");
    cx.simulate_keystrokes("left");
    assert_eq!(left(&root, cx), "Ln 2, Col 7 | 16 characters");
    cx.simulate_keystrokes("home");
    assert_eq!(left(&root, cx), "Ln 2, Col 1 | 16 characters");
}

#[gpui::test]
fn typing_and_selecting_update_the_character_count(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Note", &"a".repeat(2_418), days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    assert_eq!(status(&root, cx).characters, "2,418 characters");

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("b\u{1F600}");
    cx.simulate_keystrokes("enter");
    assert_eq!(left(&root, cx), "Ln 2, Col 1 | 2,421 characters");

    cx.simulate_keystrokes("ctrl-shift-home");
    assert_eq!(left(&root, cx), "Ln 1, Col 1 | 2,421 of 2,421 characters");
    // The emoji and the line break.
    cx.simulate_keystrokes("ctrl-end shift-left shift-left");
    assert_eq!(left(&root, cx), "Ln 1, Col 2,420 | 2 of 2,421 characters");

    cx.simulate_keystrokes("backspace");
    assert_eq!(left(&root, cx), "Ln 1, Col 2,420 | 2,419 characters");
}

#[gpui::test]
fn line_endings_and_encoding_are_those_the_note_is_saved_with(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Windows", "one\r\ntwo\r\n", days_ago(0, 10));
    write_note(dir.path(), "Unix", "one\ntwo\n", days_ago(0, 9));
    let legacy = dir.path().join("Legacy.md");
    fs::write(&legacy, b"caf\xe9\n").unwrap();
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    let right = |cx: &mut VisualTestContext| {
        let status = status(&root, cx);
        (status.line_ending, status.encoding)
    };
    click("note:Windows", cx);
    assert_eq!(right(cx), ("Windows (CRLF)", "UTF-8"));
    // A line break counts once, as the user sees it.
    assert_eq!(left(&root, cx), "Ln 1, Col 1 | 8 characters");
    cx.simulate_keystrokes("enter ctrl-end");
    assert_eq!(left(&root, cx), "Ln 3, Col 1 | 8 characters");
    cx.simulate_keystrokes("left");
    assert_eq!(left(&root, cx), "Ln 2, Col 4 | 8 characters");
    click("note:Unix", cx);
    assert_eq!(right(cx), ("Unix (LF)", "UTF-8"));
    click("note:Legacy", cx);
    assert_eq!(right(cx), ("Unix (LF)", "Not UTF-8"));
    // Once the user chooses to edit it, it will be saved as UTF-8.
    click("choice:Edit Anyway", cx);
    assert_eq!(right(cx), ("Unix (LF)", "UTF-8"));
}

#[gpui::test]
fn the_status_bar_can_be_hidden_and_it_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (_root, cx) = common::open_with(storage.clone(), cx);
    let window_bottom =
        |cx: &mut VisualTestContext| cx.update(|window, _| window.viewport_size().height);
    let note_bottom = |cx: &mut VisualTestContext| cx.debug_bounds("note-area").unwrap().bottom();

    // Shown by default, under the note.
    let bar = cx.debug_bounds("status-bar").unwrap();
    assert_eq!(bar.bottom(), window_bottom(cx));
    assert_eq!(note_bottom(cx), bar.top());

    cx.dispatch_action(ToggleStatusBar);
    assert_eq!(note_bottom(cx), window_bottom(cx));
    common::close(cx);
    assert!(Config::load(&config_path).status_bar_hidden);

    let (_root, cx) = common::open_with(storage, cx);
    assert_eq!(note_bottom(cx), window_bottom(cx));
    cx.dispatch_action(ToggleStatusBar);
    assert_eq!(
        note_bottom(cx),
        cx.debug_bounds("status-bar").unwrap().top()
    );
}
