//! End-to-end tests of the editor view, driven through the real main window like a user would.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size. The window is
//! 1100 × 720, so the editor pane is 840 px wide and the text column 680 px (75 chars per row).

mod common;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Bounds, ClipboardItem, Entity, EntityInputHandler, Focusable, Pixels, TestAppContext,
    VisualTestContext, point, px, size,
};
use scratchpad::editor_view::{EditorEvent, EditorView};

const ADVANCE: f32 = 9.;
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

/// Screen bounds of the character at UTF-16 `offset`, as the IME sees them.
fn char_bounds(
    editor: &Entity<EditorView>,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    editor.update_in(cx, |editor, window, cx| {
        editor
            .bounds_for_range(offset..offset + 1, Bounds::default(), window, cx)
            .expect("offset is visible")
    })
}

fn numbered_lines(count: usize) -> String {
    (0..count).map(|i| format!("line {i}\n")).collect()
}

fn clipboard_line_break() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

#[gpui::test]
fn typing_enter_and_backspace_edit_the_document(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("wörld");
    cx.simulate_keystrokes("backspace backspace");

    assert_eq!(text(&editor, cx), "hello\nwör");
    assert_eq!(cursor(&editor, cx), "hello\nwör".len());
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
fn typing_replaces_a_shift_selection(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "hello world");

    cx.simulate_keystrokes("end shift-left shift-left shift-left shift-left shift-left");
    assert_eq!(selection(&editor, cx), (11, 6));
    cx.simulate_keystrokes("ctrl-shift-left");
    assert_eq!(selection(&editor, cx), (11, 0));
    cx.simulate_keystrokes("ctrl-shift-right");
    assert_eq!(selection(&editor, cx), (11, 5));
    cx.simulate_input("!");

    assert_eq!(text(&editor, cx), "hello!");
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
fn undo_and_redo_whole_typing_runs(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("hello world");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("more");

    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "hello world\n");
    cx.simulate_keystrokes("ctrl-z ctrl-z");
    assert_eq!(text(&editor, cx), "");
    cx.simulate_keystrokes("ctrl-y");
    assert_eq!(text(&editor, cx), "hello world");
    cx.simulate_keystrokes("ctrl-shift-z ctrl-shift-z");
    assert_eq!(text(&editor, cx), "hello world\nmore");
}

#[gpui::test]
fn losing_focus_ends_the_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");
    // Test windows start inactive, and nothing has focus in an inactive window.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();

    cx.simulate_input("ab");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    let focus = editor.read_with(cx, |editor, cx| editor.focus_handle(cx));
    cx.update(|window, _| window.focus(&focus));
    cx.run_until_parked();
    cx.simulate_input("cd");
    // Switching to another window counts too.
    cx.deactivate_window();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.simulate_input("ef");

    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "abcd");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "ab");
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

#[gpui::test]
fn ime_composition_commits_as_one_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "ab");
    cx.simulate_keystrokes("right");

    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "k", None, window, cx);
        // The caret sits inside the composition: UTF-16 offset 1 of "かな" is byte 3.
        editor.replace_and_mark_text_in_range(None, "かな", Some(1..1), window, cx);
    });
    assert_eq!(text(&editor, cx), "aかなb");
    assert_eq!(selection(&editor, cx), (4, 4));
    let marked = editor.update_in(cx, |editor, window, cx| {
        editor.marked_text_range(window, cx)
    });
    assert_eq!(marked, Some(1..3), "UTF-16 range of the composition");

    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "仮名", window, cx);
    });
    assert_eq!(text(&editor, cx), "a仮名b");
    let marked = editor.update_in(cx, |editor, window, cx| {
        editor.marked_text_range(window, cx)
    });
    assert_eq!(marked, None);

    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "ab");
}

#[gpui::test]
fn dead_keys_replace_their_marked_accent(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    // Windows reports a dead key as marked text, then the composed character as plain input.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "´", None, window, cx);
    });
    cx.simulate_input("é");
    assert_eq!(text(&editor, cx), "é");

    // A Japanese IME's "no composition" notice must not delete the selection.
    cx.simulate_keystrokes("shift-left");
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "", window, cx);
    });
    assert_eq!(text(&editor, cx), "é");
}

#[gpui::test]
fn ime_queries_answer_in_utf16(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "😀 hi");
    cx.simulate_keystrokes("end shift-left shift-left");

    editor.update_in(cx, |editor, window, cx| {
        let selected = editor.selected_text_range(false, window, cx).unwrap();
        assert_eq!(selected.range, 3..5);
        assert!(selected.reversed);
        let mut adjusted = None;
        let text = editor.text_for_range(0..2, &mut adjusted, window, cx);
        assert_eq!(text.as_deref(), Some("😀"));
        assert_eq!(adjusted, Some(0..2));
    });
}

#[gpui::test]
fn ime_bounds_of_a_range_cover_its_first_row(cx: &mut TestAppContext) {
    // Row 0 is "word " × 15 (75 chars); the line wraps after its last space.
    let (editor, cx) = open_editor(cx, &format!("{}\nab\ncd", "word ".repeat(20)));
    let bounds = |range: Range<usize>, cx: &mut VisualTestContext| {
        editor.update_in(cx, |editor, window, cx| {
            editor.bounds_for_range(range, Bounds::default(), window, cx)
        })
    };
    let origin = char_bounds(&editor, 0, cx).origin;

    let across_rows = bounds(70..80, cx).unwrap();
    assert_eq!(across_rows.left() - origin.x, px(70. * ADVANCE));
    assert_eq!(
        across_rows.size.width,
        px(5. * ADVANCE),
        "up to the row's end"
    );
    assert_eq!(across_rows.top(), origin.y);

    let ab = "word ".repeat(20).len() + 1;
    let across_lines = bounds(ab + 1..ab + 4, cx).unwrap();
    assert_eq!(across_lines.left() - origin.x, px(ADVANCE));
    assert_eq!(across_lines.size.width, px(ADVANCE), "up to the line's end");

    let caret = bounds(ab + 2..ab + 2, cx).unwrap();
    assert_eq!(caret.left() - origin.x, px(2. * ADVANCE));
    assert_eq!(caret.size.width, px(0.));
    assert_eq!(bounds(ab + 4..ab + 1, cx), Some(across_lines), "reversed");
}

/// Emoji ZWJ sequences, flags, stacked combining marks, CJK without spaces, tabs and a word wider
/// than the column, in one wrapped paragraph.
const ADVERSARIAL: &str = "👨‍👩‍👧 fam e\u{301}\u{302}\u{303} 日本語の文章を書きます。🇺🇸🇩🇪\tTab\t\there \
     xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx \
     ok 👍🏽 e\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\u{301}\n\
     ã\u{303}\u{303}\n\t日本";

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

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
fn ime_queries_never_panic_inside_graphemes_or_out_of_bounds(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, ADVERSARIAL);
    let utf16_len = utf16_len(ADVERSARIAL);
    let origin = char_bounds(&editor, 0, cx).origin;

    editor.update_in(cx, |editor, window, cx| {
        for start in 0..utf16_len + 2 {
            for end in [start, start + 1, start + 3, 0, utf16_len + 5] {
                let bounds = editor
                    .bounds_for_range(start..end, Bounds::default(), window, cx)
                    .expect("the whole document is visible");
                assert!(bounds.left() >= origin.x && bounds.right() <= origin.x + px(680.));
                let mut adjusted = None;
                editor.text_for_range(start..end, &mut adjusted, window, cx);
            }
            let point = point(
                origin.x + px(start as f32 * 3.),
                origin.y + px(start as f32),
            );
            let index = editor.character_index_for_point(point, window, cx).unwrap();
            assert!(index <= utf16_len);
        }
    });
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

#[gpui::test]
fn layouts_follow_edits_and_renumbered_lines(cx: &mut TestAppContext) {
    let long = "word ".repeat(20); // 100 chars: two rows
    let (editor, cx) = open_editor(cx, &format!("ab\n{long}\nx"));
    let origin = char_bounds(&editor, 0, cx).origin;
    let row = char_bounds(&editor, 3, cx).origin.y - origin.y;

    cx.simulate_keystrokes("end");
    cx.simulate_input("123");
    let three = char_bounds(&editor, 4, cx).origin;
    assert_eq!(three - origin, point(px(4. * ADVANCE), px(0.)));

    // A new first line moves the long line to line 2; its two-row layout must move with it.
    cx.simulate_keystrokes("ctrl-home enter");
    let column_80 = 1 + "ab123\n".len() + 80;
    let position = char_bounds(&editor, column_80, cx).origin;
    assert_eq!(position - origin, point(px(5. * ADVANCE), row * 3.));
}
