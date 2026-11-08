//! End-to-end tests of spell checking (ADR 0170): misspelled words in the note's prose, the
//! menu of corrections, autocorrect and the View menu's switches.
//!
//! The system's checker is replaced by a word list, answering on the test's own thread.

mod common;

use std::time::Duration;

use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, MouseButton, Pixels, Point, TestAppContext,
    VisualTestContext, px, size,
};
use scratchpad::actions::view::{ToggleAutocorrect, ToggleSpellCheck};
use scratchpad::editor_view::EditorView;
use scratchpad::settings::SAVE_DELAY;
use scratchpad::spelling;
use scratchpad_core::Config;
use scratchpad_spell::{Speller, WordList};

/// Longer than the pause the editor waits for before checking typed text.
const CHECK_WAIT: Duration = Duration::from_millis(400);

const WORDS: &[&str] = &[
    "the", "quick", "brown", "fox", "jumps", "see", "and", "docs", "title", "a", "note", "is",
    "here", "it", "fn", "at",
];

fn use_word_list(cx: &mut TestAppContext) {
    let list = WordList::new(WORDS.iter().copied()).with_autocorrect("teh", "the");
    cx.update(|cx| spelling::set_speller(Speller::inline(Box::new(list)), cx));
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    use_word_list(cx);
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    (editor, cx)
}

fn misspelled(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |editor, _| editor.misspelled_words())
}

fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.text())
}

fn menu(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    editor.read_with(cx, |editor, _| editor.spelling_menu_items())
}

/// Types `text` a character at a time, as keystrokes arrive.
fn type_text(text: &str, cx: &mut VisualTestContext) {
    for c in text.chars() {
        cx.simulate_input(&c.to_string());
    }
}

/// The middle of the first occurrence of `word` on screen.
fn position_of(
    word: &str,
    editor: &Entity<EditorView>,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let text = text(editor, cx);
    let start = text.find(word).expect("the word is in the note");
    let range =
        text[..start].encode_utf16().count()..text[..start + word.len()].encode_utf16().count();
    editor
        .update_in(cx, |editor, window, cx| {
            editor.bounds_for_range(range, Bounds::default(), window, cx)
        })
        .expect("the word is on screen")
        .center()
}

fn right_click(word: &str, editor: &Entity<EditorView>, cx: &mut VisualTestContext) {
    let position = position_of(word, editor, cx);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn misspelled_prose_is_found_but_not_code_or_addresses(cx: &mut TestAppContext) {
    let (editor, cx) = open(
        cx,
        "# Title\n\nThe quikc brown fox jumps.\n\nSee `cofe` and [the docs](https://exmaple.com) \
         at https://wrold.example.\n\n```\nfn mian() {}\n```\n",
    );
    assert_eq!(misspelled(&editor, cx), ["quikc"]);
}

#[gpui::test]
fn typed_words_are_checked_once_typing_pauses(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "");
    type_text("the fxo jumps", cx);
    assert_eq!(
        misspelled(&editor, cx),
        Vec::<String>::new(),
        "not while typing"
    );
    common::wait(CHECK_WAIT, cx);
    assert_eq!(misspelled(&editor, cx), ["fxo"]);

    // Changing the word takes its underline away until it is checked again.
    cx.simulate_keystrokes("ctrl-home right right right right right right");
    type_text("x", cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    common::wait(CHECK_WAIT, cx);
    assert_eq!(misspelled(&editor, cx), ["fxxo"]);
}

#[gpui::test]
fn a_suggestion_from_the_menu_replaces_the_word(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "The quikc brown fox");
    right_click("quikc", &editor, cx);
    assert_eq!(
        menu(&editor, cx),
        Some(vec![
            "quick".to_owned(),
            "Add to dictionary".to_owned(),
            "Ignore".to_owned()
        ])
    );
    common::click("spelling:quick", cx);
    assert_eq!(text(&editor, cx), "The quick brown fox");
    assert_eq!(menu(&editor, cx), None);
    // One undo step brings the word back.
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "The quikc brown fox");
}

#[gpui::test]
fn added_and_ignored_words_are_no_longer_underlined(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "zorp is here, blib is here, zorp");
    assert_eq!(misspelled(&editor, cx), ["zorp", "blib", "zorp"]);

    right_click("zorp", &editor, cx);
    common::click("spelling:Add to dictionary", cx);
    assert_eq!(misspelled(&editor, cx), ["blib"]);

    right_click("blib", &editor, cx);
    common::click("spelling:Ignore", cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    cx.run_until_parked();
    assert_eq!(
        misspelled(&editor, cx),
        Vec::<String>::new(),
        "the check agrees"
    );
    assert_eq!(text(&editor, cx), "zorp is here, blib is here, zorp");
}

#[gpui::test]
fn a_repeated_word_can_be_deleted(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "the fox jumps jumps here");
    assert_eq!(misspelled(&editor, cx), ["jumps"]);
    let text_now = text(&editor, cx);
    let second = text_now.rfind("jumps").unwrap();
    // Right-click the second "jumps": the first one is fine.
    let position = {
        let start = text_now[..second].encode_utf16().count();
        editor
            .update_in(cx, |editor, window, cx| {
                editor.bounds_for_range(start..start + 5, Bounds::default(), window, cx)
            })
            .unwrap()
            .center()
    };
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        menu(&editor, cx),
        Some(vec!["Delete repeated word".to_owned(), "Ignore".to_owned()])
    );
    common::click("spelling:Delete repeated word", cx);
    assert_eq!(text(&editor, cx), "the fox jumps here");
}

#[gpui::test]
fn a_right_click_elsewhere_opens_no_menu(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "the quikc fox");
    right_click("fox", &editor, cx);
    assert_eq!(menu(&editor, cx), None);
    right_click("quikc", &editor, cx);
    assert!(menu(&editor, cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(menu(&editor, cx), None);
}

#[gpui::test]
fn known_typos_are_corrected_as_they_are_typed(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "");
    type_text("Is teh fox", cx);
    assert_eq!(text(&editor, cx), "Is the fox");
    // Ctrl+Z right after the space takes back the correction, then what was typed.
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    type_text("Is teh ", cx);
    assert_eq!(text(&editor, cx), "Is the ");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "Is teh ");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "");

    // Not in code, and not once switched off.
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    type_text("`teh ", cx);
    assert_eq!(text(&editor, cx), "`teh `", "the backtick is paired");
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    cx.dispatch_action(ToggleAutocorrect);
    type_text("teh ", cx);
    assert_eq!(text(&editor, cx), "teh ");
}

#[gpui::test]
fn plain_text_files_are_not_checked(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "the quikc fox");
    assert_eq!(misspelled(&editor, cx), ["quikc"]);
    editor.update(cx, |editor, cx| editor.set_markdown(false, cx));
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    editor.update(cx, |editor, cx| editor.set_markdown(true, cx));
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), ["quikc"]);
}

#[gpui::test]
fn spell_check_can_be_turned_off_and_is_remembered(cx: &mut TestAppContext) {
    use_word_list(cx);
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();

    let (root, cx) = common::open_with(storage.clone(), cx);
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), ["quikc"]);

    cx.dispatch_action(ToggleSpellCheck);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    common::wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).spell_check, Some(false));
    common::close(cx);

    let (root, cx) = common::open_with(storage, cx);
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |editor, _| editor.spell_check()));
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
}
