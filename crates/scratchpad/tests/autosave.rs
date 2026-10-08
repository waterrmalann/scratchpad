//! Opening notes and saving them: autosave after a pause, and immediately when the user leaves
//! the note, presses Ctrl+S, switches windows, closes the window or quits (PLAN §7, §35).

mod common;

use std::fs;
use std::time::Duration;

use common::{click, days_ago, editor_text, wait, write_note};
use gpui::{Entity, EntityInputHandler, Focusable, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::session::{AUTOSAVE_DELAY, MAX_AUTOSAVE_DELAY};
use scratchpad::toast;

const MS: Duration = Duration::from_millis(1);

fn focus_editor(root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
}

/// Opens the app on a folder with `Ideas` (newest, so it opens at startup) and `Meeting`, and
/// puts the caret at the end of `Ideas`.
fn open_two_notes(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, Entity<AppWindow>, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas\nfirst", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "Meeting\nagenda", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    focus_editor(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    (dir, root, cx)
}

fn read(dir: &tempfile::TempDir, title: &str) -> String {
    fs::read_to_string(dir.path().join(format!("{title}.md"))).unwrap()
}

#[gpui::test]
fn the_newest_note_opens_at_startup(cx: &mut TestAppContext) {
    let (_dir, root, cx) = open_two_notes(cx);
    assert_eq!(editor_text(&root, cx), "Ideas\nfirst");
}

#[gpui::test]
fn typing_saves_once_the_typing_pauses(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);

    cx.simulate_input(" draft");
    wait(AUTOSAVE_DELAY / 2, cx);
    cx.simulate_input(" two");
    // Each keystroke restarts the delay.
    wait(AUTOSAVE_DELAY - MS, cx);
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst");

    wait(MS, cx);
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst draft two");
}

#[gpui::test]
fn typing_without_pauses_is_still_saved_every_few_seconds(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);
    let keystroke_interval = AUTOSAVE_DELAY / 2;

    let mut typed = 0;
    while keystroke_interval * typed < MAX_AUTOSAVE_DELAY - keystroke_interval {
        cx.simulate_input("x");
        wait(keystroke_interval, cx);
        typed += 1;
    }
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst");
    cx.simulate_input("x");
    wait(keystroke_interval, cx);

    let saved = read(&dir, "Ideas");
    assert!(saved.starts_with("Ideas\nfirstxxx"), "{saved}");
}

#[gpui::test]
fn switching_notes_saves_the_one_left_at_once(cx: &mut TestAppContext) {
    let (dir, root, cx) = open_two_notes(cx);

    cx.simulate_input(" edited");
    click("note:Meeting", cx);

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst edited");
    assert_eq!(editor_text(&root, cx), "Meeting\nagenda");
    click("note:Ideas", cx);
    assert_eq!(editor_text(&root, cx), "Ideas\nfirst edited");
}

#[gpui::test]
fn ctrl_s_saves_at_once(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);

    cx.simulate_input(" saved");
    cx.simulate_keystrokes("ctrl-s");

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst saved");
}

#[gpui::test]
fn switching_to_another_window_saves_at_once(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);

    cx.simulate_input(" away");
    cx.deactivate_window();

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst away");
}

#[gpui::test]
fn closing_the_window_saves_before_it_goes(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);

    cx.simulate_input(" closing");
    assert!(cx.simulate_close());

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst closing");
}

#[gpui::test]
fn ctrl_w_saves_and_closes(cx: &mut TestAppContext) {
    let (dir, _root, cx) = open_two_notes(cx);

    cx.simulate_input(" ctrl-w");
    cx.simulate_keystrokes("ctrl-w");

    assert!(cx.windows().is_empty());
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst ctrl-w");
}

#[gpui::test]
fn quitting_saves_and_no_older_save_lands_afterwards(cx: &mut TestAppContext) {
    let (dir, root, cx) = open_two_notes(cx);
    let session = common::session(&root, cx);
    let editor = common::editor(&root, cx);
    cx.simulate_input(" quit");
    // A save is handed to a background thread but has not run yet...
    session.update(cx, |session, cx| session.flush(cx));
    while !session.read_with(cx, |session, _| session.is_writing()) {
        assert!(cx.executor().tick());
    }
    // ...more is typed, and the app quits.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, " later", window, cx)
    });
    cx.cx.update(|cx| cx.shutdown());
    cx.run_until_parked();

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst quit later");
}

#[gpui::test]
fn a_failed_save_keeps_the_text_and_succeeds_later(cx: &mut TestAppContext) {
    let (dir, root, cx) = open_two_notes(cx);
    let ideas = dir.path().join("Ideas.md");
    set_read_only(&ideas, true);

    cx.simulate_input(" kept");
    wait(AUTOSAVE_DELAY, cx);

    let message = cx.update(|_, cx| toast::current(cx)).expect("error shown");
    assert!(
        message.starts_with("Could not save \"Ideas\". "),
        "{message}"
    );
    let session = common::session(&root, cx);
    assert!(session.read_with(cx, |session, _| session.is_dirty()));
    assert_eq!(editor_text(&root, cx), "Ideas\nfirst kept");
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst");

    // Fixed outside the app (e.g. the file is unlocked); the next save goes through.
    set_read_only(&ideas, false);
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst kept");
    assert!(!session.read_with(cx, |session, _| session.is_dirty()));
}

#[gpui::test]
fn a_note_that_cannot_be_read_shows_an_error_and_the_app_goes_on(cx: &mut TestAppContext) {
    let (dir, root, cx) = open_two_notes(cx);
    fs::remove_file(dir.path().join("Meeting.md")).unwrap();

    click("note:Meeting", cx);

    let message = cx.update(|_, cx| toast::current(cx)).expect("error shown");
    assert!(
        message.starts_with("Could not read \"Meeting\". "),
        "{message}"
    );
    assert_eq!(editor_text(&root, cx), "");
    click("note:Ideas", cx);
    assert_eq!(editor_text(&root, cx), "Ideas\nfirst");
}

fn set_read_only(path: &std::path::Path, read_only: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).unwrap();
}

#[gpui::test]
fn switching_quickly_saves_the_note_left_and_opens_only_the_last(cx: &mut TestAppContext) {
    let (dir, root, cx) = open_two_notes(cx);
    let third = write_note(dir.path(), "Third", "Third\n", days_ago(0, 8));
    let notes = common::notes(&root, cx);
    notes.update(cx, |notes, cx| notes.refresh(cx));
    cx.run_until_parked();
    cx.simulate_input(" edited");

    notes.update(cx, |notes, cx| {
        notes.select(&dir.path().join("Meeting.md"), cx);
        notes.select(&third, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("typed");
    cx.simulate_keystrokes("ctrl-s");

    assert_eq!(read(&dir, "Ideas"), "Ideas\nfirst edited");
    assert_eq!(read(&dir, "Meeting"), "Meeting\nagenda");
    assert_eq!(read(&dir, "Third"), "Third\ntyped");
    assert_eq!(editor_text(&root, cx), "Third\ntyped");
}
