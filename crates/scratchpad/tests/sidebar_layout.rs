//! Hiding the sidebar, and the sidebar in narrow windows (ADR 0135).

mod common;

use common::{answer_prompt, click, days_ago, right_click, wait, write_note};
use gpui::{Entity, Focusable, Pixels, TestAppContext, VisualTestContext, point, px, size};
use scratchpad::AppWindow;
use scratchpad::app_window::{AUTO_COLLAPSE_WIDTH, SidebarMode};
use scratchpad::notes::Selection;
use scratchpad::settings::SAVE_DELAY;
use scratchpad::sidebar::{DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH};
use scratchpad_core::Config;

fn mode(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> SidebarMode {
    root.read_with(cx, |root, cx| root.sidebar_mode(cx))
}

/// Where the note's pane starts: right of a docked sidebar, else at the window's edge.
fn note_left(cx: &mut VisualTestContext) -> Pixels {
    cx.debug_bounds("editor-pane").unwrap().left()
}

fn note_width(cx: &mut VisualTestContext) -> Pixels {
    cx.debug_bounds("editor-pane").unwrap().size.width
}

fn resize(width: Pixels, cx: &mut VisualTestContext) {
    cx.simulate_resize(size(width, px(700.)));
    cx.run_until_parked();
}

fn editor_focused(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = root.read_with(cx, |root, cx| root.editor_pane().focus_handle(cx));
    cx.update(|window, _| editor.is_focused(window))
}

#[gpui::test]
fn ctrl_backslash_collapses_and_expands_the_sidebar_and_it_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (root, cx) = common::open_with(storage.clone(), cx);
    let width = root.read_with(cx, |root, cx| root.sidebar().read(cx).width());
    assert_eq!(mode(&root, cx), SidebarMode::Docked);
    assert_eq!(note_left(cx), width);

    cx.simulate_keystrokes("ctrl-\\");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert_eq!(note_left(cx), px(0.));
    // The caret stays in the note.
    assert!(editor_focused(&root, cx));
    wait(SAVE_DELAY, cx);
    assert!(Config::load(&config_path).sidebar_collapsed);

    // The button at the note's top left brings it back, as wide as it was. The click is the
    // button's alone: the caret stays where it was.
    cx.simulate_input("one\ntwo");
    click("show-sidebar", cx);
    assert_eq!(mode(&root, cx), SidebarMode::Docked);
    assert_eq!(note_left(cx), width);
    let editor = common::editor(&root, cx);
    let caret = editor.read_with(cx, |editor, _| editor.editor().selection().head.0);
    assert_eq!(caret, "one\ntwo".len());
    cx.simulate_keystrokes("ctrl-\\");
    common::close(cx);

    let (root, cx) = common::open_with(storage, cx);
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert_eq!(note_left(cx), px(0.));
    // The header button hides it too.
    cx.simulate_keystrokes("ctrl-\\");
    click("sidebar-toggle", cx);
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
}

#[gpui::test]
fn hiding_the_sidebar_while_it_has_focus_puts_the_caret_in_the_note(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_keystrokes("ctrl-p");
    assert!(!editor_focused(&root, cx));

    cx.simulate_keystrokes("ctrl-\\");
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn a_narrow_window_hides_the_sidebar_and_shows_it_over_the_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    write_note(dir.path(), "Meeting", "", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    assert_eq!(mode(&root, cx), SidebarMode::Docked);

    resize(AUTO_COLLAPSE_WIDTH - px(1.), cx);
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert_eq!(note_left(cx), px(0.));
    let full_width = note_width(cx);

    // Asked for, it floats over the note, which keeps its width, and takes the keyboard.
    cx.simulate_keystrokes("ctrl-\\");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    assert_eq!((note_left(cx), note_width(cx)), (px(0.), full_width));
    assert!(!editor_focused(&root, cx));
    // Browsing with the arrow keys keeps it open.
    cx.simulate_keystrokes("down");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);

    // Escape closes it and puts the caret back in the note.
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert!(editor_focused(&root, cx));

    // A click outside it only closes it.
    click("show-sidebar", cx);
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    let outside = point(full_width - px(40.), px(300.));
    cx.simulate_click(outside, gpui::Modifiers::none());
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);

    // Choosing a note opens it and closes the sidebar.
    cx.simulate_keystrokes("ctrl-\\");
    click("note:Ideas", cx);
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert!(editor_focused(&root, cx));
    let notes = common::notes(&root, cx);
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(dir.path().join("Ideas.md"))
    );

    // Widening the window docks it again; it was never collapsed by the user.
    cx.simulate_keystrokes("ctrl-\\");
    resize(AUTO_COLLAPSE_WIDTH, cx);
    assert_eq!(mode(&root, cx), SidebarMode::Docked);
    assert!(note_left(cx) > px(0.));
    // Narrowed again, it is hidden: the overlay was for that one look.
    resize(AUTO_COLLAPSE_WIDTH - px(1.), cx);
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
}

#[gpui::test]
fn a_wide_sidebar_narrows_so_the_note_keeps_its_room(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let sidebar = root.read_with(cx, |root, _| root.sidebar().clone());
    sidebar.update(cx, |sidebar, cx| sidebar.set_width(MAX_SIDEBAR_WIDTH, cx));

    // Just wide enough to dock it: the note keeps what a default sidebar would leave it.
    resize(AUTO_COLLAPSE_WIDTH, cx);
    assert_eq!(mode(&root, cx), SidebarMode::Docked);
    assert_eq!(note_width(cx), AUTO_COLLAPSE_WIDTH - DEFAULT_SIDEBAR_WIDTH);
    assert_eq!(note_left(cx), DEFAULT_SIDEBAR_WIDTH);
    // So the find bar's buttons stay in the window.
    cx.simulate_keystrokes("ctrl-h");
    let replace_all = cx.debug_bounds("replace-all").unwrap();
    assert!(
        replace_all.right() <= AUTO_COLLAPSE_WIDTH,
        "{replace_all:?}"
    );

    // The width chosen is kept for a wider window.
    resize(px(1100.), cx);
    assert_eq!(note_left(cx), MAX_SIDEBAR_WIDTH);
}

#[gpui::test]
fn widening_keeps_a_sidebar_the_user_collapsed_hidden(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_keystrokes("ctrl-\\");
    resize(px(600.), cx);
    // In a narrow window Ctrl+\ only shows it for a while; the setting is unchanged.
    cx.simulate_keystrokes("ctrl-\\");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);

    resize(px(1000.), cx);
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
}

#[gpui::test]
fn search_shortcuts_show_a_hidden_sidebar_and_focus_the_search(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let search_focused = |cx: &mut VisualTestContext| {
        let sidebar = root.read_with(cx, |root, _| root.sidebar().clone());
        cx.update(|window, cx| sidebar.read(cx).contains_focus(window, cx))
    };
    resize(px(600.), cx);

    cx.simulate_keystrokes("ctrl-shift-f");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    assert!(search_focused(cx));
    cx.simulate_input("x");
    assert_eq!(
        root.read_with(cx, |root, cx| root.sidebar().read(cx).search_text(cx)),
        "x"
    );
    // Escape in the search returns to the note, and the sidebar goes with it.
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert!(editor_focused(&root, cx));

    // Collapsed by the user in a wide window, search shows it over the note and leaves the
    // setting alone.
    resize(px(1000.), cx);
    cx.simulate_keystrokes("ctrl-\\");
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    assert_eq!(note_left(cx), px(0.));
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
}

#[gpui::test]
fn the_sidebar_over_the_note_stays_for_its_menu_renames_and_questions(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    resize(px(600.), cx);
    cx.simulate_keystrokes("ctrl-\\");

    // Escape closes the innermost thing first: the menu, then the title being edited.
    right_click("note:Ideas", cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);
    right_click("note:Ideas", cx);
    click("menu:Rename", cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);

    // The delete confirmation is another window: answering it leaves the sidebar up.
    right_click("note:Ideas", cx);
    click("menu:Delete", cx);
    answer_prompt("Cancel", cx);
    assert_eq!(mode(&root, cx), SidebarMode::Overlay);

    cx.simulate_keystrokes("escape");
    assert_eq!(mode(&root, cx), SidebarMode::Hidden);
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn a_context_menu_closes_with_the_sidebar(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 10));
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    right_click("note:Ideas", cx);
    cx.simulate_keystrokes("ctrl-\\ ctrl-\\");
    // The menu did not come back with the sidebar: the click lands on the row underneath.
    click("menu:Delete", cx);
    assert_eq!(cx.pending_prompt(), None);
}
