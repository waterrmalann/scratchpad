//! Renaming and deleting notes from the sidebar.

mod common;

use std::fs;

use common::{EventLog, click, days_ago, double_click, right_click, titles_on_disk, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::notes::{Notes, NotesEvent, Selection};
use scratchpad::toast;

fn selection(notes: &Entity<Notes>, cx: &mut VisualTestContext) -> Selection {
    notes.read_with(cx, |notes, _| notes.selection().clone())
}

fn toast_message(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| toast::current(cx).map(String::from))
}

#[gpui::test]
fn double_click_renames_the_file_in_place(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "webhooks", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    double_click("note:Ideas", cx);
    // The whole title is selected, so typing replaces it.
    cx.simulate_input("Webhook gateway");
    cx.simulate_keystrokes("enter");

    let renamed = dir.path().join("Webhook gateway.md");
    assert_eq!(titles_on_disk(dir.path()), ["Meeting", "Webhook gateway"]);
    assert_eq!(fs::read_to_string(&renamed).unwrap(), "webhooks");
    assert_eq!(
        events.take(),
        [
            NotesEvent::OpenNote(ideas.clone()),
            NotesEvent::Renamed {
                from: ideas,
                to: renamed.clone()
            },
        ]
    );
    assert_eq!(selection(&notes, cx), Selection::Note(renamed));
    common::center_of("note:Webhook gateway", cx);
}

#[gpui::test]
fn renaming_to_a_taken_name_adds_a_number(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    double_click("note:Ideas", cx);
    cx.simulate_input("meeting");
    cx.simulate_keystrokes("enter");

    assert_eq!(titles_on_disk(dir.path()), ["Meeting", "meeting 2"]);
    common::center_of("note:meeting 2", cx);
}

#[gpui::test]
fn escape_cancels_renaming_and_clicking_away_commits_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    double_click("note:Ideas", cx);
    cx.simulate_input("Discarded");
    cx.simulate_keystrokes("escape");
    assert_eq!(titles_on_disk(dir.path()), ["Ideas", "Meeting"]);

    double_click("note:Ideas", cx);
    cx.simulate_input("Kept");
    click("note:Meeting", cx);
    assert_eq!(titles_on_disk(dir.path()), ["Kept", "Meeting"]);
}

#[gpui::test]
fn context_menu_renames_a_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    right_click("note:Ideas", cx);
    click("menu:Rename", cx);
    cx.simulate_keystrokes("end");
    cx.simulate_input(" v2");
    cx.simulate_keystrokes("enter");

    assert_eq!(titles_on_disk(dir.path()), ["Ideas v2"]);
}

#[gpui::test]
fn deleting_the_open_note_moves_it_to_the_trash_and_opens_its_neighbour(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let first = write_note(dir.path(), "First", "", days_ago(0, 10));
    let middle = write_note(dir.path(), "Middle", "", days_ago(0, 9));
    let last = write_note(dir.path(), "Last", "", days_ago(0, 8));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    click("note:Middle", cx);
    let events = EventLog::new(&notes, cx);
    right_click("note:Middle", cx);
    click("menu:Delete", cx);

    assert_eq!(titles_on_disk(dir.path()), ["First", "Last"]);
    assert_eq!(titles_on_disk(&common::trash_dir(dir.path())), ["Middle"]);
    // The note below takes its place.
    assert_eq!(
        events.take(),
        [
            NotesEvent::Deleted(middle),
            NotesEvent::OpenNote(last.clone())
        ]
    );

    // At the end of the list, the note above takes its place.
    right_click("note:Last", cx);
    click("menu:Delete", cx);
    assert_eq!(
        events.take(),
        [
            NotesEvent::Deleted(last),
            NotesEvent::OpenNote(first.clone())
        ]
    );

    // Deleting a note that is not open leaves the open note alone.
    write_note(dir.path(), "Other", "", days_ago(0, 7));
    notes.update(cx, |notes, cx| notes.refresh(cx));
    cx.run_until_parked();
    right_click("note:Other", cx);
    click("menu:Delete", cx);
    assert_eq!(
        events.take(),
        [NotesEvent::Deleted(dir.path().join("Other.md"))]
    );
    assert_eq!(selection(&notes, cx), Selection::Note(first));
}

#[gpui::test]
fn a_listing_started_before_a_delete_does_not_bring_the_note_back(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let titles = |cx: &mut VisualTestContext| {
        notes.read_with(cx, |notes, _| {
            notes
                .notes()
                .iter()
                .map(|n| n.title.clone())
                .collect::<Vec<_>>()
        })
    };

    // E.g. the file watcher asks for a refresh; the folder is listed on a background thread
    // (the only runnable task, so one tick runs exactly that)...
    notes.update(cx, |notes, cx| notes.refresh(cx));
    assert!(cx.executor().tick());
    // ...and before the listing is applied, the user deletes a note.
    notes
        .update(cx, |notes, cx| notes.delete(&ideas, cx))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(titles(cx), ["Meeting"]);
}

#[gpui::test]
fn escape_closes_the_context_menu_without_acting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    right_click("note:Ideas", cx);
    cx.simulate_keystrokes("escape");
    // The menu is gone, so the click lands on the row underneath (which only opens the note).
    click("note:Ideas", cx);

    assert_eq!(titles_on_disk(dir.path()), ["Ideas"]);
    assert_eq!(
        events.take(),
        [NotesEvent::OpenNote(dir.path().join("Ideas.md"))]
    );
}

#[gpui::test]
fn failed_file_operations_show_a_toast_and_keep_the_app_running(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let ideas = write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);
    // Another program removes the file behind the app's back.
    fs::remove_file(&ideas).unwrap();

    double_click("note:Ideas", cx);
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");

    let message = toast_message(cx).expect("rename error shown");
    assert!(
        message.starts_with("Could not rename \"Ideas\". "),
        "{message}"
    );
    assert!(!message.contains("os error"), "{message}");
    #[cfg(windows)]
    assert_eq!(
        message,
        "Could not rename \"Ideas\". The system cannot find the file specified."
    );
    common::center_of("toast", cx);

    right_click("note:Ideas", cx);
    click("menu:Delete", cx);
    let message = toast_message(cx).expect("delete error shown");
    assert!(
        message.starts_with("Could not delete \"Ideas\". "),
        "{message}"
    );
}

#[gpui::test]
fn right_clicking_the_note_being_renamed_acts_on_its_new_name(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    double_click("note:Ideas", cx);
    cx.simulate_input("Plans");
    // Opening the menu takes focus from the title, which commits the rename.
    right_click("note:Ideas", cx);
    click("menu:Delete", cx);

    assert_eq!(toast_message(cx), None);
    assert_eq!(titles_on_disk(dir.path()), ["Meeting"]);
    assert_eq!(titles_on_disk(&common::trash_dir(dir.path())), ["Plans"]);
}
