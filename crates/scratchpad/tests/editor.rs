//! End-to-end tests of the editor view, driven through the real main window like a user would.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size. The window is
//! 1100 × 720, so the editor pane is 840 px wide and the text column 680 px (75 chars per row).

mod common;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{ClipboardItem, Entity, TestAppContext, VisualTestContext, px, size};
use scratchpad::editor_view::{EditorEvent, EditorView};

const CHARS_PER_ROW: usize = 75;

fn open_editor<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = cx.update(|_, cx| root.read(cx).editor_pane().read(cx).editor().clone());
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text())
}

/// The selection as `(anchor, head)` byte offsets.
fn selection(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> (usize, usize) {
    editor.read_with(cx, |editor, _| {
        let selection = editor.editor().selection();
        (selection.anchor.0, selection.head.0)
    })
}

fn cursor(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> usize {
    selection(editor, cx).1
}

fn line_of_cursor(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| {
        let editor = editor.editor();
        editor.buffer().line_of(editor.selection().head)
    })
}

fn visible_lines(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> Range<usize> {
    editor.read_with(cx, |editor, _| editor.visible_lines())
}

fn numbered_lines(count: usize) -> String {
    (0..count).map(|i| format!("line {i}\n")).collect()
}

fn clipboard_line_break() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

#[gpui::test]
fn word_motions_and_word_deletion(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "one two  three");

    cx.simulate_keystrokes("ctrl-right ctrl-right");
    assert_eq!(cursor(&editor, cx), 7);
    cx.simulate_keystrokes("ctrl-backspace");
    assert_eq!(text(&editor, cx), "one   three");
    assert_eq!(cursor(&editor, cx), 4);
    cx.simulate_keystrokes("ctrl-delete");
    assert_eq!(text(&editor, cx), "one ");
    cx.simulate_keystrokes("ctrl-left");
    assert_eq!(cursor(&editor, cx), 0);
}

#[gpui::test]
fn select_all_and_cut_moves_the_document_to_the_clipboard(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "first\nsecond");

    cx.simulate_keystrokes("ctrl-a ctrl-x");

    assert_eq!(text(&editor, cx), "");
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(
        clipboard,
        Some(format!("first{}second", clipboard_line_break()))
    );

    cx.simulate_keystrokes("ctrl-v");
    assert_eq!(text(&editor, cx), "first\nsecond");
}

#[gpui::test]
fn copy_keeps_the_document_and_paste_normalizes_line_breaks(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "ab");

    cx.simulate_keystrokes("shift-right ctrl-c");
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("a"));
    assert_eq!(text(&editor, cx), "ab");

    cx.write_to_clipboard(ClipboardItem::new_string("x\r\ny\rz\n".into()));
    cx.simulate_keystrokes("end ctrl-v");

    assert_eq!(text(&editor, cx), "abx\ny\nz\n");
    assert_eq!(cursor(&editor, cx), "abx\ny\nz\n".len());
}

#[gpui::test]
fn the_caret_blinks_only_while_focused(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "ab");
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let caret_visible = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.caret_visible());
    let wait = |ms, cx: &mut VisualTestContext| {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
    };

    assert!(caret_visible(cx));
    wait(600, cx);
    assert!(!caret_visible(cx), "blinked off");
    cx.simulate_keystrokes("right");
    assert!(
        caret_visible(cx),
        "moving the cursor shows the caret at once"
    );
    wait(400, cx);
    assert!(caret_visible(cx), "and restarts the blink period");
    wait(200, cx);
    assert!(!caret_visible(cx));

    cx.deactivate_window();
    assert!(!caret_visible(cx), "hidden in an inactive window");
    wait(2_000, cx);
    assert!(!caret_visible(cx));
}

#[gpui::test]
fn duplicate_and_move_lines(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "one\ntwo\nthree");

    cx.simulate_keystrokes("ctrl-shift-d");
    assert_eq!(text(&editor, cx), "one\none\ntwo\nthree");
    assert_eq!(line_of_cursor(&editor, cx), 1);

    cx.simulate_keystrokes("alt-down alt-down");
    assert_eq!(text(&editor, cx), "one\ntwo\nthree\none");
    cx.simulate_keystrokes("alt-up");
    assert_eq!(text(&editor, cx), "one\ntwo\none\nthree");
}

#[gpui::test]
fn tab_inserts_spaces(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "x");

    cx.simulate_keystrokes("tab");

    assert_eq!(text(&editor, cx), "    x");
}

/// Emoji ZWJ sequences, flags, stacked combining marks, CJK without spaces, tabs and a word wider
/// than the column, in one wrapped paragraph.
const ADVERSARIAL: &str = "👨‍👩‍👧 fam e\u{301}\u{302}\u{303} 日本語の文章を書きます。🇺🇸🇩🇪\tTab\t\there \
     xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \
     ok 👍🏽 e\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\n\
     ã\u{303}\u{303}\n\t日本";

#[gpui::test]
fn down_visits_every_row_and_home_and_end_stay_on_it(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, ADVERSARIAL);

    let mut row_start = 0;
    let mut rows = 1;
    loop {
        cx.simulate_keystrokes("end");
        let row_end = cursor(&editor, cx);
        cx.simulate_keystrokes("home");
        assert_eq!(cursor(&editor, cx), row_start);
        assert!(row_end > row_start);
        cx.simulate_keystrokes("down");
        if cursor(&editor, cx) == ADVERSARIAL.len() {
            break;
        }
        assert!(
            cursor(&editor, cx) > row_end,
            "row {rows} starts after the previous one"
        );
        row_start = cursor(&editor, cx);
        rows += 1;
    }
    // The paragraph wraps before the long word, which is broken once; then two short lines.
    assert_eq!(rows, 5);
}

#[gpui::test]
fn text_changes_are_reported_but_loading_is_not(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");
    let events = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(*event)
        })
    });

    editor.update(cx, |editor, cx| editor.set_text("loaded", cx));
    cx.simulate_keystrokes("right shift-right");
    assert!(
        events.borrow().is_empty(),
        "loading and moving are not edits"
    );

    cx.simulate_keystrokes("enter ctrl-z");
    assert_eq!(
        *events.borrow(),
        [EditorEvent::Changed, EditorEvent::Changed]
    );
    assert_eq!(editor.read_with(cx, |editor, _| editor.text()), "loaded");
}

#[gpui::test]
fn up_down_home_and_end_follow_wrapped_rows(cx: &mut TestAppContext) {
    // 40 five-char words: wraps after every 15 words (75 chars).
    let line = "word ".repeat(40);
    let (editor, cx) = open_editor(cx, &format!("{line}\nnext"));
    cx.simulate_keystrokes("right right right");

    cx.simulate_keystrokes("down");
    assert_eq!(
        cursor(&editor, cx),
        CHARS_PER_ROW + 3,
        "same x, next row of the line"
    );
    cx.simulate_keystrokes("end");
    assert_eq!(
        cursor(&editor, cx),
        2 * CHARS_PER_ROW - 1,
        "before the space it wrapped at"
    );
    cx.simulate_keystrokes("home");
    assert_eq!(cursor(&editor, cx), CHARS_PER_ROW);
    cx.simulate_keystrokes("end up");
    assert_eq!(cursor(&editor, cx), CHARS_PER_ROW - 1);
    cx.simulate_keystrokes("down down down");
    assert_eq!(line_of_cursor(&editor, cx), 1);
    cx.simulate_keystrokes("down");
    assert_eq!(
        cursor(&editor, cx),
        line.len() + 5,
        "past the last row: document end"
    );
    cx.simulate_keystrokes("ctrl-home shift-down");
    assert_eq!(selection(&editor, cx), (0, CHARS_PER_ROW));
}

#[gpui::test]
fn page_down_moves_the_cursor_and_scrolls_by_a_screen(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(200));

    cx.simulate_keystrokes("pagedown");
    let first_page = line_of_cursor(&editor, cx);
    let visible = visible_lines(&editor, cx);
    assert!(
        (25..35).contains(&first_page),
        "cursor on line {first_page}"
    );
    assert!(
        visible.start > 20 && visible.contains(&first_page),
        "{visible:?}"
    );

    cx.simulate_keystrokes("pagedown");
    assert_eq!(line_of_cursor(&editor, cx), 2 * first_page);
    cx.simulate_keystrokes("pageup pageup");
    assert_eq!(cursor(&editor, cx), 0);
    assert_eq!(visible_lines(&editor, cx).start, 0);
    cx.simulate_keystrokes("shift-pagedown");
    assert_eq!(selection(&editor, cx).0, 0);
    assert_eq!(line_of_cursor(&editor, cx), first_page);
}

#[gpui::test]
fn keyboard_moves_keep_the_cursor_on_screen(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(10_000));

    cx.simulate_keystrokes("ctrl-end");
    assert!(visible_lines(&editor, cx).contains(&10_000));
    cx.simulate_keystrokes("up up");
    assert!(visible_lines(&editor, cx).contains(&9_998));
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(visible_lines(&editor, cx).start, 0);
    for _ in 0..40 {
        cx.simulate_keystrokes("down");
    }
    let visible = visible_lines(&editor, cx);
    assert!(visible.start > 0, "{visible:?}");
    assert!(
        visible.contains(&42),
        "two rows of margin below the cursor: {visible:?}"
    );
}

#[gpui::test]
fn the_layout_cache_stays_bounded_while_paging_through_a_long_note(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(5_000));

    for _ in 0..200 {
        cx.simulate_keystrokes("pagedown");
    }

    assert!(visible_lines(&editor, cx).contains(&5_000));
    let cached = editor.read_with(cx, |editor, _| editor.cached_layout_count());
    assert!(cached <= 600, "{cached} layouts kept");
}

#[gpui::test]
fn huge_documents_only_lay_out_visible_lines(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(100_000));
    let laid_out =
        |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.cached_layout_count());

    let visible = visible_lines(&editor, cx);
    assert!(visible.len() < 40, "{visible:?}");
    assert!(laid_out(cx) < 50, "{} lines shaped", laid_out(cx));

    cx.simulate_keystrokes("ctrl-end enter");
    assert!(laid_out(cx) < 100, "{} lines shaped", laid_out(cx));
}
