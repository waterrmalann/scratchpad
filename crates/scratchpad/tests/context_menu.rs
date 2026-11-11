//! The note's context menu (ADR 0175): right-click, Shift+F10 or the Menu key in the note; the
//! Edit commands, a Formatting submenu, and the corrections of a misspelled word at the top.

mod common;

use gpui::{
    Bounds, ClipboardItem, Entity, EntityInputHandler, Focusable, Modifiers, MouseButton, Pixels,
    Point, TestAppContext, VisualTestContext, px, size,
};
use scratchpad::AppWindow;
use scratchpad::editor_view::EditorView;
use scratchpad::menu_bar::ShownItem;
use scratchpad::spelling;
use scratchpad_spell::{Speller, WordList};

fn open<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    // As after a click in the note.
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    (root, cx)
}

fn editor(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Entity<EditorView> {
    common::editor(root, cx)
}

/// The items of the open context menu (or of its open submenu); `None` while it is closed.
fn items(
    root: &Entity<AppWindow>,
    submenu: bool,
    cx: &mut VisualTestContext,
) -> Option<Vec<ShownItem>> {
    let editor = editor(root, cx);
    cx.update(|window, cx| editor.read(cx).context_menu_items(submenu, window))
}

/// `label shortcut` of each item of the open menu.
fn labels(root: &Entity<AppWindow>, submenu: bool, cx: &mut VisualTestContext) -> Vec<String> {
    items(root, submenu, cx)
        .expect("the menu is open")
        .into_iter()
        .map(|item| match item.shortcut {
            Some(keys) => format!("{} {keys}", item.label),
            None => item.label,
        })
        .collect()
}

fn enabled(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    items(root, false, cx)
        .expect("the menu is open")
        .into_iter()
        .filter(|item| item.enabled)
        .map(|item| item.label)
        .collect()
}

fn is_open(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    items(root, false, cx).is_some()
}

fn text(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> String {
    common::editor_text(root, cx)
}

/// The selection as a byte range.
fn selection(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    let editor = editor(root, cx);
    editor.read_with(cx, |editor, _| {
        let range = editor.editor().selection().range();
        range.start.0..range.end.0
    })
}

fn note_focused(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = editor(root, cx);
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

/// Where the text from byte `start` to `end` is drawn.
fn bounds_of(
    start: usize,
    end: usize,
    root: &Entity<AppWindow>,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    let text = text(root, cx);
    let range = text[..start].encode_utf16().count()..text[..end].encode_utf16().count();
    editor(root, cx)
        .update_in(cx, |editor, window, cx| {
            editor.bounds_for_range(range, Bounds::default(), window, cx)
        })
        .expect("the text is on screen")
}

/// The middle of the last occurrence of `word` on screen.
fn position_of(word: &str, root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Point<Pixels> {
    let start = text(root, cx).rfind(word).expect("the word is in the note");
    bounds_of(start, start + word.len(), root, cx).center()
}

fn right_click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

fn right_click(word: &str, root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let position = position_of(word, root, cx);
    right_click_at(position, cx);
}

/// Chooses `item` of the context menu opened on `word`.
fn choose(word: &str, item: &str, root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    right_click(word, root, cx);
    common::click(&format!("context-menu:{item}"), cx);
    cx.run_until_parked();
}

fn hover(selector: &str, cx: &mut VisualTestContext) {
    let position = common::center_of(selector, cx);
    cx.simulate_mouse_move(position, None, Modifiers::none());
}

fn menu_origin(cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.debug_bounds("context-menu")
        .expect("the menu was drawn")
        .origin
}

fn near(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    (a.x - b.x).abs() < px(1.) && (a.y - b.y).abs() < px(1.)
}

#[gpui::test]
fn a_right_click_opens_the_edit_commands_at_the_pointer(cx: &mut TestAppContext) {
    let (root, cx) = open("one two three", cx);
    assert!(!is_open(&root, cx));
    let position = position_of("two", &root, cx);
    right_click_at(position, cx);
    assert_eq!(
        labels(&root, false, cx),
        [
            "Undo Ctrl+Z",
            "Cut Ctrl+X",
            "Copy Ctrl+C",
            "Paste Ctrl+V",
            "Delete Del",
            "Select all Ctrl+A",
            "Formatting",
        ]
    );
    assert!(
        near(menu_origin(cx), position),
        "the menu opens at the pointer"
    );
    assert!(!note_focused(&root, cx), "the menu has the keys");
}

#[gpui::test]
fn a_right_click_moves_the_cursor_unless_it_is_in_the_selection(cx: &mut TestAppContext) {
    let (root, cx) = open("one two three", cx);
    // "one two" selected.
    cx.simulate_keystrokes("ctrl-home ctrl-shift-right ctrl-shift-right");
    let selected = selection(&root, cx);
    assert_eq!(text(&root, cx)[selected.clone()].trim_end(), "one two");

    right_click("one", &root, cx);
    assert_eq!(
        selection(&root, cx),
        selected,
        "inside, the selection stays"
    );
    assert!(enabled(&root, cx).contains(&"Cut".to_owned()));
    cx.simulate_keystrokes("escape");

    right_click("three", &root, cx);
    let cursor = selection(&root, cx);
    assert!(cursor.is_empty(), "outside, the selection goes");
    assert!(
        (8..=13).contains(&cursor.start),
        "the cursor is where the pointer was, in \"three\": {cursor:?}"
    );
    assert!(!enabled(&root, cx).contains(&"Cut".to_owned()));
}

#[gpui::test]
fn the_commands_act_on_the_note(cx: &mut TestAppContext) {
    let (root, cx) = open("abc def", cx);

    choose("def", "Select all", &root, cx);
    assert_eq!(selection(&root, cx), 0..7);
    assert!(note_focused(&root, cx), "the note has the focus again");
    assert!(!is_open(&root, cx));

    // Right-clicking the selection keeps it for Copy and Cut.
    choose("def", "Copy", &root, cx);
    assert_eq!(clipboard(cx).as_deref(), Some("abc def"));
    choose("def", "Cut", &root, cx);
    assert_eq!(text(&root, cx), "");
    assert_eq!(clipboard(cx).as_deref(), Some("abc def"));

    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("xyz uvw".into())));
    cx.simulate_keystrokes("ctrl-v");
    // Paste goes where the right-click put the cursor: at the end of "uvw".
    let uvw = bounds_of(4, 7, &root, cx);
    right_click_at(gpui::point(uvw.right() + px(20.), uvw.center().y), cx);
    common::click("context-menu:Paste", cx);
    assert_eq!(text(&root, cx), "xyz uvwxyz uvw");

    // The first "xyz", selected.
    cx.simulate_keystrokes("ctrl-home shift-right shift-right shift-right");
    let xyz = bounds_of(0, 3, &root, cx).center();
    right_click_at(xyz, cx);
    common::click("context-menu:Delete", cx);
    assert_eq!(text(&root, cx), " uvwxyz uvw");

    choose("uvw", "Undo", &root, cx);
    assert_eq!(text(&root, cx), "xyz uvwxyz uvw");
    // Typing goes on in the note.
    cx.simulate_input("!");
    assert!(text(&root, cx).contains('!'));
}

#[gpui::test]
fn commands_that_cannot_apply_are_disabled_as_in_the_edit_menu(cx: &mut TestAppContext) {
    let (root, cx) = open("abc", cx);

    // Nothing to undo and nothing selected.
    right_click("abc", &root, cx);
    assert_eq!(enabled(&root, cx), ["Paste", "Select all", "Formatting"]);
    // Delete would delete a letter if it ran; the menu stays.
    common::click("context-menu:Delete", cx);
    assert_eq!(text(&root, cx), "abc");
    assert!(is_open(&root, cx), "a disabled item keeps the menu");
    cx.simulate_keystrokes("escape");

    cx.simulate_keystrokes("end");
    cx.simulate_input("d");
    cx.simulate_keystrokes("shift-left");
    let selected_d = bounds_of(3, 4, &root, cx).center();
    right_click_at(selected_d, cx);
    assert_eq!(
        enabled(&root, cx),
        [
            "Undo",
            "Cut",
            "Copy",
            "Paste",
            "Delete",
            "Select all",
            "Formatting"
        ]
    );
    cx.simulate_keystrokes("escape");

    // A read-only note can be copied from but not changed.
    let editor = editor(&root, cx);
    editor.update(cx, |editor, _| editor.set_read_only(true));
    cx.simulate_keystrokes("ctrl-a");
    right_click("abcd", &root, cx);
    assert_eq!(enabled(&root, cx), ["Copy", "Select all"]);
    cx.simulate_keystrokes("escape");

    // Plain text has no Markdown formatting.
    editor.update(cx, |editor, cx| {
        editor.set_read_only(false);
        editor.set_markdown(false, cx);
    });
    right_click("abcd", &root, cx);
    assert!(!enabled(&root, cx).contains(&"Formatting".to_owned()));
}

#[gpui::test]
fn shift_f10_and_the_menu_key_open_it_at_the_cursor(cx: &mut TestAppContext) {
    let (root, cx) = open("first line\nsecond", cx);
    cx.simulate_keystrokes("ctrl-end");
    let caret = bounds_of(17, 17, &root, cx);

    cx.simulate_keystrokes("shift-f10");
    assert!(is_open(&root, cx));
    assert!(
        near(menu_origin(cx), caret.bottom_left()),
        "below the cursor"
    );
    assert_eq!(selection(&root, cx), 17..17, "the cursor stays");
    cx.simulate_keystrokes("escape");
    assert!(!is_open(&root, cx));
    assert!(note_focused(&root, cx));

    cx.simulate_keystrokes("menu");
    assert!(is_open(&root, cx));
    // From the keyboard the first item that applies is highlighted: here Paste.
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("!".into())));
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&root, cx), "first line\nsecond!");
    assert!(!is_open(&root, cx));
}

#[gpui::test]
fn arrow_keys_and_enter_choose_items_and_the_note_keys_wait(cx: &mut TestAppContext) {
    let (root, cx) = open("word", cx);
    cx.simulate_keystrokes("end");

    // While the menu is open, the note's keys do not change the note.
    cx.simulate_keystrokes("shift-f10 backspace tab ctrl-z");
    assert_eq!(text(&root, cx), "word");
    assert!(is_open(&root, cx));

    // Paste is highlighted; Down skips the disabled Delete to Select all.
    cx.simulate_keystrokes("down enter");
    assert_eq!(selection(&root, cx), 0..4);

    // Up wraps around to Formatting; Right enters it with Bold highlighted.
    cx.simulate_keystrokes("shift-f10 up right");
    assert_eq!(
        labels(&root, true, cx),
        [
            "Bold Ctrl+B",
            "Italic Ctrl+I",
            "Strikethrough Ctrl+Shift+X",
            "Inline code Ctrl+E",
            "Link Ctrl+K"
        ]
    );
    // Left and Escape leave the submenu first.
    cx.simulate_keystrokes("left");
    assert_eq!(items(&root, true, cx), Some(Vec::new()));
    cx.simulate_keystrokes("enter escape");
    assert!(is_open(&root, cx));
    assert_eq!(items(&root, true, cx), Some(Vec::new()));
    // Enter on Formatting enters it too.
    cx.simulate_keystrokes("enter down enter");
    assert_eq!(text(&root, cx), "*word*");
    assert!(!is_open(&root, cx));
    assert!(note_focused(&root, cx));
}

#[gpui::test]
fn the_formatting_submenu_formats_the_selection(cx: &mut TestAppContext) {
    let (root, cx) = open("say word", cx);
    cx.simulate_keystrokes("end ctrl-shift-left");

    right_click("word", &root, cx);
    assert_eq!(items(&root, true, cx), Some(Vec::new()));
    hover("context-menu:Formatting", cx);
    assert_eq!(items(&root, true, cx).map(|items| items.len()), Some(5));
    common::click("context-menu:Bold", cx);
    cx.run_until_parked();
    assert_eq!(text(&root, cx), "say **word**");

    cx.simulate_keystrokes("ctrl-z ctrl-z end ctrl-shift-left");
    assert_eq!(text(&root, cx), "say word");
    right_click("word", &root, cx);
    hover("context-menu:Formatting", cx);
    hover("context-menu:Link", cx);
    common::click("context-menu:Link", cx);
    cx.run_until_parked();
    assert_eq!(text(&root, cx), "say [word]()");
}

#[gpui::test]
fn escape_and_clicks_elsewhere_only_close_it(cx: &mut TestAppContext) {
    let (root, cx) = open("one two three", cx);
    cx.simulate_keystrokes("end");

    right_click("one", &root, cx);
    let cursor = selection(&root, cx);
    // A click elsewhere in the note closes the menu, and leaves the cursor.
    // Right of the menu, on the line it opened on, where a click would move the cursor.
    let menu = cx.debug_bounds("context-menu").unwrap();
    let elsewhere = gpui::point(menu.right() + px(100.), menu.top() + px(4.));
    cx.simulate_click(elsewhere, Modifiers::none());
    cx.run_until_parked();
    assert!(!is_open(&root, cx));
    assert_eq!(selection(&root, cx), cursor);
    assert!(note_focused(&root, cx));

    // So does a right-click: it opens no other menu.
    right_click("one", &root, cx);
    right_click_at(elsewhere, cx);
    assert!(!is_open(&root, cx));
    assert_eq!(selection(&root, cx), cursor);

    // A click on the menu bar only closes it too.
    right_click("one", &root, cx);
    common::click("menu:Edit", cx);
    assert!(!is_open(&root, cx));
    let bar = root.read_with(cx, |root, _| root.menu_bar().clone());
    assert_eq!(bar.read_with(cx, |bar, _| bar.open_menu()), None);

    // Opening the Edit menu from the keyboard takes the focus, and closes it.
    right_click("one", &root, cx);
    cx.simulate_keystrokes("alt-e");
    assert!(!is_open(&root, cx));
    assert_eq!(bar.read_with(cx, |bar, _| bar.open_menu()), Some("Edit"));
}

#[gpui::test]
fn a_misspelled_word_has_its_corrections_first(cx: &mut TestAppContext) {
    let speller = Speller::inline(Box::new(WordList::new(["the", "quick", "fox"])));
    cx.update(|cx| spelling::set_speller(speller.clone(), cx));
    let (root, cx) = open("the quikc fox", cx);

    // The suggestions fill in when the checker answers.
    speller.hold();
    right_click("quikc", &root, cx);
    let corrections = |root: &Entity<AppWindow>, cx: &mut VisualTestContext| {
        labels(root, false, cx)[..3].to_vec()
    };
    assert_eq!(
        corrections(&root, cx),
        ["Add to dictionary", "Ignore", "Undo Ctrl+Z"]
    );
    speller.release();
    cx.run_until_parked();
    assert_eq!(
        corrections(&root, cx),
        ["quick", "Add to dictionary", "Ignore"]
    );
    assert_eq!(
        &labels(&root, false, cx)[3..],
        [
            "Undo Ctrl+Z",
            "Cut Ctrl+X",
            "Copy Ctrl+C",
            "Paste Ctrl+V",
            "Delete Del",
            "Select all Ctrl+A",
            "Formatting",
        ]
    );
    // The arrow keys go through them as through the rest.
    cx.simulate_keystrokes("down enter");
    assert_eq!(text(&root, cx), "the quick fox");

    // Shift+F10 offers them for the word at the cursor; elsewhere there are none.
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text("the quikc fox", cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-home right right right right right shift-f10");
    assert_eq!(corrections(&root, cx)[0], "quick");
    cx.simulate_keystrokes("escape ctrl-end shift-f10");
    assert_eq!(corrections(&root, cx)[0], "Undo Ctrl+Z");
}
