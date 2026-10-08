use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    ClipboardItem, Entity, EntityInputHandler, Focusable, TestAppContext, VisualTestContext,
};
use scratchpad::text_input::{TextInput, TextInputEvent};

/// A window whose only view is a focused text field, plus a log of the events it emits.
fn open_input(
    cx: &mut TestAppContext,
) -> (
    Entity<TextInput>,
    Rc<RefCell<Vec<TextInputEvent>>>,
    &mut VisualTestContext,
) {
    cx.update(scratchpad::init);
    let (input, cx) = cx.add_window_view(|_, cx| TextInput::new("Search", cx));
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    cx.update(|window, cx| {
        window.focus(&input.focus_handle(cx));
        cx.subscribe(&input, move |_, event: &TextInputEvent, _| {
            log.borrow_mut().push(*event)
        })
        .detach();
    });
    cx.run_until_parked();
    (input, events, cx)
}

fn text(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> String {
    input.read_with(cx, |input, _| input.text().to_owned())
}

#[gpui::test]
fn typing_and_deleting_by_grapheme_and_by_word(cx: &mut TestAppContext) {
    let (input, events, cx) = open_input(cx);

    cx.simulate_input("meeting notes e\u{301}");
    assert_eq!(text(&input, cx), "meeting notes e\u{301}");
    assert_eq!(events.borrow().last(), Some(&TextInputEvent::Changed));

    // One Backspace removes the whole "é" (e + combining accent).
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&input, cx), "meeting notes ");

    // Ctrl+Backspace skips the trailing space, then removes the word.
    cx.simulate_keystrokes("ctrl-backspace");
    assert_eq!(text(&input, cx), "meeting ");
    cx.simulate_keystrokes("ctrl-backspace ctrl-backspace");
    assert_eq!(text(&input, cx), "");
}

#[gpui::test]
fn selection_replaces_text(cx: &mut TestAppContext) {
    let (input, _, cx) = open_input(cx);
    cx.simulate_input("abc");

    cx.simulate_keystrokes("left shift-left");
    cx.simulate_input("X");
    assert_eq!(text(&input, cx), "aXc");

    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("new");
    assert_eq!(text(&input, cx), "new");

    cx.simulate_keystrokes("home delete");
    assert_eq!(text(&input, cx), "ew");
}

#[gpui::test]
fn clipboard_keeps_the_field_on_one_line(cx: &mut TestAppContext) {
    let (input, _, cx) = open_input(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("two\r\nlines\n".into()));

    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(text(&input, cx), "two lines ");

    cx.simulate_keystrokes("ctrl-a ctrl-x");
    assert_eq!(text(&input, cx), "");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("two lines ".into())
    );
}

#[gpui::test]
fn enter_and_escape_are_reported(cx: &mut TestAppContext) {
    let (input, events, cx) = open_input(cx);
    cx.simulate_input("x");
    events.borrow_mut().clear();

    cx.simulate_keystrokes("enter escape");

    assert_eq!(
        *events.borrow(),
        [TextInputEvent::Confirmed, TextInputEvent::Cancelled]
    );
    assert_eq!(text(&input, cx), "x");
}

#[gpui::test]
fn ime_composition_replaces_the_marked_text(cx: &mut TestAppContext) {
    let (input, _, cx) = open_input(cx);
    cx.simulate_input("a ");

    input.update_in(cx, |input, window, cx| {
        input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
        assert_eq!(input.marked_text_range(window, cx), Some(2..4));
        input.replace_and_mark_text_in_range(None, "nih", Some(3..3), window, cx);
        input.replace_text_in_range(None, "你", window, cx);
        assert_eq!(input.marked_text_range(window, cx), None);
    });
    cx.simulate_input("!");

    assert_eq!(text(&input, cx), "a 你!");
}

#[gpui::test]
#[allow(clippy::reversed_empty_ranges)] // Deliberately, as a misbehaving IME might send them.
fn ime_ranges_out_of_order_or_inside_a_character_do_not_panic(cx: &mut TestAppContext) {
    let (input, _, cx) = open_input(cx);
    // "😀" is two UTF-16 units: the text is 4 units long.
    cx.simulate_input("a😀b");

    input.update_in(cx, |input, window, cx| {
        let mut actual = None;
        assert_eq!(
            input.text_for_range(3..1, &mut actual, window, cx),
            Some("😀".into())
        );
        assert_eq!(actual, Some(1..3));
        // A range ending between the two halves of the emoji is widened to all of it.
        assert_eq!(
            input.text_for_range(0..2, &mut actual, window, cx),
            Some("a😀".into())
        );
        input.replace_and_mark_text_in_range(Some(4..1), "xy", Some(2..0), window, cx);
        assert_eq!(input.marked_text_range(window, cx), Some(1..3));
    });

    assert_eq!(text(&input, cx), "axy");
}
