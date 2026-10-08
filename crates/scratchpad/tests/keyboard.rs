//! Driving the note list with the keyboard.

mod common;

use common::{EventLog, click, days_ago, titles_on_disk, write_note};
use gpui::{Focusable, TestAppContext, VisualTestContext};
use scratchpad::notes::NotesEvent;

#[gpui::test]
fn arrows_open_the_previous_and_next_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let first = write_note(dir.path(), "First", "", days_ago(0, 10));
    let second = write_note(dir.path(), "Second", "", days_ago(1, 10));
    let third = write_note(dir.path(), "Third", "", days_ago(9, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    click("note:First", cx);
    // Group headers are skipped; the ends of the list stop the selection.
    cx.simulate_keystrokes("down down down up");

    assert_eq!(
        events.take(),
        [
            NotesEvent::OpenNote(first),
            NotesEvent::OpenNote(second.clone()),
            NotesEvent::OpenNote(third),
            NotesEvent::OpenNote(second),
        ]
    );
}

#[gpui::test]
fn arrows_scroll_the_open_note_into_view(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..60 {
        write_note(
            dir.path(),
            &format!("Note {n:02}"),
            "",
            days_ago(0, 12) - std::time::Duration::from_secs(n),
        );
    }
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    // The list is virtualized: rows below the fold are not rendered...
    assert!(cx.debug_bounds("note:Note 40").is_none());
    click("note:Note 00", cx);
    for _ in 0..40 {
        cx.simulate_keystrokes("down");
    }

    // ...so this one was scrolled to.
    let row = common::center_of("note:Note 40", cx);
    let list_bottom = cx.debug_bounds("sidebar").unwrap().bottom();
    assert!(row.y < list_bottom);
}

#[gpui::test]
fn enter_moves_into_the_note_f2_renames_and_delete_deletes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "First", "", days_ago(0, 10));
    let second = write_note(dir.path(), "Second", "", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let editor = root.read_with(cx, |root, cx| root.editor_pane().focus_handle(cx));
    let editor_focused =
        |cx: &mut VisualTestContext| cx.update(|window, _| editor.is_focused(window));

    click("note:First", cx);
    assert!(!editor_focused(cx));
    cx.simulate_keystrokes("enter");
    assert!(editor_focused(cx));

    click("note:First", cx);
    cx.simulate_keystrokes("f2");
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    assert_eq!(titles_on_disk(dir.path()), ["Renamed", "Second"]);
    // Enter confirmed the rename; it did not also move into the note.
    assert!(!editor_focused(cx));

    let events = EventLog::new(&notes, cx);
    cx.simulate_keystrokes("delete");
    assert_eq!(titles_on_disk(dir.path()), ["Second"]);
    assert_eq!(titles_on_disk(&common::trash_dir(dir.path())), ["Renamed"]);
    assert_eq!(
        events.take(),
        [
            NotesEvent::Deleted(dir.path().join("Renamed.md")),
            NotesEvent::OpenNote(second),
        ]
    );
}
