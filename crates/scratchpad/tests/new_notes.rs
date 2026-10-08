//! New notes get their file once they have content, named after their title line, and file
//! names follow edited title lines when the user leaves the note (PLAN §9, §44; ADR 0061).

mod common;

use std::fs;

use common::{click, days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::notes::Selection;
use scratchpad::session::AUTOSAVE_DELAY;

fn open_title(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Option<String> {
    let notes = common::notes(root, cx);
    notes.read_with(cx, |notes, _| notes.open_title().map(str::to_owned))
}

fn editor_focused(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

#[gpui::test]
fn an_empty_folder_starts_with_a_new_note_ready_for_typing(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    let notes = common::notes(&root, cx);
    assert!(notes.read_with(cx, |notes, _| notes.has_draft()));
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn a_new_note_has_no_file_until_it_has_content(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas", days_ago(1, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-n");
    assert!(editor_focused(&root, cx));
    assert_eq!(editor_text(&root, cx), "");
    cx.simulate_input("  \n ");
    wait(AUTOSAVE_DELAY * 10, cx);
    cx.simulate_keystrokes("ctrl-s");
    click("note:Ideas", cx);

    assert_eq!(titles_on_disk(dir.path()), ["Ideas"]);
}

#[gpui::test]
fn a_new_note_gets_its_file_once_its_title_line_is_finished(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let dir =
        common::notes(&root, cx).read_with(cx, |notes, _| notes.store().unwrap().dir().to_owned());

    cx.simulate_input("Ideas for webhooks");
    wait(AUTOSAVE_DELAY * 10, cx);
    // Still typing the title: no file named after half a title.
    assert!(titles_on_disk(&dir).is_empty());
    assert_eq!(open_title(&root, cx).as_deref(), Some("Ideas for webhooks"));
    common::center_of("note:draft", cx);

    cx.simulate_keystrokes("enter");
    cx.simulate_input("retries");
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(titles_on_disk(&dir), ["Ideas for webhooks"]);
    assert_eq!(
        fs::read_to_string(dir.join("Ideas for webhooks.md")).unwrap(),
        "Ideas for webhooks\nretries"
    );
    let notes = common::notes(&root, cx);
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(dir.join("Ideas for webhooks.md"))
    );
    common::center_of("note:Ideas for webhooks", cx);
}

#[gpui::test]
fn leaving_a_new_note_gives_it_a_file_at_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas", days_ago(1, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-n");
    cx.simulate_input("# Short: one line");
    click("note:Ideas", cx);

    assert_eq!(titles_on_disk(dir.path()), ["Ideas", "Short one line"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Short one line.md")).unwrap(),
        "# Short: one line"
    );
    assert_eq!(editor_text(&root, cx), "Ideas");
}

#[gpui::test]
fn editing_the_title_line_renames_the_file_when_leaving_the_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas\nbody", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "Meeting", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("Big ");
    wait(AUTOSAVE_DELAY, cx);
    // Saved under the old name while the title is being typed; the list shows the new title.
    assert_eq!(titles_on_disk(dir.path()), ["Ideas", "Meeting"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Ideas.md")).unwrap(),
        "Big Ideas\nbody"
    );
    assert_eq!(open_title(&root, cx).as_deref(), Some("Big Ideas"));

    click("note:Meeting", cx);

    assert_eq!(titles_on_disk(dir.path()), ["Big Ideas", "Meeting"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Big Ideas.md")).unwrap(),
        "Big Ideas\nbody"
    );
    assert_eq!(open_title(&root, cx), None);
}

#[gpui::test]
fn a_title_typed_before_a_rename_is_saved_under_the_new_name(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas\nbody", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("Big ");
    cx.simulate_keystrokes("ctrl-s");
    // Typed after the save and rename were queued.
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(titles_on_disk(dir.path()), ["Big Ideas"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Big Ideas.md")).unwrap(),
        "Big Ideas\nbody!"
    );
    assert_eq!(editor_text(&root, cx), "Big Ideas\nbody!");
}

#[gpui::test]
fn editing_below_the_title_never_renames(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // A note whose name differs from its first line, e.g. created by another app.
    write_note(dir.path(), "Meeting", "# Agenda\n- budget", days_ago(0, 10));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("\n- hiring");
    cx.simulate_keystrokes("ctrl-s");

    assert_eq!(titles_on_disk(dir.path()), ["Meeting"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Meeting.md")).unwrap(),
        "# Agenda\n- budget\n- hiring"
    );
}

#[gpui::test]
fn renaming_in_the_sidebar_while_a_save_runs_leaves_no_old_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "Ideas\n", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let session = common::session(&root, cx);
    let notes = common::notes(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("and plans");

    // The save is on a background thread, not run yet, when the sidebar renames the note.
    session.update(cx, |session, cx| session.flush(cx));
    while !session.read_with(cx, |session, _| session.is_writing()) {
        assert!(cx.executor().tick());
    }
    notes
        .update(cx, |notes, cx| notes.rename(&ideas, "Plans", cx))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(titles_on_disk(dir.path()), ["Plans"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("Plans.md")).unwrap(),
        "Ideas\nand plans"
    );
}

#[gpui::test]
fn deleting_the_open_note_drops_its_unsaved_edits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "Ideas", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "Meeting", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let session = common::session(&root, cx);
    let notes = common::notes(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");
    session.update(cx, |session, cx| session.flush(cx));
    while !session.read_with(cx, |session, _| session.is_writing()) {
        assert!(cx.executor().tick());
    }

    notes
        .update(cx, |notes, cx| notes.delete(&ideas, cx))
        .unwrap();
    cx.run_until_parked();
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(titles_on_disk(dir.path()), ["Meeting"]);
    assert_eq!(editor_text(&root, cx), "Meeting");
    // Dropped as the delete asked, not offered for recovery.
    assert!(
        session
            .read_with(cx, |session, _| session.notices())
            .is_empty()
    );
}

#[gpui::test]
fn quitting_gives_a_new_note_its_file(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let dir =
        common::notes(&root, cx).read_with(cx, |notes, _| notes.store().unwrap().dir().to_owned());

    cx.simulate_input("Half a tit");
    cx.cx.update(|cx| cx.shutdown());
    cx.run_until_parked();

    assert_eq!(titles_on_disk(&dir), ["Half a tit"]);
}

#[gpui::test]
fn a_note_renamed_while_it_loads_still_opens(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas", days_ago(0, 10));
    let meeting = write_note(dir.path(), "Meeting", "Meeting\nagenda", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let session = common::session(&root, cx);
    let notes = common::notes(&root, cx);

    notes.update(cx, |notes, cx| notes.select(&meeting, cx));
    while !session.read_with(cx, |session, _| session.is_writing()) {
        assert!(cx.executor().tick());
    }
    notes
        .update(cx, |notes, cx| notes.rename(&meeting, "Plans", cx))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(editor_text(&root, cx), "Meeting\nagenda");
    let editor = common::editor(&root, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" and plans");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(
        fs::read_to_string(dir.path().join("Plans.md")).unwrap(),
        "Meeting\nagenda and plans"
    );
}
