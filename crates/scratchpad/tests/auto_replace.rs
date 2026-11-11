//! Typed replacements in the window (ADR 0180): `->` typed becomes →, Backspace and Ctrl+Z take it back, pasted
//! text and plain text files are left alone, and View > Auto-replace symbols turns it off and is remembered.

mod common;

use common::click;
use gpui::{ClipboardItem, Entity, TestAppContext, VisualTestContext};
use scratchpad::actions::view::ToggleAutoReplace;
use scratchpad::editor_view::EditorView;
use scratchpad::settings::SAVE_DELAY;
use scratchpad_core::Config;

fn open_editor<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    let (root, cx) = common::open_main_window(cx);
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text())
}

#[gpui::test]
fn typed_sequences_become_symbols_and_backspace_or_undo_takes_them_back(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");
    cx.simulate_input("a -> b <-> c... (c)");
    assert_eq!(text(&editor, cx), "a → b ↔ c… ©");

    // Backspace right after one brings back what was typed, then deletes as usual.
    cx.simulate_input(" x--");
    assert_eq!(text(&editor, cx), "a → b ↔ c… © x—");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&editor, cx), "a → b ↔ c… © x--");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&editor, cx), "a → b ↔ c… © x-");

    // Ctrl+Z right after one does too.
    cx.simulate_input(">");
    assert_eq!(text(&editor, cx), "a → b ↔ c… © x→");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "a → b ↔ c… © x->");
}

#[gpui::test]
fn pasted_text_is_not_replaced(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");
    cx.write_to_clipboard(ClipboardItem::new_string("a -> b... (c) --".into()));
    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(text(&editor, cx), "a -> b... (c) --");
    // Typing after it still replaces, but only what is typed.
    cx.simulate_input("=>");
    assert_eq!(text(&editor, cx), "a -> b... (c) --⇒");
}

#[gpui::test]
fn code_and_plain_text_files_are_left_alone(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "```\n\n```");
    cx.simulate_keystrokes("ctrl-home down");
    cx.simulate_input("x -> y");
    assert_eq!(text(&editor, cx), "```\nx -> y\n```");

    editor.update(cx, |editor, cx| {
        editor.set_text("", cx);
        editor.set_markdown(false, cx);
    });
    cx.simulate_input("a -> b");
    assert_eq!(text(&editor, cx), "a -> b");
}

#[gpui::test]
fn the_view_menu_turns_it_off_and_it_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();

    let (root, cx) = common::open_with(storage.clone(), cx);
    let editor = common::editor(&root, cx);
    assert!(editor.read_with(cx, |editor, _| editor.auto_replace()));

    click("menu:View", cx);
    click("menu-item:Auto-replace symbols", cx);
    assert!(!editor.read_with(cx, |editor, _| editor.auto_replace()));
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    cx.simulate_input("a -> b");
    assert_eq!(text(&editor, cx), "a -> b", "off for the next note too");
    common::wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).auto_replace, Some(false));
    common::close(cx);

    // Off after a restart; the action turns it back on.
    let (root, cx) = common::open_with(storage.clone(), cx);
    let editor = common::editor(&root, cx);
    assert!(!editor.read_with(cx, |editor, _| editor.auto_replace()));
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    cx.simulate_input("x...");
    assert_eq!(text(&editor, cx), "x...");
    cx.dispatch_action(ToggleAutoReplace);
    cx.simulate_input(" ->");
    assert_eq!(text(&editor, cx), "x... →");
    // Closing writes the settings at once, whatever is still waiting to be written.
    common::close(cx);
    assert_eq!(Config::load(&config_path).auto_replace, Some(true));
}
