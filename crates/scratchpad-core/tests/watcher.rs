//! These tests use the real OS notification API, so they wait for events with a generous
//! timeout instead of sleeping for a fixed time, and tolerate duplicate or extra events.

mod common;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{path_in, temp_store};
use scratchpad_core::{NoteEvent, NoteWatcher};

const TIMEOUT: Duration = Duration::from_secs(15);

/// Collects events until `done` returns true for the events seen so far.
fn collect_until(watcher: &NoteWatcher, done: impl Fn(&[NoteEvent]) -> bool) -> Vec<NoteEvent> {
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    while !done(&seen) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match watcher.recv_timeout(remaining) {
            Some(event) => seen.push(event),
            None => panic!("timed out waiting for filesystem events, saw {seen:?}"),
        }
    }
    seen
}

fn named(event: &NoteEvent, name: &str) -> bool {
    event.path().file_name() == Some(OsStr::new(name))
}

#[test]
fn reports_an_external_modification() {
    let (dir, _store) = temp_store();
    let path = path_in(&dir, "Meeting.md");
    fs::write(&path, "before").unwrap();
    let watcher = NoteWatcher::start(dir.path()).unwrap();

    fs::write(&path, "changed by git").unwrap();

    let events = collect_until(&watcher, |seen| {
        seen.iter()
            .any(|e| matches!(e, NoteEvent::Changed(_)) && named(e, "Meeting.md"))
    });
    assert!(events.iter().all(|e| named(e, "Meeting.md")));
}

#[test]
fn reports_created_and_removed_notes() {
    let (dir, _store) = temp_store();
    let watcher = NoteWatcher::start(dir.path()).unwrap();
    let path = path_in(&dir, "New.md");

    fs::write(&path, "hello").unwrap();
    collect_until(&watcher, |seen| {
        seen.iter()
            .any(|e| matches!(e, NoteEvent::Created(_)) && named(e, "New.md"))
    });

    fs::remove_file(&path).unwrap();
    collect_until(&watcher, |seen| {
        seen.iter()
            .any(|e| matches!(e, NoteEvent::Removed(_)) && named(e, "New.md"))
    });
}

#[test]
fn a_rename_is_a_removal_plus_a_creation() {
    let (dir, _store) = temp_store();
    fs::write(path_in(&dir, "Old.md"), "x").unwrap();
    let watcher = NoteWatcher::start(dir.path()).unwrap();

    fs::rename(path_in(&dir, "Old.md"), path_in(&dir, "New.md")).unwrap();

    collect_until(&watcher, |seen| {
        seen.iter()
            .any(|e| matches!(e, NoteEvent::Removed(_)) && named(e, "Old.md"))
            && seen
                .iter()
                .any(|e| matches!(e, NoteEvent::Created(_)) && named(e, "New.md"))
    });
}

#[test]
fn ignores_other_files_and_our_own_temporary_files() {
    let (dir, store) = temp_store();
    let note = store.create(Some("Note")).unwrap();
    let watcher = NoteWatcher::start(dir.path()).unwrap();

    fs::write(path_in(&dir, "readme.txt"), "x").unwrap();
    fs::write(path_in(&dir, ".hidden.md"), "x").unwrap();
    // An atomic save creates, fills and renames a hidden temporary file.
    store.save(&note.path, "saved").unwrap();
    // Events arrive in order, so once the sentinel is seen everything before it was reported.
    fs::write(path_in(&dir, "Sentinel.md"), "x").unwrap();

    let events = collect_until(&watcher, |seen| {
        seen.iter().any(|e| named(e, "Sentinel.md"))
    });

    let reported: Vec<&Path> = events.iter().map(NoteEvent::path).collect();
    for path in reported {
        let name = path.file_name().unwrap();
        assert!(
            name == "Note.md" || name == "Sentinel.md",
            "unexpected event for {path:?}"
        );
    }
    assert!(
        events.iter().any(|e| named(e, "Note.md")),
        "the save was not reported: {events:?}"
    );
}

#[test]
fn replacing_a_note_by_rename_is_not_reported_as_a_removal() {
    let (dir, store) = temp_store();
    let note = store.create(Some("Open")).unwrap();
    let watcher = NoteWatcher::start(dir.path()).unwrap();

    // Windows reports the replaced target as removed before the temporary file takes its name;
    // an app trusting that would close the note it is editing.
    store.save(&note.path, "saved").unwrap();
    fs::write(path_in(&dir, "Sentinel.md"), "x").unwrap();

    let events = collect_until(&watcher, |seen| {
        seen.iter().any(|e| named(e, "Sentinel.md"))
    });
    assert!(
        !events.iter().any(|e| matches!(e, NoteEvent::Removed(_))),
        "{events:?}"
    );
    assert!(events.iter().any(|e| named(e, "Open.md")), "{events:?}");
}

#[test]
fn starting_on_a_missing_folder_fails_with_the_folder_in_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let error = NoteWatcher::start(&missing).err().expect("must fail");
    assert!(error.to_string().contains("nope"), "{error}");
}
