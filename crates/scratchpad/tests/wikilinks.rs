//! Wikilinks to pages in the note's folder: Ctrl+click opens the page, or creates it first, and the
//! View > Markdown features toggle (ADR 0188).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{days_ago, editor_text, write_note};
use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, TestAppContext, VisualTestContext, point, px, size,
};
use scratchpad::AppWindow;
use scratchpad::actions::view::ToggleWikilinks;
use scratchpad::editor_view::EditorView;
use scratchpad::notes::Selection;
use scratchpad::settings::SAVE_DELAY;
use scratchpad_core::Config;

/// The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size.
const ADVANCE: f32 = 9.;

/// Opens the app on `dir`, whose newest note is shown, in a window wide enough for one-row lines.
fn open<'a>(
    dir: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
    let (root, cx) = common::open_main_window_in(dir, cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    cx.run_until_parked();
    (root, cx)
}

/// Screen bounds of the start of the first line, as the IME sees them.
fn line_start(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    editor.update_in(cx, |editor, window, cx| {
        editor
            .bounds_for_range(0..0, Bounds::default(), window, cx)
            .expect("the first line is visible")
    })
}

/// Clicks the first line at displayed `column` with `modifiers`, the cursor elsewhere so that
/// live preview hides the line's markers.
fn click_first_line(
    root: &Entity<AppWindow>,
    column: f32,
    modifiers: Modifiers,
    cx: &mut VisualTestContext,
) {
    let editor = common::editor(root, cx);
    cx.simulate_keystrokes("ctrl-end");
    let line = line_start(&editor, cx);
    let position = point(line.left() + px(column * ADVANCE + 4.), line.center().y);
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();
}

fn ctrl_click(root: &Entity<AppWindow>, column: f32, cx: &mut VisualTestContext) {
    click_first_line(root, column, Modifiers::secondary_key(), cx);
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

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[gpui::test]
fn ctrl_click_opens_an_existing_page_ignoring_case(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let target = write_note(dir.path(), "Project Plan", "# Plan\nsteps", days_ago(2, 9));
    write_note(
        dir.path(),
        "Home",
        "[[project plan#Steps|the plan]] x\nnext",
        days_ago(0, 9),
    );
    let (root, cx) = open(dir.path(), cx);

    // A plain click edits.
    click_first_line(&root, 2., Modifiers::none(), cx);
    assert_eq!(
        selection(&root, cx),
        Selection::Note(dir.path().join("Home.md"))
    );

    // Displayed "the plan x".
    ctrl_click(&root, 2., cx);
    assert_eq!(selection(&root, cx), Selection::Note(target));
    assert_eq!(editor_text(&root, cx), "# Plan\nsteps");
    assert_eq!(names_in(dir.path()), ["Home.md", "Project Plan.md"]);
}

#[gpui::test]
fn markdown_pages_come_before_text_files_which_open_as_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("both.txt"), "text both").unwrap();
    write_note(dir.path(), "Both", "markdown both", days_ago(3, 9));
    fs::write(dir.path().join("Log.TXT"), "the log").unwrap();
    write_note(dir.path(), "Home", "[[both]] [[log]]\nnext", days_ago(0, 9));
    let (root, cx) = open(dir.path(), cx);

    // Displayed "both log".
    ctrl_click(&root, 1., cx);
    assert_eq!(
        selection(&root, cx),
        Selection::Note(dir.path().join("Both.md"))
    );
    assert_eq!(editor_text(&root, cx), "markdown both");

    let home = dir.path().join("Home.md");
    common::notes(&root, cx).update(cx, |notes, cx| notes.select(&home, cx));
    cx.run_until_parked();
    ctrl_click(&root, 6., cx);
    assert_eq!(
        selection(&root, cx),
        Selection::File(dir.path().join("Log.TXT"))
    );
    assert_eq!(editor_text(&root, cx), "the log");
    let editor = common::editor(&root, cx);
    assert!(!editor.read_with(cx, |editor, _| editor.is_markdown()));
}

#[gpui::test]
fn ctrl_click_creates_a_missing_page_in_the_folder_and_opens_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(
        dir.path(),
        "Home",
        "see [[New Idea]] x\nnext",
        days_ago(0, 9),
    );
    let (root, cx) = open(dir.path(), cx);
    let editor = common::editor(&root, cx);
    let page_known = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor
                .pages()
                .map(|pages| pages.resolve("new idea").is_some())
        })
    };
    assert_eq!(
        page_known(cx),
        Some(false),
        "the editor knows it is missing"
    );

    // Displayed "see New Idea x".
    ctrl_click(&root, 6., cx);
    let created = dir.path().join("New Idea.md");
    assert_eq!(selection(&root, cx), Selection::Note(created.clone()));
    assert_eq!(fs::read_to_string(&created).unwrap(), "# New Idea\n");
    assert_eq!(editor_text(&root, cx), "# New Idea\n");
    assert!(listed(&root, cx).contains(&created));
    assert_eq!(page_known(cx), Some(true));
    assert_eq!(names_in(dir.path()), ["Home.md", "New Idea.md"]);

    // Back to the link: it opens the page now rather than creating another.
    let home = dir.path().join("Home.md");
    common::notes(&root, cx).update(cx, |notes, cx| notes.select(&home, cx));
    cx.run_until_parked();
    ctrl_click(&root, 6., cx);
    assert_eq!(selection(&root, cx), Selection::Note(created));
    assert_eq!(names_in(dir.path()), ["Home.md", "New Idea.md"]);
}

#[gpui::test]
fn a_page_created_meanwhile_by_another_program_is_opened_not_duplicated(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Home", "[[Later]]\nnext", days_ago(0, 9));
    let (root, cx) = open(dir.path(), cx);
    // Not listed yet: there is no watcher in tests.
    fs::write(dir.path().join("later.txt"), "from elsewhere").unwrap();
    ctrl_click(&root, 1., cx);
    assert_eq!(
        selection(&root, cx),
        Selection::File(dir.path().join("later.txt"))
    );
    assert_eq!(names_in(dir.path()), ["Home.md", "later.txt"]);
}

#[gpui::test]
fn targets_that_are_not_plain_page_names_are_not_links(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let escape = format!(
        "[[../{0}]] [[CON]] [[a/b]]\nnext",
        outside.path().file_name().unwrap().to_string_lossy()
    );
    write_note(dir.path(), "Home", &escape, days_ago(0, 9));
    let (root, cx) = open(dir.path(), cx);
    let home = Selection::Note(dir.path().join("Home.md"));
    for column in [3., 13., 20.] {
        ctrl_click(&root, column, cx);
        assert_eq!(selection(&root, cx), home);
    }
    assert_eq!(names_in(dir.path()), ["Home.md"]);
    assert!(names_in(outside.path()).is_empty());
}

#[gpui::test]
fn a_file_from_another_folder_links_to_pages_in_its_own_folder(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Sibling", "the notes folder's", days_ago(1, 9));
    write_note(dir.path(), "Home", "home", days_ago(0, 9));
    let file = elsewhere.path().join("readme.md");
    fs::write(&file, "[[Sibling]] [[Other]]\nnext").unwrap();
    let (root, cx) = open(dir.path(), cx);
    common::notes(&root, cx).update(cx, |notes, cx| notes.open_file(&file, cx));
    cx.run_until_parked();
    assert_eq!(selection(&root, cx), Selection::File(file.clone()));

    // Displayed "Sibling Other": not the notes folder's Sibling, but a new page next to the file.
    ctrl_click(&root, 1., cx);
    let sibling = elsewhere.path().join("Sibling.md");
    assert_eq!(selection(&root, cx), Selection::File(sibling.clone()));
    assert_eq!(fs::read_to_string(&sibling).unwrap(), "# Sibling\n");
    assert!(!listed(&root, cx).contains(&sibling));
    assert_eq!(names_in(dir.path()), ["Home.md", "Sibling.md"]);

    fs::write(elsewhere.path().join("Other.txt"), "other").unwrap();
    common::notes(&root, cx).update(cx, |notes, cx| notes.open_file(&file, cx));
    cx.run_until_parked();
    let editor = common::editor(&root, cx);
    let known = editor.read_with(cx, |editor, _| {
        editor
            .pages()
            .and_then(|p| p.resolve("other").map(Path::to_owned))
    });
    assert_eq!(known, Some(elsewhere.path().join("Other.txt")));
    ctrl_click(&root, 10., cx);
    assert_eq!(
        selection(&root, cx),
        Selection::File(elsewhere.path().join("Other.txt"))
    );
}

#[gpui::test]
fn the_wikilinks_feature_is_ticked_toggles_and_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    write_note(notes_dir.path(), "Home", "[[Gone]]\nnext", days_ago(0, 9));
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (root, cx) = common::open_with(storage.clone(), cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    cx.run_until_parked();
    let editor = common::editor(&root, cx);
    assert!(editor.read_with(cx, |e, _| e.markdown_options().wikilinks));

    cx.dispatch_action(ToggleWikilinks);
    assert!(!editor.read_with(cx, |e, _| e.markdown_options().wikilinks));
    // Off, the text is not a link: Ctrl+click creates nothing.
    ctrl_click(&root, 3., cx);
    assert_eq!(names_in(notes_dir.path()), ["Home.md"]);
    common::wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).markdown_wikilinks, Some(false));
    common::close(cx);

    let (root, cx) = common::open_with(storage, cx);
    let editor = common::editor(&root, cx);
    assert!(!editor.read_with(cx, |e, _| e.markdown_options().wikilinks));
    cx.dispatch_action(ToggleWikilinks);
    assert!(editor.read_with(cx, |e, _| e.markdown_options().wikilinks));
    common::close(cx);
    assert_eq!(Config::load(&config_path).markdown_wikilinks, Some(true));
}
