//! Unsaved text survives a crash in recovery snapshots and is offered on the next start
//! (PLAN §40, ADR 0021). A crash is simulated by dropping the window without the flush that
//! closing it normally does, then opening the app again on the same folders.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{click, days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::session::{AUTOSAVE_DELAY, Notice, SNAPSHOT_INTERVAL};
use scratchpad_core::RecoveryStore;

struct Folders {
    notes: tempfile::TempDir,
    data: tempfile::TempDir,
}

impl Folders {
    fn new() -> Self {
        Self {
            notes: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
        }
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
        common::open_with(common::storage(self.notes.path(), self.data.path()), cx)
    }

    fn snapshots(&self) -> Vec<(PathBuf, String)> {
        let recovery = RecoveryStore::new(self.data.path().join("recovery"));
        recovery
            .list()
            .into_iter()
            .map(|snapshot| (snapshot.note_path, snapshot.text))
            .collect()
    }
}

/// Ends the app the way a crash would: no flush, no tasks run afterwards.
fn crash(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

fn notices(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Vec<Notice> {
    let session = common::session(root, cx);
    session.read_with(cx, |session, _| session.notices())
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(read_only);
    fs::set_permissions(path, permissions).unwrap();
}

#[gpui::test]
fn a_new_note_is_snapshotted_until_it_gets_its_file(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (_root, cx) = folders.open(cx);

    cx.simulate_input("Half a title");
    wait(AUTOSAVE_DELAY, cx);
    let snapshots = folders.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].1, "Half a title");

    cx.simulate_input(" done\nbody");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(titles_on_disk(folders.notes.path()), ["Half a title done"]);
    assert!(folders.snapshots().is_empty());
}

#[gpui::test]
fn a_new_note_lost_in_a_crash_is_restored_as_a_new_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (_root, cx) = folders.open(cx);
    cx.simulate_input("Lost idea");
    wait(AUTOSAVE_DELAY, cx);
    crash(cx);

    let (root, cx) = folders.open(cx);
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Lost idea".into(),
            new_note: true
        }]
    );
    click("choice:Restore", cx);
    assert_eq!(editor_text(&root, cx), "Lost idea");
    cx.simulate_keystrokes("ctrl-s");

    assert!(notices(&root, cx).is_empty());
    assert_eq!(titles_on_disk(folders.notes.path()), ["Lost idea"]);
    assert!(folders.snapshots().is_empty());
}

#[gpui::test]
fn edits_that_could_not_be_saved_are_restored_after_a_crash(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(folders.notes.path(), "Ideas", "Ideas", days_ago(0, 10));
    write_note(folders.notes.path(), "Other", "Other", days_ago(0, 9));
    let (_root, cx) = folders.open(cx);
    set_read_only(&ideas, true);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(
        folders.snapshots(),
        [(ideas.clone(), "Ideas unsaved".into())]
    );
    crash(cx);
    set_read_only(&ideas, false);
    // The app reopens on another note, so restoring has to open this one.
    fs::write(
        folders.data.path().join("config.json"),
        format!(
            r#"{{"last_opened_note": {:?}}}"#,
            folders.notes.path().join("Other.md")
        ),
    )
    .unwrap();

    let (root, cx) = folders.open(cx);
    assert_eq!(editor_text(&root, cx), "Other");
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );
    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(editor_text(&root, cx), "Ideas unsaved");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas unsaved");
    assert!(folders.snapshots().is_empty());
    // The version it replaced is one undo away.
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(editor_text(&root, cx), "Ideas");
}

#[gpui::test]
fn discarded_recovered_text_is_not_offered_again(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (_root, cx) = folders.open(cx);
    cx.simulate_input("Not wanted");
    wait(AUTOSAVE_DELAY, cx);
    crash(cx);

    let (root, cx) = folders.open(cx);
    click("choice:Discard", cx);
    assert!(notices(&root, cx).is_empty());
    assert!(folders.snapshots().is_empty());
    assert!(titles_on_disk(folders.notes.path()).is_empty());
    crash(cx);

    let (root, cx) = folders.open(cx);
    assert!(notices(&root, cx).is_empty());
}

#[gpui::test]
fn snapshots_of_text_that_was_saved_after_all_are_not_offered(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(folders.notes.path(), "Ideas", "Ideas v2", days_ago(0, 10));
    RecoveryStore::new(folders.data.path().join("recovery"))
        .write(&ideas, "Ideas v2")
        .unwrap();

    let (root, cx) = folders.open(cx);

    assert!(notices(&root, cx).is_empty());
    assert!(folders.snapshots().is_empty());
}

#[gpui::test]
fn typing_without_pauses_is_kept_in_a_snapshot_until_it_is_saved(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(folders.notes.path(), "Ideas", "Ideas", days_ago(0, 10));
    let (_root, cx) = folders.open(cx);
    cx.simulate_keystrokes("ctrl-end");

    // Too fast for the autosave delay, for longer than the snapshot interval.
    let keystroke_interval = AUTOSAVE_DELAY / 2;
    let mut typed_for = Duration::ZERO;
    while typed_for <= SNAPSHOT_INTERVAL {
        cx.simulate_input("x");
        wait(keystroke_interval, cx);
        typed_for += keystroke_interval;
    }
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas");
    let snapshots = folders.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert!(snapshots[0].1.starts_with("Ideasxxx"), "{snapshots:?}");

    // A pause saves the text, and the snapshot goes.
    wait(AUTOSAVE_DELAY, cx);
    assert!(fs::read_to_string(&ideas).unwrap().starts_with("Ideasxxx"));
    assert!(folders.snapshots().is_empty());
}

#[gpui::test]
fn text_that_could_not_be_saved_to_a_note_left_is_offered(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(
        folders.notes.path(),
        "Ideas",
        "Ideas
body",
        days_ago(0, 10),
    );
    write_note(folders.notes.path(), "Other", "Other", days_ago(0, 9));
    let (root, cx) = folders.open(cx);
    set_read_only(&ideas, true);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");

    click("note:Other", cx);

    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );
    set_read_only(&ideas, false);
    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(
        fs::read_to_string(&ideas).unwrap(),
        "Ideas
body unsaved"
    );
}

#[gpui::test]
fn quitting_after_a_failed_save_offers_the_text_on_the_next_start(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(
        folders.notes.path(),
        "Ideas",
        "Ideas
body",
        days_ago(0, 10),
    );
    let (_root, cx) = folders.open(cx);
    set_read_only(&ideas, true);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");

    cx.cx.update(|cx| cx.shutdown());
    cx.run_until_parked();
    set_read_only(&ideas, false);

    let (root, cx) = folders.open(cx);
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );
    click("choice:Restore", cx);
    assert_eq!(
        editor_text(&root, cx),
        "Ideas
body unsaved"
    );
}

#[gpui::test]
fn discarding_older_recovered_text_keeps_the_open_notes_snapshot(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let ideas = write_note(
        folders.notes.path(),
        "Ideas",
        "Ideas
body",
        days_ago(0, 10),
    );
    RecoveryStore::new(folders.data.path().join("recovery"))
        .write(
            &ideas,
            "Ideas
older",
        )
        .unwrap();
    set_read_only(&ideas, true);
    let (_root, cx) = folders.open(cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" newer");
    wait(AUTOSAVE_DELAY, cx);

    click("choice:Discard", cx);
    wait(AUTOSAVE_DELAY, cx);
    crash(cx);
    set_read_only(&ideas, false);

    assert_eq!(
        folders.snapshots(),
        [(
            ideas.clone(),
            "Ideas
body newer"
                .into()
        )]
    );
}
