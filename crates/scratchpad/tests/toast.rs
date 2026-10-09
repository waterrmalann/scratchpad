mod common;

use std::time::Duration;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use scratchpad::toast::{self, TOAST_DURATION};

fn shown(cx: &mut VisualTestContext) -> Option<String> {
    let message = cx.update(|_, cx| toast::current(cx).map(String::from));
    // GPUI never clears recorded debug bounds, so they can only prove presence.
    if message.is_some() {
        assert!(cx.debug_bounds("toast").is_some(), "toast not rendered");
    }
    message
}

fn show(message: &'static str, cx: &mut VisualTestContext) {
    cx.update(|_, cx| toast::show_error(message, cx));
    cx.run_until_parked();
}

#[gpui::test]
fn toast_hides_itself_and_a_newer_message_restarts_the_timer(cx: &mut TestAppContext) {
    let (_root, cx) = common::open_main_window(cx);
    assert_eq!(shown(cx), None);

    show("Could not save \"Ideas\". Disk full.", cx);
    assert_eq!(
        shown(cx).as_deref(),
        Some("Could not save \"Ideas\". Disk full.")
    );

    cx.executor().advance_clock(TOAST_DURATION / 2);
    show("Could not delete \"Ideas\". Access is denied.", cx);
    // The first message's timer would fire here; it must not hide the second message.
    cx.executor()
        .advance_clock(TOAST_DURATION / 2 + Duration::from_millis(1));
    assert_eq!(
        shown(cx).as_deref(),
        Some("Could not delete \"Ideas\". Access is denied.")
    );

    cx.executor().advance_clock(TOAST_DURATION / 2);
    assert_eq!(shown(cx), None);
}

#[gpui::test]
fn clicking_the_toast_dismisses_it(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(&"text\n".repeat(100), cx));
    show("Could not save \"Ideas\". Disk full.", cx);

    let bounds = cx.debug_bounds("toast").expect("toast rendered");
    cx.simulate_click(bounds.center(), Modifiers::none());

    assert_eq!(shown(cx), None);
    // Only that: the note under it keeps its caret.
    let caret = editor.read_with(cx, |editor, _| editor.editor().selection().head.0);
    assert_eq!(caret, 0);
}
