//! The File, Edit and View menus under the title bar (ADR 0150).

mod common;

use common::{click, days_ago, editor_text, write_note};
use gpui::{
    ClipboardItem, Entity, Focusable, KeyBinding, Modifiers, TestAppContext, VisualTestContext, px,
    size,
};
use scratchpad::AppWindow;
use scratchpad::actions::view::ToggleWordWrap;
use scratchpad::app_window::{AUTO_COLLAPSE_WIDTH, SidebarMode};
use scratchpad::editor_view::SEARCH_DEBOUNCE;
use scratchpad::file_dialogs::PickFileForTests;
use scratchpad::find_bar::ReplaceInNote;
use scratchpad::menu_bar::{MenuBar, ShownItem};
use scratchpad_core::Config;

fn menu_bar(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Entity<MenuBar> {
    root.read_with(cx, |root, _| root.menu_bar().clone())
}

fn open_menu(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Option<&'static str> {
    menu_bar(root, cx).read_with(cx, |bar, _| bar.open_menu())
}

/// The items of the open menu (or of its open submenu).
fn items(root: &Entity<AppWindow>, submenu: bool, cx: &mut VisualTestContext) -> Vec<ShownItem> {
    let bar = menu_bar(root, cx);
    cx.update(|window, cx| bar.read(cx).shown_items(submenu, window, cx))
}

fn shown(root: &Entity<AppWindow>, label: &str, cx: &mut VisualTestContext) -> ShownItem {
    let mut all = items(root, false, cx);
    all.extend(items(root, true, cx));
    all.into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{label} is not shown"))
}

/// `(label, shortcut)` of each item of the open menu.
fn labels(root: &Entity<AppWindow>, submenu: bool, cx: &mut VisualTestContext) -> Vec<String> {
    items(root, submenu, cx)
        .into_iter()
        .map(|item| match item.shortcut {
            Some(keys) => format!("{} {keys}", item.label),
            None => item.label.to_owned(),
        })
        .collect()
}

/// Chooses `item` in `menu` with the mouse.
fn choose(menu: &str, item: &str, cx: &mut VisualTestContext) {
    click(&format!("menu:{menu}"), cx);
    click(&format!("menu-item:{item}"), cx);
}

fn editor_focused(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn hover(selector: &str, cx: &mut VisualTestContext) {
    let position = common::center_of(selector, cx);
    cx.simulate_mouse_move(position, None, Modifiers::none());
}

#[gpui::test]
fn each_menu_lists_its_commands_with_the_keys_from_the_keymap(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    click("menu:File", cx);
    assert_eq!(open_menu(&root, cx), Some("File"));
    assert_eq!(
        labels(&root, false, cx),
        [
            "New note Ctrl+N",
            "Open\u{2026} Ctrl+O",
            "Save Ctrl+S",
            "Save as\u{2026} Ctrl+Shift+S",
            "Settings Ctrl+,",
            "Exit Ctrl+Q",
        ]
    );

    click("menu:Edit", cx);
    assert_eq!(open_menu(&root, cx), Some("Edit"));
    assert_eq!(
        labels(&root, false, cx),
        [
            "Undo Ctrl+Z",
            // Of Ctrl+Shift+Z and Ctrl+Y, Notepad's; of Ctrl+C and Ctrl+Insert, the usual one.
            "Redo Ctrl+Y",
            "Cut Ctrl+X",
            "Copy Ctrl+C",
            "Paste Ctrl+V",
            "Delete Del",
            "Find Ctrl+F",
            "Find next F3",
            "Find previous Shift+F3",
            "Replace Ctrl+H",
            "Go to Ctrl+G",
            "Select all Ctrl+A",
        ]
    );

    click("menu:View", cx);
    assert_eq!(
        labels(&root, false, cx),
        [
            "Zoom",
            "Status bar",
            "Word wrap",
            "Source mode Ctrl+/",
            "Sidebar Ctrl+\\"
        ]
    );
    hover("menu-item:Zoom", cx);
    assert_eq!(
        labels(&root, true, cx),
        [
            "Zoom in Ctrl+Plus",
            "Zoom out Ctrl+Minus",
            "Restore default zoom Ctrl+0"
        ]
    );
}

#[gpui::test]
fn the_keys_shown_follow_the_keymap(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.update(|_, cx| {
        cx.clear_key_bindings();
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-h", ReplaceInNote, None),
            KeyBinding::new("alt-z", ToggleWordWrap, None),
        ]);
    });
    cx.run_until_parked();

    click("menu:Edit", cx);
    assert_eq!(
        shown(&root, "Replace", cx).shortcut.as_deref(),
        Some("Ctrl+Shift+H")
    );
    assert_eq!(shown(&root, "Undo", cx).shortcut, None);
    click("menu:View", cx);
    assert_eq!(
        shown(&root, "Word wrap", cx).shortcut.as_deref(),
        Some("Alt+Z")
    );
}

#[gpui::test]
fn undo_from_the_menu_undoes_in_the_note_and_leaves_the_caret_there(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_input("hello");

    click("menu:Edit", cx);
    assert!(!editor_focused(&root, cx), "the open menu has the keys");
    click("menu-item:Undo", cx);
    assert_eq!(editor_text(&root, cx), "");
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));

    choose("Edit", "Redo", cx);
    assert_eq!(editor_text(&root, cx), "hello");
    // Typing goes on in the note.
    cx.simulate_input("!");
    assert_eq!(editor_text(&root, cx), "hello!");
}

#[gpui::test]
fn cut_copy_and_paste_from_the_menu_use_the_clipboard(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_input("abc");
    let clipboard = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
    };

    choose("Edit", "Select all", cx);
    choose("Edit", "Copy", cx);
    assert_eq!(clipboard(cx).as_deref(), Some("abc"));
    cx.simulate_keystrokes("ctrl-end");
    choose("Edit", "Paste", cx);
    assert_eq!(editor_text(&root, cx), "abcabc");

    cx.simulate_keystrokes("shift-home");
    choose("Edit", "Cut", cx);
    assert_eq!(editor_text(&root, cx), "");
    assert_eq!(clipboard(cx).as_deref(), Some("abcabc"));

    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("xyz".into())));
    choose("Edit", "Paste", cx);
    assert_eq!(editor_text(&root, cx), "xyz");
    cx.simulate_keystrokes("shift-left");
    choose("Edit", "Delete", cx);
    assert_eq!(editor_text(&root, cx), "xy");
}

#[gpui::test]
fn commands_that_cannot_apply_are_disabled_and_do_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Note", "abc", days_ago(0, 10));
    let legacy = dir.path().join("Legacy.md");
    std::fs::write(&legacy, b"caf\xe9").unwrap();
    let file = std::fs::File::options().write(true).open(&legacy).unwrap();
    file.set_modified(days_ago(1, 10)).unwrap();
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let enabled = |root: &Entity<AppWindow>, cx: &mut VisualTestContext| -> Vec<&'static str> {
        items(root, false, cx)
            .into_iter()
            .filter(|item| item.enabled)
            .map(|item| item.label)
            .collect()
    };

    // Nothing to undo and nothing selected; the caret is before "abc".
    click("menu:Edit", cx);
    assert_eq!(
        enabled(&root, cx),
        [
            "Paste",
            "Find",
            "Find next",
            "Find previous",
            "Replace",
            "Go to",
            "Select all"
        ]
    );
    // Delete would delete the "a" after the caret if it ran.
    click("menu-item:Delete", cx);
    assert_eq!(editor_text(&root, cx), "abc");
    assert_eq!(
        open_menu(&root, cx),
        Some("Edit"),
        "a disabled item keeps the menu"
    );
    cx.simulate_keystrokes("escape");
    // The arrow keys pass over them: from Paste, the first, Down goes to Find.
    cx.simulate_keystrokes("alt-e down enter");
    let find_open = root.read_with(cx, |root, cx| {
        let pane = root.editor_pane().read(cx);
        pane.find_bar().read(cx).is_open()
    });
    assert!(find_open);
    cx.simulate_keystrokes("escape");
    click("note-area", cx);
    cx.simulate_keystrokes("ctrl-home");

    cx.simulate_keystrokes("shift-end");
    cx.simulate_input("x");
    cx.simulate_keystrokes("shift-left");
    click("menu:Edit", cx);
    assert_eq!(
        &enabled(&root, cx)[..5],
        ["Undo", "Cut", "Copy", "Paste", "Delete"]
    );
    cx.simulate_keystrokes("escape");

    // A note shown read-only can be copied from but not changed.
    click("note:Legacy", cx);
    click("note-area", cx);
    choose("Edit", "Select all", cx);
    click("menu:Edit", cx);
    assert!(shown(&root, "Copy", cx).enabled);
    assert!(!shown(&root, "Cut", cx).enabled);
    assert!(!shown(&root, "Paste", cx).enabled);
}

#[gpui::test]
fn edit_commands_act_on_the_note_wherever_focus_was(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let find_bar = root.read_with(cx, |root, cx| {
        root.editor_pane().read(cx).find_bar().clone()
    });
    cx.simulate_input("one two one");

    // After Replace all, from the replacement field.
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("one");
    common::wait(SEARCH_DEBOUNCE, cx);
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("1");
    cx.simulate_keystrokes("alt-a");
    assert_eq!(editor_text(&root, cx), "1 two 1");
    choose("Edit", "Undo", cx);
    assert_eq!(editor_text(&root, cx), "one two one");
    assert!(
        editor_focused(&root, cx),
        "the caret is where the undo happened"
    );
    assert_eq!(
        find_bar.read_with(cx, |bar, cx| bar.replacement(cx)),
        "1",
        "the field is untouched"
    );

    // From the note search.
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("!".into())));
    cx.simulate_keystrokes("ctrl-end ctrl-p");
    choose("Edit", "Paste", cx);
    assert_eq!(editor_text(&root, cx), "one two one!");
    let search = root.read_with(cx, |root, cx| root.sidebar().read(cx).search_text(cx));
    assert_eq!(search, "");
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn a_menu_opened_over_the_narrow_sidebar_gives_the_note_the_keys_and_commands(
    cx: &mut TestAppContext,
) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(AUTO_COLLAPSE_WIDTH - px(1.), px(700.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-\\");
    let mode = root.read_with(cx, |root, cx| root.sidebar_mode(cx));
    assert_eq!(mode, SidebarMode::Overlay);

    // The sidebar closes as the menu takes focus, so focus cannot go back there.
    click("menu:View", cx);
    cx.simulate_keystrokes("escape");
    cx.simulate_input("typed");
    assert_eq!(editor_text(&root, cx), "typed");

    // Nor do the commands chosen there: they run as from the note.
    let editor = common::editor(&root, cx);
    cx.simulate_keystrokes("ctrl-\\ alt-v down down enter");
    assert!(!editor.read_with(cx, |editor, _| editor.soft_wrap()));
    cx.simulate_keystrokes("ctrl-\\");
    click("menu:View", cx);
    hover("menu-item:Zoom", cx);
    click("menu-item:Zoom in", cx);
    assert_eq!(editor.read_with(cx, |editor, _| editor.zoom_percent()), 110);
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn word_wrap_and_sidebar_are_ticked_while_on(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let editor = common::editor(&root, cx);

    click("menu:View", cx);
    assert_eq!(shown(&root, "Word wrap", cx).checked, Some(true));
    assert_eq!(shown(&root, "Sidebar", cx).checked, Some(true));
    click("menu-item:Word wrap", cx);
    assert!(!editor.read_with(cx, |editor, _| editor.soft_wrap()));
    click("menu:View", cx);
    assert_eq!(shown(&root, "Word wrap", cx).checked, Some(false));

    click("menu-item:Sidebar", cx);
    let mode = root.read_with(cx, |root, cx| root.sidebar_mode(cx));
    assert_eq!(mode, SidebarMode::Hidden);
    click("menu:View", cx);
    assert_eq!(shown(&root, "Sidebar", cx).checked, Some(false));
    assert_eq!(shown(&root, "Word wrap", cx).checked, Some(false));
}

#[gpui::test]
fn hiding_the_status_bar_from_the_menu_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (root, cx) = common::open_with(storage.clone(), cx);
    let window_bottom =
        |cx: &mut VisualTestContext| cx.update(|window, _| window.viewport_size().height);

    click("menu:View", cx);
    assert_eq!(shown(&root, "Status bar", cx).checked, Some(true));
    click("menu-item:Status bar", cx);
    assert_eq!(
        cx.debug_bounds("note-area").unwrap().bottom(),
        window_bottom(cx)
    );
    common::close(cx);
    assert!(Config::load(&config_path).status_bar_hidden);

    let (root, cx) = common::open_with(storage, cx);
    click("menu:View", cx);
    assert_eq!(shown(&root, "Status bar", cx).checked, Some(false));
}

#[gpui::test]
fn zoom_from_the_menu_shows_in_the_status_bar(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let zoom = |cx: &mut VisualTestContext| {
        root.read_with(cx, |root, cx| {
            let pane = root.editor_pane().read(cx);
            pane.status_bar().read(cx).status().zoom.clone()
        })
    };

    click("menu:View", cx);
    hover("menu-item:Zoom", cx);
    click("menu-item:Zoom in", cx);
    assert_eq!(zoom(cx), "110%");
    assert_eq!(open_menu(&root, cx), None);

    click("menu:View", cx);
    hover("menu-item:Zoom", cx);
    click("menu-item:Restore default zoom", cx);
    assert_eq!(zoom(cx), "100%");
}

#[gpui::test]
fn escape_and_clicks_elsewhere_close_the_menu_and_return_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Note", "abc", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let editor = common::editor(&root, cx);
    let cursor = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.editor().selection().head.0)
    };

    click("menu:File", cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));

    // Into the Zoom submenu and out again: Escape leaves the submenu first.
    cx.simulate_keystrokes("alt-v right");
    assert_eq!(items(&root, true, cx).len(), 3);
    cx.simulate_keystrokes("escape");
    assert!(items(&root, true, cx).is_empty());
    assert_eq!(open_menu(&root, cx), Some("View"));
    cx.simulate_keystrokes("escape");
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));

    // A click in the note only closes the menu: the caret stays.
    cx.simulate_keystrokes("ctrl-end");
    click("menu:Edit", cx);
    click("note-area", cx);
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));
    assert_eq!(cursor(cx), 3);

    // So does a click on its title or on the empty part of the bar.
    click("menu:Edit", cx);
    click("menu:Edit", cx);
    assert_eq!(open_menu(&root, cx), None);
    click("menu:Edit", cx);
    click("menu-bar", cx);
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));

    // Go to line closes when the menu takes focus; focus then goes back to the note.
    cx.simulate_keystrokes("ctrl-g");
    click("menu:File", cx);
    let go_to_line = root.read_with(cx, |root, cx| {
        root.editor_pane().read(cx).go_to_line_box().is_some()
    });
    assert!(!go_to_line);
    cx.simulate_keystrokes("escape");
    assert!(editor_focused(&root, cx));
}

#[gpui::test]
fn focus_goes_back_to_the_note_list_too(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "First", "one", days_ago(0, 10));
    write_note(dir.path(), "Second", "two", days_ago(0, 9));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    click("note:First", cx);
    click("menu:View", cx);
    cx.simulate_keystrokes("escape");
    // Down in the list opens the next note.
    cx.simulate_keystrokes("down");
    assert_eq!(editor_text(&root, cx), "two");
    assert!(!editor_focused(&root, cx));
}

#[gpui::test]
fn moving_to_another_title_switches_menus(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    // Not before a menu is open.
    hover("menu:Edit", cx);
    assert_eq!(open_menu(&root, cx), None);
    hover("menu:File", cx);
    click("menu:File", cx);
    hover("menu:Edit", cx);
    assert_eq!(open_menu(&root, cx), Some("Edit"));
    hover("menu:View", cx);
    assert_eq!(open_menu(&root, cx), Some("View"));
}

#[gpui::test]
fn the_menus_work_from_the_keyboard(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let zoom = common::editor(&root, cx);
    let zoom = move |cx: &mut VisualTestContext| zoom.read_with(cx, |e, _| e.zoom_percent());

    cx.simulate_keystrokes("alt-f");
    assert_eq!(open_menu(&root, cx), Some("File"));
    cx.simulate_keystrokes("right");
    assert_eq!(open_menu(&root, cx), Some("Edit"));
    cx.simulate_keystrokes("left left");
    assert_eq!(open_menu(&root, cx), Some("View"));
    // Zoom is highlighted: Right enters it with Zoom in highlighted.
    cx.simulate_keystrokes("right enter");
    assert_eq!(zoom(cx), 110);
    assert_eq!(open_menu(&root, cx), None);
    assert!(editor_focused(&root, cx));

    cx.simulate_keystrokes("alt-v down down enter");
    let editor = common::editor(&root, cx);
    assert!(!editor.read_with(cx, |editor, _| editor.soft_wrap()));
    cx.simulate_keystrokes("f10");
    assert_eq!(open_menu(&root, cx), Some("File"));
    cx.simulate_keystrokes("alt-e");
    assert_eq!(open_menu(&root, cx), Some("Edit"));
    // Up wraps around to the last item, Sidebar; Space runs it like Enter.
    cx.simulate_keystrokes("escape alt-v up space");
    let mode = root.read_with(cx, |root, cx| root.sidebar_mode(cx));
    assert_eq!(mode, SidebarMode::Hidden);
}

#[gpui::test]
fn open_and_save_as_run_from_the_file_menu(cx: &mut TestAppContext) {
    let elsewhere = tempfile::tempdir().unwrap();
    let file = elsewhere.path().join("todo.txt");
    std::fs::write(&file, "from elsewhere").unwrap();
    let (root, cx) = common::open_main_window(cx);
    cx.set_global(PickFileForTests(Some(file)));

    choose("File", "Open\u{2026}", cx);
    assert_eq!(editor_text(&root, cx), "from elsewhere");

    choose("File", "Save as\u{2026}", cx);
    cx.deactivate_window();
    let copy = elsewhere.path().join("copy.txt");
    cx.simulate_new_path_selection(|_| Some(copy.clone()));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "from elsewhere");
}
