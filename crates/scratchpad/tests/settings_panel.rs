//! The settings panel (Ctrl+,): theme and notes folder (PLAN §33, §35). See ADRs 0080-0082.

mod common;

use std::fs;
use std::path::Path;

use common::{click, days_ago, editor_text, titles_on_disk, wait, write_note};
use gpui::{
    Entity, Focusable, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, point,
    px,
};
use scratchpad::notes::Selection;
use scratchpad::session::{AUTOSAVE_DELAY, Notice};
use scratchpad::settings::SAVE_DELAY;
use scratchpad::settings_panel::PickFolderForTests;
use scratchpad::theme::{ActiveTheme, Appearance, ThemeMode};
use scratchpad::{AppWindow, Storage, toast};
use scratchpad_core::{Config, RecoveryStore, ThemePreference};

fn is_open(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    root.read_with(cx, |root, _| root.settings_panel().is_some())
}

fn editor_has_focus(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn focus_editor(root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
}

fn theme_mode(cx: &mut VisualTestContext) -> ThemeMode {
    cx.update(|_, cx| cx.theme_mode())
}

/// Presses Enter the way a real keyboard does: GPUI presses a focused button on key-up.
fn press_enter(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("enter");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
}

fn notes_dir(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> std::path::PathBuf {
    let notes = common::notes(root, cx);
    notes.read_with(cx, |notes, _| notes.location().dir.clone())
}

fn picks_folder(dir: &Path, cx: &mut VisualTestContext) {
    cx.update(|_, cx| cx.set_global(PickFolderForTests(Some(dir.to_owned()))));
}

#[gpui::test]
fn ctrl_comma_opens_the_settings_and_closing_them_returns_to_the_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    focus_editor(&root, cx);

    cx.simulate_keystrokes("ctrl-,");
    assert!(is_open(&root, cx));
    assert!(!editor_has_focus(&root, cx), "the panel takes the focus");
    cx.simulate_keystrokes("escape");
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));

    // A click outside the panel closes it too.
    cx.simulate_keystrokes("ctrl-,");
    assert!(is_open(&root, cx));
    cx.simulate_click(point(px(4.), px(4.)), Modifiers::none());
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));

    // So does the close button, and a click inside the panel does not.
    cx.simulate_keystrokes("ctrl-,");
    click("notes-folder-path", cx);
    assert!(is_open(&root, cx));
    click("button:close-settings", cx);
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));
}

#[gpui::test]
fn the_settings_close_when_another_command_takes_the_focus(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    cx.simulate_keystrokes("ctrl-, ctrl-p");

    assert!(!is_open(&root, cx));
    assert!(
        !editor_has_focus(&root, cx),
        "the search field keeps the focus"
    );
}

#[gpui::test]
fn the_settings_stay_open_while_another_window_is_active(cx: &mut TestAppContext) {
    // Such as the folder dialog, whose answer the panel waits for.
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_keystrokes("ctrl-,");

    cx.deactivate_window();
    assert!(is_open(&root, cx));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(is_open(&root, cx));

    // The panel still has the focus.
    cx.simulate_keystrokes("escape");
    assert!(!is_open(&root, cx));
}

#[gpui::test]
fn choosing_a_theme_applies_it_and_remembers_it(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (_root, cx) = common::open_with(storage, cx);
    let appearance = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.theme().appearance);
    // The test platform's OS appearance is light.
    assert_eq!(appearance(cx), Appearance::Light);

    cx.simulate_keystrokes("ctrl-,");
    click("theme:Dark", cx);

    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    assert_eq!(appearance(cx), Appearance::Dark);
    wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).theme, ThemePreference::Dark);

    click("theme:System", cx);
    assert_eq!(appearance(cx), Appearance::Light);
    wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).theme, ThemePreference::System);
}

#[gpui::test]
fn the_settings_work_from_the_keyboard(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_keystrokes("ctrl-,");

    // The theme control has the focus first; the arrows pick a theme, wrapping around.
    cx.simulate_keystrokes("right");
    assert_eq!(theme_mode(cx), ThemeMode::Light);
    cx.simulate_keystrokes("down");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    cx.simulate_keystrokes("right");
    assert_eq!(theme_mode(cx), ThemeMode::System);
    cx.simulate_keystrokes("left");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);

    // Tab leaves the theme control (the arrows do nothing on a button)...
    cx.simulate_keystrokes("tab right");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    // ...and comes back to it after Change…, Open Folder and Close.
    cx.simulate_keystrokes("tab tab tab right");
    assert_eq!(theme_mode(cx), ThemeMode::System);

    // Shift+Tab from the theme control wraps around to Close; Enter presses it.
    cx.simulate_keystrokes("shift-tab");
    assert!(is_open(&root, cx));
    press_enter(cx);
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));
}

/// Opens the app on `old` with the config in a temp folder, which the result keeps alive.
fn open_on<'a>(
    old: &Path,
    cx: &'a mut TestAppContext,
) -> (
    tempfile::TempDir,
    Storage,
    Entity<AppWindow>,
    &'a mut VisualTestContext,
) {
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(old, data_dir.path());
    let (root, cx) = common::open_with(storage.clone(), cx);
    (data_dir, storage, root, cx)
}

#[gpui::test]
fn changing_the_folder_saves_the_open_note_first_and_opens_the_newest_of_the_new_one(
    cx: &mut TestAppContext,
) {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    write_note(old.path(), "Ideas", "Ideas", days_ago(0, 10));
    write_note(new.path(), "Older", "Older", days_ago(2, 10));
    let recipes = write_note(new.path(), "Recipes", "Recipes\nsoup", days_ago(1, 10));
    let (_data, storage, root, cx) = open_on(old.path(), cx);
    let notes = common::notes(&root, cx);
    focus_editor(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    // Neither saved nor renamed yet: the autosave delay has not passed.
    cx.simulate_input(" and plans");

    // A search of the old folder is running.
    notes.update(cx, |notes, cx| notes.set_query("plans", cx));

    picks_folder(new.path(), cx);
    cx.simulate_keystrokes("ctrl-,");
    click("button:change-folder", cx);

    // Leaving the note saved it and renamed it after its new title, in the old folder.
    let ideas = old.path().join("Ideas and plans.md");
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas and plans");
    assert_eq!(notes_dir(&root, cx), new.path());
    let listed: Vec<String> = notes.read_with(cx, |notes, _| {
        notes.notes().iter().map(|n| n.title.clone()).collect()
    });
    assert_eq!(listed, ["Recipes", "Older"]);
    assert!(!notes.read_with(cx, |notes, _| notes.is_searching()));
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(recipes.clone())
    );
    assert_eq!(editor_text(&root, cx), "Recipes\nsoup");
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));
    wait(SAVE_DELAY, cx);
    let config = Config::load(storage.config_path.as_ref().unwrap());
    assert_eq!(config.notes_dir.as_deref(), Some(new.path()));
    assert_eq!(config.last_opened_note, Some(recipes.clone()));

    // Edits now go to the new folder.
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" and bread");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(
        fs::read_to_string(&recipes).unwrap(),
        "Recipes\nsoup and bread"
    );
    assert_eq!(titles_on_disk(old.path()), ["Ideas and plans"]);
    assert_eq!(titles_on_disk(new.path()), ["Older", "Recipes"]);
}

#[gpui::test]
fn a_new_note_is_saved_to_the_old_folder_and_an_empty_folder_starts_a_new_note(
    cx: &mut TestAppContext,
) {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let (_data, _storage, root, cx) = open_on(old.path(), cx);
    let notes = common::notes(&root, cx);
    // The empty folder opened a new note; its title line is not finished, so it has no file.
    cx.simulate_input("Shopping");
    assert!(titles_on_disk(old.path()).is_empty());

    picks_folder(new.path(), cx);
    cx.simulate_keystrokes("ctrl-,");
    click("button:change-folder", cx);

    assert_eq!(titles_on_disk(old.path()), ["Shopping"]);
    assert_eq!(
        fs::read_to_string(old.path().join("Shopping.md")).unwrap(),
        "Shopping"
    );
    assert!(notes.read_with(cx, |notes, _| notes.has_draft()));
    assert_eq!(editor_text(&root, cx), "");
    cx.simulate_input("Ideas\n");
    wait(AUTOSAVE_DELAY, cx);
    assert_eq!(titles_on_disk(new.path()), ["Ideas"]);
    assert_eq!(titles_on_disk(old.path()), ["Shopping"]);
}

#[gpui::test]
fn a_folder_that_cannot_hold_notes_is_refused(cx: &mut TestAppContext) {
    let old = tempfile::tempdir().unwrap();
    write_note(old.path(), "Ideas", "Ideas", days_ago(0, 10));
    let (_data, storage, root, cx) = open_on(old.path(), cx);
    // A folder can't be created inside a file.
    let file = old.path().join("Ideas.md");
    let impossible = file.join("Notes");

    picks_folder(&impossible, cx);
    cx.simulate_keystrokes("ctrl-,");
    click("button:change-folder", cx);

    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Could not")),
        "{message:?}"
    );
    assert_eq!(notes_dir(&root, cx), old.path());
    assert!(is_open(&root, cx));
    assert_eq!(editor_text(&root, cx), "Ideas");
    wait(SAVE_DELAY, cx);
    let config = Config::load(storage.config_path.as_ref().unwrap());
    assert_eq!(config.notes_dir, None);
}

#[gpui::test]
fn a_folder_set_by_the_environment_cannot_be_changed(cx: &mut TestAppContext) {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let storage = Storage {
        notes_dir_overridden: true,
        ..common::storage(old.path(), data.path())
    };
    let (root, cx) = common::open_with(storage, cx);
    picks_folder(new.path(), cx);

    cx.simulate_keystrokes("ctrl-,");
    assert!(cx.debug_bounds("notes-folder-overridden").is_some());
    assert!(cx.debug_bounds("button:change-folder").is_none());
    // The keyboard cannot reach "Change…": Tab goes from the theme to Open Folder, then Close.
    cx.simulate_keystrokes("tab tab");
    press_enter(cx);
    assert!(!is_open(&root, cx), "Enter pressed Close");
    // Nor does clicking it do anything.
    cx.simulate_keystrokes("ctrl-,");
    click("button:change-folder-disabled", cx);

    assert_eq!(notes_dir(&root, cx), old.path());
}

#[gpui::test]
fn unsaved_text_of_a_note_in_the_previous_folder_is_restored_as_a_new_note(
    cx: &mut TestAppContext,
) {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let ideas = write_note(old.path(), "Ideas", "Ideas", days_ago(0, 10));
    // Left by a crash before the folder was changed to `new`.
    let recovery = RecoveryStore::new(data.path().join("recovery"));
    recovery
        .write(
            &ideas,
            "Ideas
unsaved",
        )
        .unwrap();
    let (root, cx) = common::open_with(common::storage(new.path(), data.path()), cx);

    click("choice:Restore", cx);

    // Its title line is finished, so the new note got its file right away.
    let restored = new.path().join("Ideas.md");
    let notes = common::notes(&root, cx);
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(restored.clone())
    );
    assert_eq!(
        fs::read_to_string(&restored).unwrap(),
        "Ideas
unsaved"
    );
    assert_eq!(fs::read_to_string(&ideas).unwrap(), "Ideas");
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

fn change_folder_to(dir: &Path, cx: &mut VisualTestContext) {
    picks_folder(dir, cx);
    cx.simulate_keystrokes("ctrl-,");
    click("button:change-folder", cx);
}

#[gpui::test]
fn edits_that_cannot_be_saved_to_the_old_folder_are_offered_in_the_new_one(
    cx: &mut TestAppContext,
) {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let ideas = write_note(
        old.path(),
        "Ideas",
        "Ideas
body",
        days_ago(0, 10),
    );
    let (_data, _storage, root, cx) = open_on(old.path(), cx);
    focus_editor(&root, cx);
    cx.simulate_keystrokes("ctrl-end");
    cx.simulate_input(" unsaved");
    set_read_only(&ideas, true);

    change_folder_to(new.path(), cx);

    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    assert!(
        message
            .as_deref()
            .is_some_and(|m| m.starts_with("Could not save \"Ideas\"")),
        "{message:?}"
    );
    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Ideas".into(),
            new_note: false
        }]
    );
    click("choice:Restore", cx);
    assert_eq!(
        fs::read_to_string(new.path().join("Ideas.md")).unwrap(),
        "Ideas
body unsaved"
    );
    assert_eq!(
        fs::read_to_string(&ideas).unwrap(),
        "Ideas
body"
    );
    set_read_only(&ideas, false);
}

#[gpui::test]
fn a_new_note_that_cannot_get_a_file_in_the_old_folder_is_offered_in_the_new_one(
    cx: &mut TestAppContext,
) {
    let base = tempfile::tempdir().unwrap();
    let old = base.path().join("Notes");
    let new = tempfile::tempdir().unwrap();
    let (_data, _storage, root, cx) = open_on(&old, cx);
    cx.simulate_input(
        "Shopping
milk",
    );
    // The old folder is gone (e.g. a drive was removed) and its name taken by a file.
    fs::remove_dir(&old).unwrap();
    fs::write(&old, "").unwrap();

    change_folder_to(new.path(), cx);

    assert_eq!(
        notices(&root, cx),
        [Notice::Recovered {
            title: "Shopping".into(),
            new_note: true
        }]
    );
    click("choice:Restore", cx);
    assert_eq!(
        fs::read_to_string(new.path().join("Shopping.md")).unwrap(),
        "Shopping
milk"
    );
}
