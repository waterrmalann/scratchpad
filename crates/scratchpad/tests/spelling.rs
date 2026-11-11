//! End-to-end tests of spell checking (ADR 0170): misspelled words in the note's prose, the
//! corrections at the top of the context menu (ADR 0175), autocorrect and the View menu's
//! switches.
//!
//! The system's checker is replaced by a word list, answering on the test's own thread (or
//! holding its answers back, as a busy checker would).

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext, Bounds, Entity, EntityInputHandler, Focusable, Modifiers, MouseButton, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
    size,
};
use scratchpad::actions::view::{ToggleAutocorrect, ToggleSpellCheck};
use scratchpad::editor_view::EditorView;
use scratchpad::settings::SAVE_DELAY;
use scratchpad::spelling;
use scratchpad_core::Config;
use scratchpad_spell::{Backend, Misspelling, Speller, WordList};

/// Longer than the pause the editor waits for before checking typed text.
const CHECK_WAIT: Duration = Duration::from_millis(400);

const WORDS: &[&str] = &[
    "the", "quick", "brown", "fox", "jumps", "see", "and", "docs", "title", "a", "note", "is",
    "here", "it", "fn", "at",
];

fn word_list() -> WordList {
    WordList::new(WORDS.iter().copied()).with_autocorrect("teh", "the")
}

/// Makes the word list the app's checker; the speller returned can hold its answers back.
fn use_word_list(cx: &mut TestAppContext) -> Speller {
    let speller = Speller::inline(Box::new(word_list()));
    cx.update(|cx| spelling::set_speller(speller.clone(), cx));
    speller
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

    // Changing the word takes its underline away until it is checked again, and while the
    // cursor stays where typing is, the word there is still being typed.
    cx.simulate_keystrokes("ctrl-home right right right right right right");
    type_text("x", cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    common::wait(CHECK_WAIT, cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    cx.simulate_keystrokes("end");
    assert_eq!(misspelled(&editor, cx), ["fxxo"]);
}

#[gpui::test]
fn the_word_being_typed_is_not_underlined(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "");
    type_text("the fxo", cx);
    // Checked once typing pauses, with the cursor still at the end of the word.
    common::wait(CHECK_WAIT, cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    // Moving away ends the word.
    cx.simulate_keystrokes("home");
    assert_eq!(misspelled(&editor, cx), ["fxo"]);
}

#[gpui::test]
fn a_reply_to_an_older_text_is_dropped(cx: &mut TestAppContext) {
    let speller = use_word_list(cx);
    let (root, cx) = common::open_main_window(cx);
    let editor = common::editor(&root, cx);
    speller.hold();
    editor.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    cx.run_until_parked();
    // While the checker works, the misspelled word is corrected at the start of the note.
    cx.simulate_keystrokes("ctrl-home");
    type_text("a ", cx);
    speller.release();
    cx.run_until_parked();
    assert_eq!(
        misspelled(&editor, cx),
        Vec::<String>::new(),
        "not 3 bytes off"
    );

    // A newer check of the same note takes the place of one still waiting.
    speller.hold();
    common::wait(CHECK_WAIT, cx);
    cx.simulate_keystrokes("ctrl-end");
    type_text(" brwn", cx);
    common::wait(CHECK_WAIT, cx);
    speller.release();
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(misspelled(&editor, cx), ["quikc", "brwn"]);
}

#[gpui::test]
fn checks_of_two_notes_do_not_cancel_each_other(cx: &mut TestAppContext) {
    let speller = use_word_list(cx);
    let (root, cx) = common::open_main_window(cx);
    let first = common::editor(&root, cx);
    let second = cx.update(|window, cx| cx.new(|cx| EditorView::new("", window, cx)));
    speller.hold();
    first.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    second.update(cx, |editor, cx| editor.set_text("the brwn fox", cx));
    // The first note is checked again before the checker gets to the second.
    first.update(cx, |editor, cx| editor.set_text("the quikc zorp", cx));
    speller.release();
    cx.run_until_parked();
    assert_eq!(misspelled(&first, cx), ["quikc", "zorp"]);
    assert_eq!(misspelled(&second, cx), ["brwn"]);
}

#[gpui::test]
fn front_matter_is_not_checked(cx: &mut TestAppContext) {
    let (editor, cx) = open(
        cx,
        "---\ntitle: Zorp\ntags: [quikc, blib]\n---\n\nThe brwn fox\n\n---\n\nzorp here\n",
    );
    assert_eq!(misspelled(&editor, cx), ["brwn", "zorp"]);
    // Without its closing line it is no front matter.
    editor.update(cx, |editor, cx| editor.set_text("---\ntitle: Zorp\n", cx));
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), ["Zorp"]);
}

#[gpui::test]
fn addresses_and_paths_are_not_checked(cx: &mut TestAppContext) {
    let (editor, cx) = open(
        cx,
        "see alice@exmaple.com, see notes/tdoo and C:\\Notse\\fiel and quikc",
    );
    assert_eq!(misspelled(&editor, cx), ["quikc"]);
}

#[gpui::test]
fn scrolling_checks_lines_not_checked_yet(cx: &mut TestAppContext) {
    let mut text = "the fox jumps\n".repeat(500);
    text.push_str("a zorp is here\n");
    let (editor, cx) = open(cx, &text);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
    let pane = cx.debug_bounds("editor-pane").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: pane.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-1_000_000.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), ["zorp"]);
}

/// The word list, noting the longest text it was given at once.
struct Measured {
    list: WordList,
    longest: Rc<RefCell<usize>>,
}

impl Backend for Measured {
    fn check(&mut self, text: &str) -> Vec<Misspelling> {
        let mut longest = self.longest.borrow_mut();
        *longest = (*longest).max(text.len());
        self.list.check(text)
    }
    fn suggest(&mut self, word: &str) -> Vec<String> {
        self.list.suggest(word)
    }
    fn add(&mut self, word: &str) {
        self.list.add(word)
    }
    fn ignore(&mut self, word: &str) {
        self.list.ignore(word)
    }
}

#[gpui::test]
fn a_long_line_is_checked_in_pieces_up_to_a_limit(cx: &mut TestAppContext) {
    let longest = Rc::new(RefCell::new(0));
    let backend = Measured {
        list: word_list(),
        longest: longest.clone(),
    };
    cx.update(|cx| spelling::set_speller(Speller::inline(Box::new(backend)), cx));
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    // One line of 400 KB: words to check at 0, ~200 KB, and past the 256 KB limit.
    let filler = "the quick brown fox ".repeat(10_000);
    let line = format!("zorp {filler}blib {filler}quikc");
    editor.update(cx, |editor, cx| editor.set_text(&line, cx));
    cx.run_until_parked();
    assert_eq!(misspelled(&editor, cx), ["zorp", "blib"]);
    let longest = *longest.borrow();
    assert!(
        (1..=16 * 1024).contains(&longest),
        "{longest} bytes at once"
    );
}

#[gpui::test]
fn a_read_only_note_offers_no_corrections(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "The quikc fox jumps jumps");
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    right_click("quikc", &editor, cx);
    assert_eq!(
        menu(&editor, cx),
        Some(vec!["Add to dictionary".to_owned(), "Ignore".to_owned()])
    );
    cx.simulate_keystrokes("escape");
    let text_now = text(&editor, cx);
    let second = text_now.rfind("jumps").unwrap();
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
    assert_eq!(menu(&editor, cx), Some(vec!["Ignore".to_owned()]));
}

#[gpui::test]
fn an_edit_closes_the_menu_and_gives_the_note_the_focus(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "the quikc fox");
    right_click("quikc", &editor, cx);
    assert!(menu(&editor, cx).is_some());
    let note_focused = |editor: &Entity<EditorView>, cx: &mut VisualTestContext| {
        editor.update_in(cx, |editor, window, cx| {
            editor.focus_handle(cx).is_focused(window)
        })
    };
    assert!(!note_focused(&editor, cx), "the menu has the focus");
    // The note changes under the menu, as when another program edits it.
    editor.update(cx, |editor, cx| {
        editor.replace_text("the quikc fox jumps", cx)
    });
    cx.run_until_parked();
    assert_eq!(menu(&editor, cx), None);
    assert!(note_focused(&editor, cx));

    common::wait(CHECK_WAIT, cx);
    right_click("quikc", &editor, cx);
    assert!(menu(&editor, cx).is_some());
    cx.dispatch_action(ToggleSpellCheck);
    assert_eq!(menu(&editor, cx), None);
    assert!(note_focused(&editor, cx));
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
fn a_right_click_elsewhere_offers_no_corrections(cx: &mut TestAppContext) {
    let (editor, cx) = open(cx, "the quikc fox");
    right_click("fox", &editor, cx);
    let context_menu_open = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).context_menu_items(false, window).is_some())
    };
    assert!(context_menu_open(cx), "the context menu opens all the same");
    assert_eq!(menu(&editor, cx), None);
    cx.simulate_keystrokes("escape");
    right_click("quikc", &editor, cx);
    assert!(menu(&editor, cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(menu(&editor, cx), None);
    assert!(!context_menu_open(cx));
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

    // After punctuation too, and next to a web address but not inside one.
    for mark in [".", ",", ";", ":", "!", "?"] {
        editor.update(cx, |editor, cx| editor.set_text("", cx));
        type_text(&format!("teh{mark}"), cx);
        assert_eq!(text(&editor, cx), format!("the{mark}"));
    }
    editor.update(cx, |editor, cx| editor.set_text("", cx));
    type_text("see https://teh.example/teh, teh https://x.io teh ", cx);
    assert_eq!(
        text(&editor, cx),
        "see https://teh.example/teh, the https://x.io the "
    );

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

/// A checker that breaks on its first check, as a crashing system checker would.
struct Broken;

impl Backend for Broken {
    fn check(&mut self, _: &str) -> Vec<Misspelling> {
        panic!("the checker broke (on purpose, for this test)")
    }
    fn suggest(&mut self, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn add(&mut self, _: &str) {}
    fn ignore(&mut self, _: &str) {}
}

#[gpui::test]
fn a_checker_that_stops_turns_spelling_off(cx: &mut TestAppContext) {
    // On a real thread: its end is what is noticed.
    cx.update(|cx| spelling::set_speller(Speller::start(|| Some(Box::new(Broken))), cx));
    let (root, cx) = common::open_main_window(cx);
    let editor = common::editor(&root, cx);
    cx.executor().allow_parking();
    editor.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    let available = |cx: &mut VisualTestContext| cx.update(|_, cx| spelling::is_available(cx));
    for _ in 0..500 {
        cx.run_until_parked();
        if !available(cx) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!available(cx), "View > Spell check is greyed out");
    // Nothing more is sent to it: typing schedules no check.
    type_text(" jumps", cx);
    common::wait(CHECK_WAIT, cx);
    assert_eq!(misspelled(&editor, cx), Vec::<String>::new());
}
