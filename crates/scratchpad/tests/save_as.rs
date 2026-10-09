//! File > Save As (Ctrl+Shift+S): the text goes to a file the user picks, which is edited from
//! then on; the note or file left behind keeps what it held (ADR 0146).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::file_dialogs::PickFileForTests;
use scratchpad::notes::Selection;
use scratchpad::session::{AUTOSAVE_DELAY, Notice};
use scratchpad::toast;
use scratchpad_core::{NoteEvent, RecoveryStore};
use tempfile::TempDir;

struct Folders {
    notes: TempDir,
    elsewhere: TempDir,
    data: TempDir,
}

impl Folders {
    fn new() -> Self {
        Self {
            notes: tempfile::tempdir().unwrap(),
            elsewhere: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
        }
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
        common::open_with(common::storage(self.notes.path(), self.data.path()), cx)
    }

    fn snapshots(&self) -> usize {
        RecoveryStore::new(self.data.path().join("recovery"))
            .list()
            .len()
    }
}

/// Ctrl+Shift+S, answering the dialog with `pick`, which gets the folder it starts in.
fn save_as(pick: impl FnOnce(&Path) -> Option<PathBuf>, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("ctrl-shift-s");
    // The window loses activation to the dialog.
    cx.deactivate_window();
    cx.simulate_new_path_selection(pick);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
}

fn selection(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Selection {
    let notes = common::notes(root, cx);
    notes.read_with(cx, |notes, _| notes.selection().clone())
}

fn listed(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    let notes = common::notes(root, cx);
    notes.read_with(cx, |notes, _| {
        notes.notes().iter().map(|note| note.path.clone()).collect()
    })
}

fn is_markdown(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    common::editor(root, cx).read_with(cx, |editor, _| editor.is_markdown())
}

#[gpui::test]
fn a_note_saved_as_a_text_file_elsewhere_is_edited_there_and_stays_as_it_was(
    cx: &mut TestAppContext,
) {
    let folders = Folders::new();
    let ideas = write_note(
        folders.notes.path(),
        "Ideas",
        "Ideas\nbody",
        days_ago(0, 10),
    );
    let copy = folders.elsewhere.path().join("copy.txt");
    let (root, cx) = folders.open(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");

    let notes_dir = folders.notes.path().to_owned();
    save_as(
        |dir| {
            assert_eq!(dir, notes_dir, "a note's dialog starts in the notes folder");
            Some(copy.clone())
        },
        cx,
    );

    assert_eq!(fs::read_to_string(&copy).unwrap(), "Ideas\nbody unsaved");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nbody unsaved");
    assert_eq!(selection(&root, cx), Selection::File(copy.clone()));
    assert!(!is_markdown(&root, cx));
    assert_eq!(listed(&root, cx), [ideas.as_path()]);

    cx.simulate_input(" more");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(
        fs::read_to_string(&copy).unwrap(),
        "Ideas\nbody unsaved more"
    );
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\nbody unsaved");
    assert_eq!(editor_text(&root, cx), "Ideas\nbody unsaved more");
}

#[gpui::test]
fn a_new_note_saved_into_the_notes_folder_gets_only_the_chosen_file(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (root, cx) = folders.open(cx);
    // A finished title line, which would give a new note its file at the next autosave.
    cx.simulate_input("Shopping\nmilk");

    let list = folders.notes.path().join("Groceries.md");
    save_as(|_| Some(list.clone()), cx);
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(titles_on_disk(folders.notes.path()), ["Groceries"]);
    assert_eq!(fs::read_to_string(&list).unwrap(), "Shopping\nmilk");
    assert_eq!(selection(&root, cx), Selection::Note(list.clone()));
    assert_eq!(listed(&root, cx), [list.as_path()]);
    assert!(is_markdown(&root, cx));
    assert_eq!(folders.snapshots(), 0, "nothing left to recover");

    // It is a note like any other from now on: editing its title renames it.
    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("Weekly ");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(titles_on_disk(folders.notes.path()), ["Weekly Shopping"]);
}

#[gpui::test]
fn cancelling_save_as_leaves_a_new_note_to_be_saved_as_usual(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (root, cx) = folders.open(cx);
    cx.simulate_input("Shopping\nmilk");

    save_as(|_| None, cx);
    assert!(titles_on_disk(folders.notes.path()).is_empty());
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(titles_on_disk(folders.notes.path()), ["Shopping"]);
    let shopping = folders.notes.path().join("Shopping.md");
    assert_eq!(selection(&root, cx), Selection::Note(shopping));
}

#[gpui::test]
fn a_file_is_saved_as_from_its_own_folder_under_its_own_name(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.elsewhere.path().join("log.txt");
    fs::write(&log, "log").unwrap();
    let (root, cx) = folders.open(cx);
    cx.update(|_, cx| cx.set_global(PickFileForTests(Some(log.clone()))));
    cx.simulate_keystrokes("ctrl-o");
    cx.run_until_parked();

    let elsewhere = folders.elsewhere.path().to_owned();
    let copy = folders.elsewhere.path().join("log.md");
    save_as(
        |dir| {
            assert_eq!(dir, elsewhere);
            Some(copy.clone())
        },
        cx,
    );

    assert_eq!(selection(&root, cx), Selection::File(copy.clone()));
    assert!(is_markdown(&root, cx), "named like Markdown now");
    assert_eq!(fs::read_to_string(&copy).unwrap(), "log");
    assert_eq!(fs::read_to_string(&log).unwrap(), "log");
}

#[gpui::test]
fn a_file_that_is_not_utf8_is_saved_as_only_after_edit_anyway(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let legacy = folders.elsewhere.path().join("legacy.txt");
    fs::write(&legacy, b"Caf\xE9").unwrap();
    let copy = folders.elsewhere.path().join("copy.txt");
    let (root, cx) = folders.open(cx);
    cx.update(|_, cx| cx.set_global(PickFileForTests(Some(legacy.clone()))));
    cx.simulate_keystrokes("ctrl-o");
    cx.run_until_parked();

    // The copy would hold replacement characters, which the notice asks about first.
    cx.simulate_keystrokes("ctrl-shift-s");
    cx.run_until_parked();
    assert!(!cx.did_prompt_for_new_path(), "no dialog");
    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.contains("Edit Anyway")),
        "{message:?}"
    );

    common::click("choice:Edit Anyway", cx);
    save_as(|_| Some(copy.clone()), cx);
    assert_eq!(fs::read_to_string(&copy).unwrap(), "Caf\u{FFFD}");
    assert_eq!(selection(&root, cx), Selection::File(copy));
    assert_eq!(fs::read(&legacy).unwrap(), b"Caf\xE9");
}

#[gpui::test]
fn saving_as_the_open_note_itself_just_saves_it(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(
        folders.notes.path(),
        "Ideas",
        "Ideas\nbody",
        days_ago(0, 10),
    );
    let (root, cx) = folders.open(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" typed");
    wait(AUTOSAVE_DELAY, cx);
    // Another program changes it while there are unsaved edits.
    cx.simulate_input(" mine");
    fs::write(&ideas, "Ideas\ntheirs").unwrap();
    let session = common::session(&root, cx);
    session.update(cx, |session, cx| {
        session.disk_events(vec![NoteEvent::Changed(ideas.clone())], cx)
    });
    cx.run_until_parked();

    // As the dialog may spell it. Saving it still waits for the user's decision.
    let respelled = folders.notes.path().join("IDEAS.md");
    save_as(|_| Some(respelled.clone()), cx);

    assert_eq!(selection(&root, cx), Selection::Note(ideas.clone()));
    assert_eq!(titles_on_disk(folders.notes.path()), ["Ideas"]);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas\ntheirs");
    let notices = session.read_with(cx, |session, _| session.notices());
    assert_eq!(notices, [Notice::ChangedOnDisk]);
    common::click("choice:Keep My Version", cx);
    assert_eq!(
        fs::read_to_string(&ideas).unwrap(),
        "Ideas\nbody typed mine"
    );
}

#[gpui::test]
fn a_file_saved_as_over_a_note_replaces_it_and_opens_as_that_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let old = write_note(folders.notes.path(), "Old", "Old", days_ago(1, 10));
    let log = folders.elsewhere.path().join("log.txt");
    fs::write(&log, "Log").unwrap();
    let (root, cx) = folders.open(cx);
    cx.update(|_, cx| cx.set_global(PickFileForTests(Some(log.clone()))));
    cx.simulate_keystrokes("ctrl-o");
    cx.run_until_parked();

    // The dialog asked before replacing it.
    save_as(|_| Some(old.clone()), cx);

    assert_eq!(fs::read_to_string(&old).unwrap(), "Log");
    assert_eq!(fs::read_to_string(&log).unwrap(), "Log");
    assert_eq!(selection(&root, cx), Selection::Note(old.clone()));
    assert_eq!(listed(&root, cx), [old.as_path()]);
    assert!(is_markdown(&root, cx));
    // A note now: its title renames it.
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" book");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(titles_on_disk(folders.notes.path()), ["Log book"]);
    assert_eq!(fs::read_to_string(&log).unwrap(), "Log");
}

#[gpui::test]
fn a_failed_save_as_keeps_editing_the_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(folders.notes.path(), "Ideas", "Ideas", days_ago(0, 10));
    let (root, cx) = folders.open(cx);

    let missing = folders.elsewhere.path().join("gone").join("copy.md");
    save_as(|_| Some(missing.clone()), cx);

    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Could not save \"copy\"")),
        "{message:?}"
    );
    assert_eq!(selection(&root, cx), Selection::Note(ideas.clone()));
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" still mine");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas still mine");
}
