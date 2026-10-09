//! File > Open (Ctrl+O): files from outside the notes folder are edited in place with the same
//! guarantees as notes, plain text files without Markdown styling (ADRs 0145, 0147).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{click, days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::file_dialogs::PickFileForTests;
use scratchpad::notes::{NotesEvent, Selection};
use scratchpad::session::{AUTOSAVE_DELAY, Notice, SNAPSHOT_INTERVAL};
use scratchpad::{Storage, toast};
use scratchpad_core::{NoteEvent, RecoveryStore};
use tempfile::TempDir;

/// A notes folder with one note, a folder elsewhere for other files and one for the config and
/// recovery data, so the app can be restarted on the same folders.
struct Folders {
    notes: TempDir,
    elsewhere: TempDir,
    data: TempDir,
}

impl Folders {
    fn new() -> Self {
        let folders = Self {
            notes: tempfile::tempdir().unwrap(),
            elsewhere: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
        };
        write_note(
            folders.notes.path(),
            "Ideas",
            "Ideas\nbody",
            days_ago(0, 10),
        );
        folders
    }

    fn storage(&self) -> Storage {
        common::storage(self.notes.path(), self.data.path())
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
        common::open_with(self.storage(), cx)
    }

    /// Writes `name` outside the notes folder.
    fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.elsewhere.path().join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    fn ideas(&self) -> PathBuf {
        self.notes.path().join("Ideas.md")
    }
}

/// Ctrl+O, picking `path` in the dialog.
fn open_file(path: &Path, cx: &mut VisualTestContext) {
    cx.update(|_, cx| cx.set_global(PickFileForTests(Some(path.to_owned()))));
    cx.simulate_keystrokes("ctrl-o");
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

fn is_read_only(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    common::editor(root, cx).read_with(cx, |editor, _| editor.is_read_only())
}

fn notices(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Vec<Notice> {
    let session = common::session(root, cx);
    session.read_with(cx, |session, _| session.notices())
}

fn report(event: NoteEvent, root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let session = common::session(root, cx);
    session.update(cx, |session, cx| session.disk_events(vec![event], cx));
    cx.run_until_parked();
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[gpui::test]
fn a_text_file_is_edited_in_place_as_plain_text_and_keeps_its_line_endings(
    cx: &mut TestAppContext,
) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"# not a heading\r\nsecond\r\n");
    let (root, cx) = folders.open(cx);

    open_file(&log, cx);

    assert_eq!(selection(&root, cx), Selection::File(log.clone()));
    assert_eq!(editor_text(&root, cx), "# not a heading\r\nsecond\r\n");
    assert!(!is_markdown(&root, cx));
    assert!(
        cx.debug_bounds("open-file").is_some(),
        "the sidebar shows it"
    );
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input("third");
    cx.simulate_keystrokes("enter");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        "# not a heading\r\nsecond\r\nthird\r\n"
    );

    // A changed first line never renames it, and it never joins the notes.
    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("Renamed? ");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(file_names(folders.elsewhere.path()), ["log.txt"]);
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .starts_with("Renamed? # not")
    );
    assert_eq!(listed(&root, cx), [folders.ideas()]);
    assert_eq!(titles_on_disk(folders.notes.path()), ["Ideas"]);
    assert_eq!(fs::read_to_string(folders.ideas()).unwrap(), "Ideas\nbody");

    // A new note is Markdown again.
    cx.simulate_keystrokes("ctrl-n");
    assert!(is_markdown(&root, cx));
}

#[gpui::test]
fn a_markdown_file_from_elsewhere_gets_live_preview(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let readme = folders.file("README.markdown", b"# Readme\nText");
    let (root, cx) = folders.open(cx);
    let events = common::EventLog::new(&common::notes(&root, cx), cx);

    open_file(&readme, cx);

    assert_eq!(events.take(), [NotesEvent::OpenFile(readme.clone())]);
    assert!(is_markdown(&root, cx));
    assert_eq!(editor_text(&root, cx), "# Readme\nText");
}

#[gpui::test]
fn a_text_file_in_the_notes_folder_is_not_a_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let todo = folders.notes.path().join("todo.txt");
    fs::write(&todo, "todo").unwrap();
    let (root, cx) = folders.open(cx);

    open_file(&todo, cx);
    cx.simulate_input("more ");
    cx.simulate_keystrokes("ctrl-s");

    assert_eq!(fs::read_to_string(&todo).unwrap(), "more todo");
    assert_eq!(selection(&root, cx), Selection::File(todo));
    assert_eq!(listed(&root, cx), [folders.ideas()]);
}

#[gpui::test]
fn a_note_of_the_notes_folder_opens_as_a_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    write_note(folders.notes.path(), "Newer", "Newer", days_ago(0, 11));
    let (root, cx) = folders.open(cx);
    assert_eq!(
        selection(&root, cx),
        Selection::Note(folders.notes.path().join("Newer.md"))
    );

    open_file(&folders.ideas(), cx);

    assert_eq!(selection(&root, cx), Selection::Note(folders.ideas()));
    assert_eq!(editor_text(&root, cx), "Ideas\nbody");
}

#[gpui::test]
fn a_file_changed_by_another_program_reloads_or_asks_like_a_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"one");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);

    fs::write(&log, "two").unwrap();
    report(NoteEvent::Changed(log.clone()), &root, cx);
    assert_eq!(editor_text(&root, cx), "two", "no edits: reloaded");

    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" mine");
    fs::write(&log, "theirs").unwrap();
    report(NoteEvent::Changed(log.clone()), &root, cx);
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(notices(&root, cx), [Notice::ChangedOnDisk]);
    assert_eq!(fs::read_to_string(&log).unwrap(), "theirs");
    assert_eq!(editor_text(&root, cx), "two mine");
}

#[gpui::test]
fn a_file_deleted_by_another_program_can_be_kept(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"kept");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);

    fs::remove_file(&log).unwrap();
    report(NoteEvent::Removed(log.clone()), &root, cx);
    assert_eq!(notices(&root, cx), [Notice::DeletedOnDisk]);

    click("choice:Keep File", cx);
    assert_eq!(fs::read_to_string(&log).unwrap(), "kept");
    assert_eq!(listed(&root, cx), [folders.ideas()]);
}

#[gpui::test]
fn closing_a_deleted_file_opens_the_newest_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"gone");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);

    fs::remove_file(&log).unwrap();
    report(NoteEvent::Removed(log.clone()), &root, cx);
    click("choice:Close File", cx);

    assert_eq!(selection(&root, cx), Selection::Note(folders.ideas()));
    assert_eq!(editor_text(&root, cx), "Ideas\nbody");
    assert!(!log.exists());
}

#[gpui::test]
fn picking_a_note_saves_the_file_first(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"start");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" typed");

    click("note:Ideas", cx);

    assert_eq!(fs::read_to_string(&log).unwrap(), "start typed");
    assert_eq!(selection(&root, cx), Selection::Note(folders.ideas()));
    assert_eq!(editor_text(&root, cx), "Ideas\nbody");
    assert!(is_markdown(&root, cx));
}

#[gpui::test]
fn a_file_that_is_not_utf8_opens_read_only(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let legacy = folders.file("legacy.ini", b"name=Caf\xE9");
    let (root, cx) = folders.open(cx);

    open_file(&legacy, cx);

    assert_eq!(notices(&root, cx), [Notice::NotUtf8]);
    assert!(is_read_only(&root, cx));
    cx.simulate_input("x");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(fs::read(&legacy).unwrap(), b"name=Caf\xE9");
}

#[gpui::test]
fn the_last_file_reopens_on_restart_and_a_missing_one_falls_back_to_the_last_note(
    cx: &mut TestAppContext,
) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"remembered");
    let (_root, cx) = folders.open(cx);
    open_file(&log, cx);
    common::close(cx);

    let (root, cx) = folders.open(cx);
    assert_eq!(selection(&root, cx), Selection::File(log.clone()));
    assert_eq!(editor_text(&root, cx), "remembered");
    common::close(cx);

    fs::remove_file(&log).unwrap();
    let (root, cx) = folders.open(cx);
    assert_eq!(selection(&root, cx), Selection::Note(folders.ideas()));
}

#[gpui::test]
fn unsaved_text_of_a_file_is_recovered_after_a_crash(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let log = folders.file("log.txt", b"saved");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);
    cx.simulate_keystrokes("ctrl-end");
    // Too fast for the autosave delay, for longer than the snapshot interval.
    let mut typed_for = Duration::ZERO;
    while typed_for <= SNAPSHOT_INTERVAL {
        cx.simulate_input("x");
        wait(AUTOSAVE_DELAY / 2, cx);
        typed_for += AUTOSAVE_DELAY / 2;
    }
    assert_eq!(fs::read_to_string(&log).unwrap(), "saved");
    common::crash(root, cx);

    let (root, cx) = folders.open(cx);
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "log.txt".into(),
            new_note: false
        }]
    );
    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);

    assert_eq!(selection(&root, cx), Selection::File(log.clone()));
    assert!(fs::read_to_string(&log).unwrap().starts_with("savedxxx"));
    assert_eq!(titles_on_disk(folders.notes.path()), ["Ideas"]);
}

#[gpui::test]
fn recovered_text_of_a_markdown_file_elsewhere_goes_to_a_new_note_and_says_so(
    cx: &mut TestAppContext,
) {
    let folders = Folders::new();
    // An old notes folder's note looks the same, and is restored into this folder (ADR 0081).
    let readme = folders.file("readme.md", b"Readme");
    RecoveryStore::new(folders.data.path().join("recovery"))
        .write(&readme, "Readme\nunsaved")
        .unwrap();
    let (root, cx) = folders.open(cx);

    click("choice:Restore", cx);
    wait(AUTOSAVE_DELAY, cx);

    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Restored \"readme\" as a new note")),
        "{message:?}"
    );
    assert_eq!(titles_on_disk(folders.notes.path()), ["Ideas", "Readme"]);
    assert_eq!(
        fs::read_to_string(folders.notes.path().join("Readme.md")).unwrap(),
        "Readme\nunsaved"
    );
    assert_eq!(fs::read_to_string(&readme).unwrap(), "Readme");
    assert!(matches!(selection(&root, cx), Selection::Note(_)));
}

#[gpui::test]
fn the_file_stays_open_when_the_notes_folder_changes(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let other = tempfile::tempdir().unwrap();
    write_note(other.path(), "Other", "Other", days_ago(0, 10));
    let log = folders.file("log.txt", b"open");
    let (root, cx) = folders.open(cx);
    open_file(&log, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" still");

    let location = common::location(other.path());
    let store = location.open_writable().unwrap();
    let session = common::session(&root, cx);
    let notes = common::notes(&root, cx);
    session.update(cx, |session, cx| {
        session.change_folder(other.path().to_owned(), cx)
    });
    notes.update(cx, |notes, cx| notes.change_folder(location, store, cx));
    cx.run_until_parked();

    assert_eq!(selection(&root, cx), Selection::File(log.clone()));
    assert_eq!(listed(&root, cx), [other.path().join("Other.md")]);
    assert_eq!(fs::read_to_string(&log).unwrap(), "open still");
    cx.simulate_input(" here");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(fs::read_to_string(&log).unwrap(), "open still here");
}

#[gpui::test]
fn a_file_that_becomes_a_note_of_the_new_notes_folder_is_left_like_a_note(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let readme = folders.file("Readme.md", b"Readme");
    write_note(folders.elsewhere.path(), "Newer", "Newer", days_ago(0, 1));
    let (root, cx) = folders.open(cx);
    open_file(&readme, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" typed");

    let location = common::location(folders.elsewhere.path());
    let store = location.open_writable().unwrap();
    let session = common::session(&root, cx);
    let notes = common::notes(&root, cx);
    session.update(cx, |session, cx| {
        session.change_folder(folders.elsewhere.path().to_owned(), cx)
    });
    notes.update(cx, |notes, cx| notes.change_folder(location, store, cx));
    cx.run_until_parked();

    // Listed now, so it is open as a note: the list's rename and delete apply to it, which a
    // file kept open apart from the list would not follow.
    let newest = folders.elsewhere.path().join("Readme.md");
    assert_eq!(selection(&root, cx), Selection::Note(newest.clone()));
    assert_eq!(fs::read_to_string(&newest).unwrap(), "Readme typed");
    cx.simulate_keystrokes("ctrl-home");
    cx.simulate_input("My ");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(
        titles_on_disk(folders.elsewhere.path()),
        ["My Readme typed", "Newer"]
    );
}

#[gpui::test]
fn a_file_that_cannot_be_read_is_reported_and_the_newest_note_opens(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let (root, cx) = folders.open(cx);

    // A folder passes for the file the dialog picked but cannot be read as one.
    open_file(folders.elsewhere.path(), cx);

    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Could not read")),
        "{message:?}"
    );
    assert_eq!(selection(&root, cx), Selection::Note(folders.ideas()));
    assert_eq!(editor_text(&root, cx), "Ideas\nbody");
}

#[gpui::test]
fn the_status_bar_describes_the_open_file(cx: &mut TestAppContext) {
    let folders = Folders::new();
    let legacy = folders.file("legacy.ini", b"[cafe]\r\nname=Caf\xE9\r\n");
    let (root, cx) = folders.open(cx);
    let status = |cx: &mut VisualTestContext| {
        root.read_with(cx, |root, cx| {
            let status_bar = root.editor_pane().read(cx).status_bar().read(cx);
            let status = status_bar.status();
            (status.line_ending, status.encoding)
        })
    };

    open_file(&legacy, cx);
    assert_eq!(status(cx), ("Windows (CRLF)", "Not UTF-8"));
    click("choice:Edit Anyway", cx);
    assert_eq!(
        status(cx),
        ("Windows (CRLF)", "UTF-8"),
        "what it is saved in"
    );

    click("note:Ideas", cx);
    assert_eq!(status(cx), ("Unix (LF)", "UTF-8"));
}
