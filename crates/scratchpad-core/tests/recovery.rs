mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use common::{file_names, temp_store};
use scratchpad_core::RecoveryStore;

fn recovery_store() -> (tempfile::TempDir, RecoveryStore) {
    let dir = tempfile::tempdir().unwrap();
    // A folder that does not exist yet, like on first run.
    let store = RecoveryStore::new(recovery_folder(&dir));
    (dir, store)
}

fn recovery_folder(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("Scratchpad").join("recovery")
}

/// The folders of the instances in the recovery folder `root`.
fn instance_folders(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect()
}

/// The folder of the only instance that has one in `root`.
fn instance_folder(root: &Path) -> PathBuf {
    match &instance_folders(root)[..] {
        [folder] => folder.clone(),
        folders => panic!("expected one instance folder, got {folders:?}"),
    }
}

/// The note paths and texts of `snapshots`, sorted.
fn texts(snapshots: Vec<scratchpad_core::Snapshot>) -> Vec<(PathBuf, String)> {
    let mut texts: Vec<_> = snapshots
        .into_iter()
        .map(|snapshot| (snapshot.note_path, snapshot.text))
        .collect();
    texts.sort();
    texts
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

/// The file name is a hash of the note path. A different hash in a later version would orphan the
/// snapshots an earlier version left behind, so the value is pinned.
#[test]
fn snapshot_file_names_are_stable_across_versions() {
    let (dir, recovery) = recovery_store();
    recovery
        .write(Path::new("C:/Notes/Draft.md"), "text")
        .unwrap();

    let folder = instance_folder(&recovery_folder(&dir));
    assert_eq!(file_names(&folder), ["5ca43ed72c936d45.json"]);
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
fn a_snapshot_that_cannot_be_removed_is_reported() {
    let (dir, recovery) = recovery_store();
    recovery
        .write(Path::new("C:/Notes/Other.md"), "other")
        .unwrap();
    // A folder where the snapshot of Draft.md would be (see the pinned file name above).
    let folder = instance_folder(&recovery_folder(&dir));
    fs::create_dir(folder.join("5ca43ed72c936d45.json")).unwrap();

    let error = recovery.remove(Path::new("C:/Notes/Draft.md")).unwrap_err();

    assert!(
        error.to_string().contains("remove recovery snapshot"),
        "{error}"
    );
    // The folder is no snapshot; the real one is still listed.
    assert_eq!(recovery.list().len(), 1);
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
    let folder = instance_folder(&recovery_folder(&dir));
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
fn leftovers_are_snapshots_of_a_crashed_run_that_differ_from_their_note() {
    let (_notes_dir, notes) = temp_store();
    let (dir, crashed) = recovery_store();
    let unsaved = notes.create(Some("Unsaved")).unwrap();
    let saved = notes.create(Some("Saved")).unwrap();
    let gone = notes.create(Some("Gone")).unwrap();
    notes.save(&saved.path, "same text").unwrap();
    crashed.write(&unsaved.path, "lost work").unwrap();
    crashed.write(&saved.path, "same text").unwrap();
    crashed.write(&gone.path, "orphan").unwrap();
    fs::remove_file(&gone.path).unwrap();
    drop(crashed);
    let recovery = RecoveryStore::new(recovery_folder(&dir));
    let current = notes.dir().join("Current.md");
    recovery.write(&current, "this run").unwrap();

    assert_eq!(
        texts(recovery.leftovers()),
        [
            (gone.path.clone(), "orphan".to_owned()),
            (unsaved.path.clone(), "lost work".to_owned())
        ]
    );
    // The snapshot that matched its note is removed; this run's snapshot is left alone.
    assert_eq!(
        texts(recovery.list()),
        [
            (current, "this run".to_owned()),
            (gone.path, "orphan".to_owned()),
            (unsaved.path, "lost work".to_owned())
        ]
    );
}

/// Writes a snapshot file the way `RecoveryStore::write` does, but with a chosen time.
fn write_snapshot_saved_at(dir: &Path, file: &str, note: &Path, text: &str, secs: u64) {
    let json = format!(
        r#"{{"note_path": {note:?}, "text": {text:?}, "saved_at": {{"secs_since_epoch": {secs}, "nanos_since_epoch": 0}}}}"#
    );
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(file), json).unwrap();
}

#[test]
fn snapshots_are_listed_newest_first() {
    let (dir, recovery) = recovery_store();
    let folder = recovery_folder(&dir);
    // File names are in the opposite order of the times, so sorting by name would fail.
    write_snapshot_saved_at(&folder, "a.json", Path::new("Oldest.md"), "1", 1_000);
    write_snapshot_saved_at(&folder, "b.json", Path::new("Newest.md"), "3", 3_000);
    write_snapshot_saved_at(&folder, "c.json", Path::new("Middle.md"), "2", 2_000);

    let texts: Vec<_> = recovery.list().into_iter().map(|s| s.text).collect();
    assert_eq!(texts, ["3", "2", "1"]);

    let texts: Vec<_> = recovery.leftovers().into_iter().map(|s| s.text).collect();
    assert_eq!(texts, ["3", "2", "1"]);
}

#[test]
fn a_note_that_is_not_utf8_never_counts_as_saved() {
    let (notes_dir, notes) = temp_store();
    let (dir, crashed) = recovery_store();
    let path = notes_dir.path().join("Broken.md");
    fs::write(&path, b"caf\xE9").unwrap();
    // What reading that note shows, and so what the user may have kept typing after.
    let shown = notes.read(&path).unwrap();
    assert!(shown.lossy);
    crashed.write(&path, &shown.text).unwrap();
    drop(crashed);
    let recovery = RecoveryStore::new(recovery_folder(&dir));

    let leftovers = recovery.leftovers();

    // Equal text is not proof: saving it would have replaced the original bytes with U+FFFD.
    assert_eq!(leftovers.len(), 1);
    assert_eq!(recovery.list().len(), 1, "the snapshot is kept");
}

// --- Several instances at once (ADR 0155) ---

#[test]
fn a_running_instance_keeps_its_snapshots_until_it_is_gone() {
    let (dir, first) = recovery_store();
    let note = Path::new("C:/Notes/Typing.md");
    first.write(note, "still typing").unwrap();

    // A second instance starts while the first runs.
    let second = RecoveryStore::new(recovery_folder(&dir));
    assert!(second.leftovers().is_empty());
    second.remove(note).unwrap();
    assert_eq!(texts(first.list()), [(note.into(), "still typing".into())]);

    // The first one crashes (or quits with text it could not save).
    drop(first);
    assert_eq!(
        texts(second.leftovers()),
        [(note.into(), "still typing".into())]
    );
}

/// A cleanup tool (or the user) can remove the folder while its lock stays held; snapshots
/// must not stop for the rest of the run.
#[test]
fn a_running_instance_makes_its_removed_folder_again() {
    let (dir, recovery) = recovery_store();
    let note = Path::new("C:/Notes/Draft.md");
    recovery.write(note, "v1").unwrap();
    fs::remove_dir_all(instance_folder(&recovery_folder(&dir))).unwrap();

    recovery.write(note, "v2").unwrap();

    assert_eq!(texts(recovery.list()), [(note.into(), "v2".into())]);
}

#[test]
fn leftovers_move_to_the_instance_that_offers_them() {
    let (dir, crashed) = recovery_store();
    let restored = Path::new("C:/Notes/Restored.md");
    let discarded = Path::new("C:/Notes/Discarded.md");
    crashed.write(restored, "one").unwrap();
    crashed.write(discarded, "two").unwrap();
    drop(crashed);
    let root = recovery_folder(&dir);

    let recovery = RecoveryStore::new(&root);
    assert_eq!(recovery.leftovers().len(), 2);

    // The crashed instance's folder and lock are gone; only this instance's remain.
    let own = instance_folder(&root);
    let own_name = own.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        file_names(&root),
        [own_name.clone(), format!("{own_name}.lock")]
    );
    // A third instance does not offer them again while this one runs.
    assert!(RecoveryStore::new(&root).leftovers().is_empty());
    // Restoring (once saved) and discarding remove them as this instance's own.
    recovery.remove(restored).unwrap();
    recovery.remove(discarded).unwrap();
    assert!(recovery.list().is_empty());
    drop(recovery);
    // An empty folder left by an instance that is gone is cleaned up as well.
    let next = RecoveryStore::new(&root);
    assert!(next.leftovers().is_empty());
    assert!(file_names(&root).is_empty());
}

#[test]
fn snapshots_from_before_instance_folders_are_leftovers() {
    let (dir, recovery) = recovery_store();
    let root = recovery_folder(&dir);
    let note = Path::new("C:/Notes/Draft.md");
    // Where earlier versions kept the snapshot of Draft.md (see the pinned name above).
    write_snapshot_saved_at(&root, "5ca43ed72c936d45.json", note, "old work", 1_000);

    assert_eq!(
        texts(recovery.leftovers()),
        [(note.into(), "old work".into())]
    );
    // Taken over: no other instance offers it, and discarding it removes it.
    assert!(RecoveryStore::new(&root).leftovers().is_empty());
    recovery.remove(note).unwrap();
    assert!(recovery.list().is_empty());
}

#[test]
fn a_second_snapshot_of_the_same_note_waits_for_the_next_start() {
    let (dir, recovery) = recovery_store();
    let root = recovery_folder(&dir);
    let note = Path::new("C:/Notes/Both.md");
    // Two instances crashed while both had unsaved text in the same note.
    for text in ["from one", "from two"] {
        let crashed = RecoveryStore::new(&root);
        crashed.write(note, text).unwrap();
    }

    let first = texts(recovery.leftovers());
    assert_eq!(first.len(), 1);
    recovery.remove(note).unwrap();
    drop(recovery);
    let second = texts(RecoveryStore::new(&root).leftovers());

    let mut offered: Vec<_> = first
        .into_iter()
        .chain(second)
        .map(|(_, text)| text)
        .collect();
    offered.sort();
    assert_eq!(offered, ["from one", "from two"]);
}

/// Set by [`another_process_keeps_its_snapshots_until_it_is_killed`] for its child process.
const CHILD_RECOVERY_DIR_ENV: &str = "SCRATCHPAD_TEST_CHILD_RECOVERY_DIR";
/// What the child process prints once its snapshot is written.
const CHILD_READY: &str = "recovery snapshot written";

/// Another process is a running instance until it ends, however abruptly. Release builds abort
/// on panic (ADR 0021): no destructor runs, so only the OS can release the lock.
#[test]
fn another_process_keeps_its_snapshots_until_it_is_killed() {
    let (dir, recovery) = recovery_store();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "write_a_snapshot_and_wait",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_RECOVERY_DIR_ENV, recovery_folder(&dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let ready = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains(CHILD_READY));
    assert!(ready, "the child process ended before writing its snapshot");

    assert!(recovery.leftovers().is_empty(), "the child process runs");

    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        texts(recovery.leftovers()),
        [(PathBuf::from("C:/Notes/Crash.md"), "unsaved".into())]
    );
}

#[test]
#[ignore = "run by another_process_keeps_its_snapshots_until_it_is_killed in a child process"]
fn write_a_snapshot_and_wait() {
    let dir = std::env::var_os(CHILD_RECOVERY_DIR_ENV).expect("run by the parent test");
    let recovery = RecoveryStore::new(dir);
    recovery
        .write(Path::new("C:/Notes/Crash.md"), "unsaved")
        .unwrap();
    println!("{CHILD_READY}");
    // Until killed. Should the parent fail first, its end of stdin closes and this returns.
    let _ = std::io::stdin().read(&mut [0]);
}
