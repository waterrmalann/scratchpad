mod common;

use std::fs;
use std::io;

use common::{at, file_names, path_in, set_modified, temp_store};
use scratchpad_core::NoteStore;

fn titles(store: &NoteStore) -> Vec<String> {
    store
        .list()
        .unwrap()
        .into_iter()
        .map(|note| note.title)
        .collect()
}

#[test]
fn open_creates_a_missing_notes_folder() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("Documents").join("Scratchpad");
    let store = NoteStore::open(&nested).unwrap();
    assert!(nested.is_dir());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn create_save_read_round_trip() {
    let (_dir, store) = temp_store();
    let note = store.create(Some("Ideas for webhook gateway")).unwrap();
    assert_eq!(note.title, "Ideas for webhook gateway");
    assert_eq!(
        note.path.file_name().unwrap(),
        "Ideas for webhook gateway.md"
    );

    // CRLF, trailing newline-less text and non-ASCII must come back byte for byte.
    let text = "# Grüße 👋\r\nsecond line\r\n\r\nlast";
    store.save(&note.path, text).unwrap();
    let read = store.read(&note.path).unwrap();
    assert_eq!(read.text, text);
    assert!(!read.lossy);
    assert_eq!(store.note(&note.path).unwrap().len, text.len() as u64);
}

#[test]
fn list_is_sorted_by_modification_time_newest_first() {
    let (_dir, store) = temp_store();
    let old = store.create(Some("old")).unwrap();
    let new = store.create(Some("new")).unwrap();
    let middle = store.create(Some("middle")).unwrap();
    set_modified(&old.path, at(1_000));
    set_modified(&middle.path, at(2_000));
    set_modified(&new.path, at(3_000));

    assert_eq!(titles(&store), ["new", "middle", "old"]);

    // Saving moves a note to the top.
    set_modified(&new.path, at(500));
    assert_eq!(titles(&store), ["middle", "old", "new"]);
    store.save(&new.path, "edited").unwrap();
    assert_eq!(titles(&store)[0], "new");
}

#[test]
fn list_only_reports_visible_markdown_files_in_the_folder() {
    let (dir, store) = temp_store();
    fs::write(path_in(&dir, "Plain.md"), "x").unwrap();
    fs::write(path_in(&dir, "Shouting.MD"), "x").unwrap();
    fs::write(path_in(&dir, "readme.txt"), "x").unwrap();
    fs::write(path_in(&dir, "noext"), "x").unwrap();
    fs::write(path_in(&dir, ".dotfile.md"), "x").unwrap();
    fs::write(path_in(&dir, ".scratchpad-1234-0.tmp"), "x").unwrap();
    fs::create_dir(path_in(&dir, "folder.md")).unwrap();
    fs::create_dir(path_in(&dir, "sub")).unwrap();
    fs::write(path_in(&dir, "sub").join("Nested.md"), "x").unwrap();

    let mut found = titles(&store);
    found.sort();
    assert_eq!(found, ["Plain", "Shouting"]);
}

#[cfg(windows)]
#[test]
fn list_skips_files_with_the_windows_hidden_attribute() {
    let (dir, store) = temp_store();
    fs::write(path_in(&dir, "Visible.md"), "x").unwrap();
    let hidden = path_in(&dir, "Hidden.md");
    fs::write(&hidden, "x").unwrap();
    let status = std::process::Command::new("attrib")
        .arg("+h")
        .arg(&hidden)
        .status()
        .unwrap();
    assert!(status.success());

    assert_eq!(titles(&store), ["Visible"]);
}

#[test]
fn list_reports_metadata_from_the_filesystem() {
    let (_dir, store) = temp_store();
    let note = store.create(Some("Meta")).unwrap();
    store.save(&note.path, "12345").unwrap();
    set_modified(&note.path, at(42));

    let listed = store.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, note.path);
    assert_eq!(listed[0].title, "Meta");
    assert_eq!(listed[0].modified_at, at(42));
    assert_eq!(listed[0].len, 5);
}

#[test]
fn reading_malformed_utf8_succeeds_and_reports_loss() {
    let (dir, store) = temp_store();
    let path = path_in(&dir, "Broken.md");
    fs::write(&path, b"caf\xE9 ok\xFF").unwrap();

    let read = store.read(&path).unwrap();
    assert!(read.lossy);
    assert_eq!(read.text, "caf\u{FFFD} ok\u{FFFD}");
}

#[test]
fn reading_drops_a_utf8_byte_order_mark() {
    let (dir, store) = temp_store();
    let path = path_in(&dir, "Notepad.md");
    fs::write(&path, b"\xEF\xBB\xBFhello").unwrap();

    let read = store.read(&path).unwrap();
    assert_eq!(read.text, "hello");
    assert!(!read.lossy);
}

#[test]
fn reading_a_missing_note_reports_not_found_with_the_file_name() {
    let (dir, store) = temp_store();
    let error = store.read(&path_in(&dir, "Gone.md")).unwrap_err();
    assert_eq!(error.io_kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains("Gone.md"), "{error}");
}

#[test]
fn created_notes_get_unique_names_ignoring_case() {
    let (_dir, store) = temp_store();
    let names: Vec<String> = [
        None,
        None,
        None,
        Some("Ideas"),
        Some("ideas"),
        Some("IDEAS"),
    ]
    .into_iter()
    .map(|title| {
        let note = store.create(title).unwrap();
        note.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    })
    .collect();
    assert_eq!(
        names,
        [
            "Untitled.md",
            "Untitled 2.md",
            "Untitled 3.md",
            "Ideas.md",
            "ideas 2.md",
            "IDEAS 3.md"
        ]
    );
}

#[test]
fn created_note_names_are_sanitized() {
    let (_dir, store) = temp_store();
    let note = store.create(Some("What/why: a \"test\"?")).unwrap();
    assert_eq!(note.title, "What why a test");
    let reserved = store.create(Some("CON")).unwrap();
    assert_eq!(reserved.title, "CON_");
    let nothing = store.create(Some("???")).unwrap();
    assert_eq!(nothing.title, "Untitled");
}

#[test]
fn rename_changes_the_file_name_and_keeps_the_content() {
    let (dir, store) = temp_store();
    let note = store.create(Some("Draft")).unwrap();
    store.save(&note.path, "body").unwrap();

    let renamed = store.rename(&note.path, "Final: v2").unwrap();
    assert_eq!(renamed, path_in(&dir, "Final v2.md"));
    assert_eq!(file_names(dir.path()), ["Final v2.md"]);
    assert_eq!(store.read(&renamed).unwrap().text, "body");
}

#[test]
fn rename_avoids_collisions_with_other_notes() {
    let (dir, store) = temp_store();
    let ideas = store.create(Some("Ideas")).unwrap();
    let other = store.create(Some("Other")).unwrap();
    store.save(&ideas.path, "ideas").unwrap();

    let renamed = store.rename(&other.path, "ideas").unwrap();
    assert_eq!(renamed, path_in(&dir, "ideas 2.md"));
    assert_eq!(store.read(&ideas.path).unwrap().text, "ideas");
}

#[test]
fn rename_to_the_current_name_changes_nothing() {
    let (dir, store) = temp_store();
    let _first = store.create(Some("Ideas")).unwrap();
    let second = store.create(Some("Ideas")).unwrap();
    assert_eq!(second.path, path_in(&dir, "Ideas 2.md"));

    assert_eq!(store.rename(&second.path, "Ideas 2").unwrap(), second.path);
    // Asking for the taken name again keeps the existing suffix instead of creating "Ideas 3".
    assert_eq!(store.rename(&second.path, "Ideas").unwrap(), second.path);
    assert_eq!(file_names(dir.path()), ["Ideas 2.md", "Ideas.md"]);
}

#[test]
fn rename_that_only_changes_case_works() {
    let (dir, store) = temp_store();
    let note = store.create(Some("meeting notes")).unwrap();
    store.save(&note.path, "body").unwrap();

    let renamed = store.rename(&note.path, "Meeting Notes").unwrap();
    assert_eq!(renamed, path_in(&dir, "Meeting Notes.md"));
    assert_eq!(file_names(dir.path()), ["Meeting Notes.md"]);
    assert_eq!(store.read(&renamed).unwrap().text, "body");
}

#[test]
fn rename_preserves_the_extension_and_falls_back_to_untitled() {
    let (dir, store) = temp_store();
    let path = path_in(&dir, "Shouting.MD");
    fs::write(&path, "x").unwrap();

    assert_eq!(
        store.rename(&path, "Quiet").unwrap(),
        path_in(&dir, "Quiet.MD")
    );
    assert_eq!(
        store.rename(&path_in(&dir, "Quiet.MD"), "///").unwrap(),
        path_in(&dir, "Untitled.MD")
    );
}

#[test]
fn rename_of_a_missing_note_fails_without_creating_anything() {
    let (dir, store) = temp_store();
    let error = store.rename(&path_in(&dir, "Gone.md"), "New").unwrap_err();
    assert_eq!(error.io_kind(), io::ErrorKind::NotFound);
    assert!(file_names(dir.path()).is_empty());
}

#[test]
fn delete_hands_the_file_to_the_deleter() {
    let (dir, store) = temp_store();
    let keep = store.create(Some("Keep")).unwrap();
    let doomed = store.create(Some("Doomed")).unwrap();
    store.save(&doomed.path, "bye").unwrap();

    store.delete(&doomed.path).unwrap();

    assert_eq!(titles(&store), ["Keep"]);
    assert!(keep.path.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join(".trash").join("Doomed.md")).unwrap(),
        "bye"
    );
}

#[test]
fn delete_failures_are_reported_with_the_note_name() {
    let (dir, store) = temp_store();
    let error = store.delete(&path_in(&dir, "Ghost.md")).unwrap_err();
    assert_eq!(error.io_kind(), io::ErrorKind::NotFound);
    assert!(error.to_string().contains("Ghost.md"), "{error}");
}

#[test]
fn save_replaces_content_without_leaving_temporary_files() {
    let (dir, store) = temp_store();
    let note = store.create(Some("Note")).unwrap();
    for i in 0..5 {
        store.save(&note.path, &format!("version {i}")).unwrap();
    }
    assert_eq!(store.read(&note.path).unwrap().text, "version 4");
    assert_eq!(file_names(dir.path()), ["Note.md"]);
}

#[test]
fn failed_save_leaves_no_temporary_file_and_reports_the_error() {
    let (dir, store) = temp_store();
    // A directory in the way makes the final rename impossible.
    let blocker = path_in(&dir, "Blocked.md");
    fs::create_dir(&blocker).unwrap();
    fs::write(blocker.join("inside.txt"), "precious").unwrap();

    let error = store.save(&blocker, "text").unwrap_err();

    assert!(error.to_string().contains("Blocked.md"), "{error}");
    assert_eq!(file_names(dir.path()), ["Blocked.md"]);
    assert_eq!(
        fs::read_to_string(blocker.join("inside.txt")).unwrap(),
        "precious"
    );
}

#[test]
fn save_works_for_notes_with_names_near_the_length_limit() {
    let (dir, store) = temp_store();
    // Created by another tool; 245 of the 255 characters a file name may have.
    let path = path_in(&dir, &format!("{}.md", "x".repeat(242)));
    fs::write(&path, "before").unwrap();

    store.save(&path, "after").unwrap();

    assert_eq!(store.read(&path).unwrap().text, "after");
}

#[test]
fn open_removes_stale_temporary_files_left_by_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let stale = path_in(&dir, ".scratchpad-4242-0.tmp");
    fs::write(&stale, "half-written").unwrap();
    set_modified(&stale, at(1_000));
    // Possibly another running instance in the middle of a save.
    let fresh = path_in(&dir, ".scratchpad-4243-0.tmp");
    fs::write(&fresh, "being written").unwrap();
    let unrelated = path_in(&dir, "backup.tmp");
    fs::write(&unrelated, "not ours").unwrap();
    set_modified(&unrelated, at(1_000));

    NoteStore::open(dir.path()).unwrap();

    assert_eq!(
        file_names(dir.path()),
        [".scratchpad-4243-0.tmp", "backup.tmp"]
    );
}

#[cfg(windows)]
#[test]
fn save_keeps_the_creation_time() {
    use std::os::windows::fs::FileTimesExt;

    let (_dir, store) = temp_store();
    let note = store.create(Some("Old")).unwrap();
    fs::File::options()
        .write(true)
        .open(&note.path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_created(at(1_000_000)))
        .unwrap();

    store.save(&note.path, "edited").unwrap();

    assert_eq!(
        store.note(&note.path).unwrap().created_at,
        Some(at(1_000_000))
    );
}

#[test]
fn save_into_a_vanished_folder_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes");
    let store = NoteStore::open(&notes).unwrap();
    let note = store.create(Some("Orphan")).unwrap();
    fs::remove_dir_all(&notes).unwrap();

    let error = store.save(&note.path, "text").unwrap_err();
    assert_eq!(error.io_kind(), io::ErrorKind::NotFound);
    assert!(!notes.exists());
}

// On Unix a read-only file can still be replaced via rename, so this is Windows behaviour only.
#[cfg(windows)]
#[test]
fn save_to_a_read_only_note_fails_and_keeps_the_original() {
    let (dir, store) = temp_store();
    let note = store.create(Some("Locked")).unwrap();
    store.save(&note.path, "original").unwrap();
    let mut permissions = fs::metadata(&note.path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&note.path, permissions).unwrap();

    let error = store.save(&note.path, "replacement").unwrap_err();

    assert_eq!(error.io_kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(store.read(&note.path).unwrap().text, "original");
    assert_eq!(file_names(dir.path()), ["Locked.md"]);
}
