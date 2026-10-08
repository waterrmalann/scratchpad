//! Notes changed or deleted by other programs while open (PLAN §30), and notes that are not
//! valid UTF-8 (PLAN §56). Filesystem events are passed to the session the way the watcher
//! does; the last test uses the real watcher.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{click, days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::session::{AUTOSAVE_DELAY, Notice, POLL_INTERVAL};
use scratchpad_core::NoteEvent;
use scratchpad_editor::Point;

fn changed_outside(path: &Path, text: &str, root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    fs::write(path, text).unwrap();
    report(NoteEvent::Changed(path.to_owned()), root, cx);
}

fn report(event: NoteEvent, root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let session = common::session(root, cx);
    session.update(cx, |session, cx| session.disk_events(vec![event], cx));
    cx.run_until_parked();
}

fn notices(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Vec<Notice> {
    let session = common::session(root, cx);
    session.read_with(cx, |session, _| session.notices())
}

fn cursor(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Point {
    let editor = common::editor(root, cx);
    editor.read_with(cx, |editor, _| {
        let engine = editor.editor();
        engine.buffer().offset_to_point(engine.selection().head)
    })
}

fn open_ideas(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    PathBuf,
    Entity<AppWindow>,
    &mut VisualTestContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "Ideas\nmine", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    (dir, ideas, root, cx)
}

#[gpui::test]
fn a_note_without_edits_reloads_and_keeps_the_cursor_line(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("down end");

    changed_outside(&ideas, "Ideas\nmine and theirs\nmore", &root, cx);

    assert_eq!(editor_text(&root, cx), "Ideas\nmine and theirs\nmore");
    assert_eq!(cursor(&root, cx), Point::new(1, 4));
    assert!(notices(&root, cx).is_empty());
}

#[gpui::test]
fn a_change_reported_while_the_note_loads_is_not_missed(cx: &mut TestAppContext) {
    let (dir, _ideas, root, cx) = open_ideas(cx);
    let meeting = write_note(dir.path(), "Meeting", "Meeting", days_ago(1, 10));
    let notes = common::notes(&root, cx);
    let session = common::session(&root, cx);

    // The note is read on a background thread (the only task left to run)...
    notes.update(cx, |notes, cx| notes.select(&meeting, cx));
    while !session.read_with(cx, |session, _| session.is_writing()) {
        assert!(cx.executor().tick());
    }
    assert!(cx.executor().tick());
    // ...then changed by another program, which is reported before the app has the text.
    fs::write(&meeting, "Meeting\nagenda").unwrap();
    session.update(cx, |session, cx| {
        session.disk_events(vec![NoteEvent::Changed(meeting.clone())], cx)
    });
    cx.run_until_parked();

    assert_eq!(editor_text(&root, cx), "Meeting\nagenda");
}

#[gpui::test]
fn our_own_saves_are_not_taken_for_outside_changes(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" typed");
    wait(AUTOSAVE_DELAY, cx);
    let before = cursor(&root, cx);

    // The watcher reports our save, even several times.
    report(NoteEvent::Changed(ideas.clone()), &root, cx);
    report(NoteEvent::Created(ideas), &root, cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(cursor(&root, cx), before);
    cx.simulate_input("!");
    assert_eq!(editor_text(&root, cx), "Ideas\nmine typed!");
}

// Saves serialize the text on the writer's thread (ADR 0111). What they write, and what the next
// save and disk check compare the file with, must be the file's own bytes, line endings included.
#[gpui::test]
fn saves_of_a_windows_note_keep_its_line_endings_and_are_not_taken_for_outside_changes(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "Ideas\r\nmine\r\n", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("typed");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(fs::read(&ideas).unwrap(), b"Ideas\r\nmine\r\ntyped");

    // Reported while there are unsaved edits, which a change by another program would put on hold.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("more");
    report(NoteEvent::Changed(ideas.clone()), &root, cx);
    wait(AUTOSAVE_DELAY, cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(fs::read(&ideas).unwrap(), b"Ideas\r\nmine\r\ntyped\r\nmore");
}

#[gpui::test]
fn an_outside_change_during_editing_asks_and_keeping_mine_overwrites(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");

    changed_outside(&ideas, "Ideas\ntheirs", &root, cx);
    assert_eq!(notices(&root, cx), [Notice::ChangedOnDisk]);
    // Neither version is lost while the question is open.
    cx.simulate_input(" more");
    wait(AUTOSAVE_DELAY * 10, cx);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
    assert_eq!(editor_text(&root, cx), "Ideas\nmine edited more");

    click("choice:Keep My Version", cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(
        fs::read_to_string(&ideas).unwrap(),
        "Ideas\nmine edited more"
    );
    // Their version is one undo away.
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(editor_text(&root, cx), "Ideas\ntheirs");
}

#[gpui::test]
fn an_outside_change_during_editing_can_be_loaded_instead(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    changed_outside(&ideas, "Ideas\ntheirs", &root, cx);

    click("choice:Load Their Version", cx);
    wait(AUTOSAVE_DELAY * 10, cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(editor_text(&root, cx), "Ideas\ntheirs");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
    let session = common::session(&root, cx);
    assert!(!session.read_with(cx, |session, _| session.is_dirty()));
    // Mine is one undo away.
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(editor_text(&root, cx), "Ideas\nmine edited");
}

#[gpui::test]
fn their_version_that_is_not_utf8_loads_read_only(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    fs::write(&ideas, b"Ideas\ncaf\xE9").unwrap();
    report(NoteEvent::Changed(ideas.clone()), &root, cx);

    click("choice:Load Their Version", cx);
    cx.simulate_input("!");
    cx.simulate_keystrokes("ctrl-s");

    assert_eq!(notices(&root, cx), [Notice::NotUtf8]);
    assert_eq!(editor_text(&root, cx), "Ideas\ncaf\u{FFFD}");
    assert_eq!(fs::read(&ideas).unwrap(), b"Ideas\ncaf\xE9");
}

#[gpui::test]
fn leaving_a_note_before_deciding_offers_my_version(cx: &mut TestAppContext) {
    let (dir, ideas, root, cx) = open_ideas(cx);
    write_note(dir.path(), "Other", "Other", days_ago(1, 10));
    let notes = common::notes(&root, cx);
    notes.update(cx, |notes, cx| notes.refresh(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    changed_outside(&ideas, "Ideas\ntheirs", &root, cx);

    click("note:Other", cx);

    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );
    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(editor_text(&root, cx), "Ideas\nmine edited");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nmine edited");
}

#[gpui::test]
fn a_rename_at_a_flush_is_not_taken_for_a_change_by_another_program(cx: &mut TestAppContext) {
    let (dir, ideas, root, cx) = open_ideas(cx);
    let session = common::session(&root, cx);
    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("Big ");

    // The watcher reports our save to the old name while the rename waits in the queue, then
    // the rename itself.
    session.update(cx, |session, cx| {
        session.flush(cx);
        session.disk_events(vec![NoteEvent::Changed(ideas.clone())], cx);
    });
    cx.run_until_parked();
    let renamed = dir.path().join("Big Ideas.md");
    report(NoteEvent::Removed(ideas), &root, cx);
    report(NoteEvent::Created(renamed.clone()), &root, cx);
    cx.simulate_input("!");
    wait(AUTOSAVE_DELAY, cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(titles_on_disk(dir.path()), ["Big Ideas"]);
    assert_eq!(fs::read_to_string(&renamed).unwrap(), "Big !Ideas\nmine");
}

#[gpui::test]
fn an_autosave_never_overwrites_a_change_the_watcher_has_not_reported_yet(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    fs::write(&ideas, "Ideas\ntheirs").unwrap();

    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
    assert_eq!(notices(&root, cx), [Notice::ChangedOnDisk]);
    click("choice:Keep My Version", cx);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nmine edited");
}

#[gpui::test]
fn edits_kept_from_a_changed_note_that_was_left_are_offered(cx: &mut TestAppContext) {
    let (dir, ideas, root, cx) = open_ideas(cx);
    write_note(dir.path(), "Other", "Other", days_ago(1, 10));
    let notes = common::notes(&root, cx);
    notes.update(cx, |notes, cx| notes.refresh(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    fs::write(&ideas, "Ideas\ntheirs").unwrap();

    click("note:Other", cx);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );

    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(editor_text(&root, cx), "Ideas\nmine edited");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nmine edited");
}

#[gpui::test]
fn a_save_waiting_in_the_queue_counts_as_unsaved(cx: &mut TestAppContext) {
    let (_dir, ideas, root, cx) = open_ideas(cx);
    let session = common::session(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" edited");
    fs::write(&ideas, "Ideas\ntheirs").unwrap();

    // The check of the disk is queued while our save has not run yet.
    session.update(cx, |session, cx| {
        session.disk_events(vec![NoteEvent::Changed(ideas.clone())], cx);
        session.flush(cx);
    });
    cx.run_until_parked();

    assert_eq!(notices(&root, cx), [Notice::ChangedOnDisk]);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
}

#[gpui::test]
fn a_note_deleted_outside_can_be_kept(cx: &mut TestAppContext) {
    let (dir, ideas, root, cx) = open_ideas(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" kept");
    fs::remove_file(&ideas).unwrap();
    report(NoteEvent::Removed(ideas.clone()), &root, cx);
    assert_eq!(notices(&root, cx), [Notice::DeletedOnDisk]);
    wait(AUTOSAVE_DELAY * 10, cx);
    assert!(titles_on_disk(dir.path()).is_empty());

    click("choice:Keep Note", cx);

    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nmine kept");
    common::center_of("note:Ideas", cx);
}

#[gpui::test]
fn a_note_deleted_outside_can_be_closed(cx: &mut TestAppContext) {
    let (dir, ideas, root, cx) = open_ideas(cx);
    write_note(dir.path(), "Other", "Other", days_ago(1, 10));
    fs::remove_file(&ideas).unwrap();
    report(NoteEvent::Removed(ideas.clone()), &root, cx);

    click("choice:Close Note", cx);
    wait(AUTOSAVE_DELAY * 10, cx);

    assert!(notices(&root, cx).is_empty());
    assert_eq!(titles_on_disk(dir.path()), ["Other"]);
    let notes = common::notes(&root, cx);
    assert!(notes.read_with(cx, |notes, _| notes.note(&ideas).is_none()));
}

#[gpui::test]
fn a_note_that_is_not_utf8_is_read_only_until_the_user_agrees(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Latin1.md");
    fs::write(&path, b"Menu\ncaf\xE9").unwrap();
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    assert_eq!(notices(&root, cx), [Notice::NotUtf8]);

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("!");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(editor_text(&root, cx), "Menu\ncaf\u{FFFD}");
    assert_eq!(fs::read(&path).unwrap(), b"Menu\ncaf\xE9");

    click("choice:Edit Anyway", cx);
    cx.simulate_input("!");
    cx.simulate_keystrokes("ctrl-s");
    assert!(notices(&root, cx).is_empty());
    assert_eq!(fs::read_to_string(&path).unwrap(), "Menu\ncaf\u{FFFD}!");
}

/// The one test with the real file watcher: it waits (in real time, bounded) for the OS to
/// report the change, since there is no way to know when that happens.
#[gpui::test]
fn the_real_watcher_reports_outside_changes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "Ideas", days_ago(0, 10));
    let data = tempfile::tempdir().unwrap();
    let storage = scratchpad::Storage {
        watch: true,
        ..common::storage(dir.path(), data.path())
    };
    let (root, cx) = common::open_with(storage, cx);
    // Let the watcher start.
    wait(POLL_INTERVAL, cx);

    fs::write(&ideas, "Ideas\nfrom another app").unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while editor_text(&root, cx) != "Ideas\nfrom another app" {
        assert!(Instant::now() < deadline, "the change was never reported");
        std::thread::sleep(Duration::from_millis(5));
        wait(POLL_INTERVAL, cx);
    }
}
