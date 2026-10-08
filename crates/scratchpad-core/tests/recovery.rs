mod common;

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use common::{file_names, temp_store};
use scratchpad_core::RecoveryStore;

fn recovery_store() -> (tempfile::TempDir, RecoveryStore) {
    let dir = tempfile::tempdir().unwrap();
    // A folder that does not exist yet, like on first run.
    let store = RecoveryStore::new(dir.path().join("Scratchpad").join("recovery"));
    (dir, store)
}

#[test]
fn snapshots_round_trip_and_stay_out_of_the_notes_folder() {
    let (notes_dir, notes) = temp_store();
    let (_recovery_dir, recovery) = recovery_store();
    let first = notes.create(Some("First")).unwrap();
    let second = notes.create(Some("Second")).unwrap();
    let before = SystemTime::now();

    recovery
        .write(&first.path, "unsaved \u{1F44B}\r\nwork")
        .unwrap();
    recovery.write(&second.path, "other").unwrap();

    let mut snapshots = recovery.list();
    snapshots.sort_by(|a, b| a.note_path.cmp(&b.note_path));
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].note_path, first.path);
    assert_eq!(snapshots[0].text, "unsaved \u{1F44B}\r\nwork");
    assert_eq!(snapshots[1].note_path, second.path);
    assert!(snapshots.iter().all(|s| s.saved_at >= before));
    assert_eq!(file_names(notes_dir.path()), ["First.md", "Second.md"]);
    assert_eq!(notes.read(&first.path).unwrap().text, "");
}

#[test]
fn writing_again_replaces_the_snapshot_of_that_note() {
    let (_dir, recovery) = recovery_store();
    let note = Path::new("C:/Notes/Draft.md");
    recovery.write(note, "v1").unwrap();
    recovery.write(note, "v2").unwrap();

    let snapshots = recovery.list();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].text, "v2");
}

#[test]
fn removing_a_snapshot_forgets_only_that_note() {
    let (_dir, recovery) = recovery_store();
    let kept = Path::new("C:/Notes/Kept.md");
    let saved = Path::new("C:/Notes/Saved.md");
    recovery.write(kept, "keep me").unwrap();
    recovery.write(saved, "now on disk").unwrap();

    recovery.remove(saved).unwrap();

    let snapshots = recovery.list();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].note_path, kept);
}

#[test]
fn removing_a_missing_snapshot_is_not_an_error() {
    let (_dir, recovery) = recovery_store();
    recovery.remove(Path::new("C:/Notes/Never.md")).unwrap();
}

#[test]
fn listing_without_a_recovery_folder_is_empty_and_creates_nothing() {
    let (dir, recovery) = recovery_store();
    assert!(recovery.list().is_empty());
    assert!(file_names(dir.path()).is_empty());
}

#[test]
fn corrupt_snapshots_are_ignored_but_valid_ones_survive() {
    let (dir, recovery) = recovery_store();
    recovery
        .write(Path::new("C:/Notes/Good.md"), "precious")
        .unwrap();
    let folder = dir.path().join("Scratchpad").join("recovery");
    fs::write(
        folder.join("0000000000000001.json"),
        b"{\"note_path\": \"C:/x.md\", \"te",
    )
    .unwrap();
    fs::write(folder.join("0000000000000002.json"), b"").unwrap();
    fs::write(folder.join("0000000000000003.json"), b"\xFF\xFE binary").unwrap();
    fs::write(folder.join("notes.txt"), b"not a snapshot").unwrap();

    let snapshots = recovery.list();

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].text, "precious");
}

#[test]
fn a_failed_write_is_reported() {
    let (dir, _recovery) = recovery_store();
    // The recovery "folder" is a file, so nothing can be written below it.
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "file").unwrap();
    let recovery = RecoveryStore::new(blocker.join("recovery"));

    let error = recovery
        .write(Path::new("C:/Notes/Draft.md"), "text")
        .unwrap_err();

    assert!(error.to_string().contains("recovery"), "{error}");
    assert!(recovery.list().is_empty());
}

#[test]
fn leftovers_are_snapshots_from_before_this_run_that_differ_from_their_note() {
    let (_notes_dir, notes) = temp_store();
    let (_recovery_dir, recovery) = recovery_store();
    let unsaved = notes.create(Some("Unsaved")).unwrap();
    let saved = notes.create(Some("Saved")).unwrap();
    let gone = notes.create(Some("Gone")).unwrap();
    notes.save(&saved.path, "same text").unwrap();
    recovery.write(&unsaved.path, "lost work").unwrap();
    recovery.write(&saved.path, "same text").unwrap();
    recovery.write(&gone.path, "orphan").unwrap();
    fs::remove_file(&gone.path).unwrap();
    let started = SystemTime::now();
    let current = notes.dir().join("Current.md");
    recovery.write(&current, "this run").unwrap();

    let mut leftovers: Vec<_> = recovery
        .leftovers(started)
        .into_iter()
        .map(|snapshot| (snapshot.note_path, snapshot.text))
        .collect();
    leftovers.sort();

    assert_eq!(
        leftovers,
        [
            (gone.path.clone(), "orphan".to_owned()),
            (unsaved.path.clone(), "lost work".to_owned())
        ]
    );
    // The snapshot that matched its note is removed; this run's snapshot is left alone.
    let mut remaining: Vec<_> = recovery.list().into_iter().map(|s| s.note_path).collect();
    remaining.sort();
    assert_eq!(remaining, [current, gone.path, unsaved.path]);
}
