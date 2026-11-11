//! End-to-end tests of GFM tables in live preview (ADR 0186): the aligned grid away from the cursor,
//! the source as typed with the cursor in the table, clicks, wide tables and when widths are measured.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size, bold included.
//! Positions are read back through the IME bounds query, which reports displayed positions.

mod common;

use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Point, TestAppContext, VisualTestContext, point, px, size,
};
use scratchpad::editor_view::EditorView;

const ADVANCE: f32 = 9.;
/// Room each side of a cell's text: 0.6 em, rounded.
const PADDING: f32 = 9.;

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

fn cursor(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| editor.editor().selection().head.0)
}

fn measured(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> u64 {
    editor.read_with(cx, |editor, _| editor.measured_table_count())
}

/// Screen bounds of the char at `offset` (ASCII text), as the IME sees them.
fn bounds(
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

/// x of the char at `offset` from the left edge of the text column, where the note's first line
/// (`x`, unindented) starts.
fn x_of(editor: &Entity<EditorView>, offset: usize, cx: &mut VisualTestContext) -> f32 {
    (bounds(editor, offset, cx).left() - bounds(editor, 0, cx).left()).into()
}

fn click(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 1,
    });
}

/// Offset of the first `needle` in `text`.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle).expect("needle in text")
}

const TABLE: &str = "x\n\n| a | bbb |\n|:-:|--:|\n| cc | dd |\n\nafter";

#[gpui::test]
fn away_from_the_cursor_a_table_is_an_aligned_grid(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, TABLE);
    // Column widths are the widest text: 2 and 3 chars. Each column has 9 px of padding each side,
    // so column 1 starts at 2 × 9 + 18 = 36 px.
    let (col0, col1) = (2. * ADVANCE, 3. * ADVANCE);
    let col1_start = col0 + 2. * PADDING;
    // Column 0 is centred: `a` gets half of the 9 px it lacks (rounded), `cc` fills it.
    assert_eq!(x_of(&editor, at(TABLE, "a |"), cx), PADDING + 5.);
    assert_eq!(x_of(&editor, at(TABLE, "cc"), cx), PADDING);
    // Column 1 is right-aligned.
    assert_eq!(x_of(&editor, at(TABLE, "bbb"), cx), col1_start + PADDING);
    assert_eq!(
        x_of(&editor, at(TABLE, "dd"), cx),
        col1_start + PADDING + col1 - 2. * ADVANCE
    );
    // The delimiter row is a short row with a rule: the body row is 1.5 em + 0.5 em below the
    // header.
    let header = bounds(&editor, at(TABLE, "a |"), cx);
    let body = bounds(&editor, at(TABLE, "cc"), cx);
    assert_eq!(body.top() - header.top(), px(15. * 1.5 + 8.));
    // Nothing is inserted into the text.
    let text = editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text());
    assert_eq!(text, TABLE);
}

#[gpui::test]
fn clicking_a_cell_puts_the_caret_there_and_shows_the_table_as_typed(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, TABLE);
    let dd = at(TABLE, "dd");
    let grid = bounds(&editor, dd, cx);
    click(point(grid.left() + px(2.), grid.center().y), cx);
    assert_eq!(cursor(&editor, cx), dd, "the char clicked in the grid");
    // The cursor is in the table: every row shows as typed, pipes included.
    let row_start = at(TABLE, "| cc");
    assert_eq!(x_of(&editor, dd, cx), (dd - row_start) as f32 * ADVANCE);
    assert_eq!(x_of(&editor, at(TABLE, "bbb"), cx), 6. * ADVANCE);
    let delimiter = at(TABLE, "|:-:");
    assert_eq!(x_of(&editor, delimiter + 1, cx), ADVANCE);
    // Moving through it is moving through text: Up keeps the column.
    cx.simulate_keystrokes("up up");
    assert_eq!(cursor(&editor, cx), at(TABLE, "| a") + (dd - row_start));
    // Leaving it draws the grid again.
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(x_of(&editor, at(TABLE, "cc"), cx), PADDING);
}

#[gpui::test]
fn up_and_down_from_outside_enter_the_table_as_typed(cx: &mut TestAppContext) {
    let text = "| a | b |\n|---|---|\n| c | d |\nafter";
    let (editor, cx) = open_editor(cx, text);
    cx.simulate_keystrokes("ctrl-end");
    // The cursor is after "after" (column 5); the row above, revealed, has `|` at 4 and ` ` at 5.
    cx.simulate_keystrokes("home right right");
    cx.simulate_keystrokes("up");
    assert_eq!(cursor(&editor, cx), at(text, "| c") + 2);
    cx.simulate_keystrokes("up up");
    assert_eq!(cursor(&editor, cx), 2);
}

#[gpui::test]
fn tables_in_quotes_and_list_items_line_their_rows_up(cx: &mut TestAppContext) {
    for text in [
        "x\n\n> | a | b |\n> |---|---|\n> | c | d |",
        "x\n\n- | a | b |\n  |---|---|\n  | c | d |",
    ] {
        let (editor, cx) = open_editor(cx, text);
        assert_eq!(
            x_of(&editor, at(text, "a |"), cx),
            x_of(&editor, at(text, "c |"), cx),
            "{text:?}"
        );
        assert_eq!(
            x_of(&editor, at(text, "b |"), cx),
            x_of(&editor, at(text, "d |"), cx),
            "{text:?}"
        );
        assert!(x_of(&editor, at(text, "a |"), cx) > 0., "{text:?}");
    }
}

#[gpui::test]
fn a_table_too_wide_to_wrap_is_shown_as_typed_until_word_wrap_is_off(cx: &mut TestAppContext) {
    let wide = "w".repeat(200);
    let text = format!("x\n\n| a | {wide} |\n|---|---|\n| b | c |");
    let (editor, cx) = open_editor(cx, &text);
    let b = at(&text, "b |");
    let row_start = at(&text, "| b");
    assert_eq!(
        x_of(&editor, b, cx),
        (b - row_start) as f32 * ADVANCE,
        "as typed, pipes shown"
    );
    editor.update(cx, |editor, cx| editor.set_soft_wrap(false, cx));
    cx.run_until_parked();
    assert_eq!(x_of(&editor, b, cx), PADDING, "a grid, scrolled sideways");
}

#[gpui::test]
fn widths_are_measured_once_per_table_and_again_only_after_it_changes(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, TABLE);
    let text = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text())
    };
    assert_eq!(measured(&editor, cx), 1);
    // Typing outside the table and moving around outside it: nothing to measure.
    cx.simulate_input("yz");
    cx.simulate_keystrokes("end ctrl-end home ctrl-home");
    assert_eq!(measured(&editor, cx), 1);
    // In the table it is shown as typed, so typing there measures nothing either.
    let second_c = bounds(&editor, at(&text(cx), "cc") + 1, cx);
    click(point(second_c.left() + px(2.), second_c.center().y), cx);
    cx.simulate_input("ccc");
    assert_eq!(measured(&editor, cx), 1);
    // Leaving it measures the changed table once, and its new width lines the columns up.
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(measured(&editor, cx), 2);
    let col1_start = 5. * ADVANCE + 2. * PADDING;
    let bbb = at(&text(cx), "bbb");
    assert_eq!(x_of(&editor, bbb, cx), col1_start + PADDING);
    // Entering and leaving it again without a change reuses the widths.
    cx.simulate_keystrokes("down down end ctrl-home");
    assert_eq!(measured(&editor, cx), 2);
}

#[gpui::test]
fn typing_on_the_line_above_a_table_keeps_its_widths(cx: &mut TestAppContext) {
    let text = big_table(3);
    let (editor, cx) = open_editor(cx, &text);
    assert_eq!(measured(&editor, cx), 1);
    // The cursor at the end of `## Results`, the line right above the header.
    cx.simulate_keystrokes("ctrl-home down down end");
    cx.simulate_input("ab");
    cx.simulate_keystrokes("backspace");
    assert_eq!(measured(&editor, cx), 1);
    // A line added above the table moves it down; its widths move with it.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("more");
    assert_eq!(measured(&editor, cx), 1);
    assert_eq!(
        x_of(&editor, at(&editor_text(&editor, cx), "row 0"), cx),
        PADDING
    );
    // Typing in the header row changes the table.
    cx.simulate_keystrokes("down home right right");
    cx.simulate_input("z");
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(measured(&editor, cx), 2);
}

#[gpui::test]
fn resizing_the_window_keeps_the_widths_and_zooming_measures_again(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, TABLE);
    assert_eq!(measured(&editor, cx), 1);
    for width in [1000., 900., 1050.] {
        cx.simulate_resize(size(px(width), px(720.)));
        cx.run_until_parked();
    }
    assert_eq!(measured(&editor, cx), 1);
    assert_eq!(x_of(&editor, at(TABLE, "cc"), cx), PADDING);
    editor.update(cx, |editor, cx| editor.zoom_by(1, cx));
    cx.run_until_parked();
    assert_eq!(measured(&editor, cx), 2);
}

#[gpui::test]
fn a_table_too_long_for_a_grid_is_shown_as_typed_without_measuring(cx: &mut TestAppContext) {
    // 500 lines is the most a grid has (`MAX_GRID_LINES`): header, delimiter and 498 rows.
    let text = big_table(498);
    let (editor, cx) = open_editor(cx, &text);
    assert_eq!(measured(&editor, cx), 1);
    assert_eq!(x_of(&editor, at(&text, "row 0"), cx), PADDING, "a grid");
    let text = big_table(499);
    editor.update(cx, |editor, cx| editor.set_text(&text, cx));
    cx.run_until_parked();
    assert_eq!(measured(&editor, cx), 1, "not measured");
    let row = at(&text, "| row 0");
    assert_eq!(
        x_of(&editor, at(&text, "row 0"), cx),
        (at(&text, "row 0") - row) as f32 * ADVANCE,
        "as typed, pipes shown"
    );
}

fn editor_text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text())
}

/// A note with a heading right above a table of `rows` body rows.
fn big_table(rows: usize) -> String {
    let mut text = String::from("x\n\n## Results\n| name | **value** | note |\n|---|--:|:-:|\n");
    for row in 0..rows {
        let value = row * 37 % 1000;
        text.push_str(&format!("| row {row} | {value} | `code` *{row}* |\n"));
    }
    text.push_str("\nafter");
    text
}

/// Times typing in a heading touching a 400-row table, dragging the window edge with it visible and
/// opening a 5,000-row table (run with `--ignored --nocapture`, ideally in release). The counts of
/// tables measured are what the other tests check; this reports what they cost.
#[gpui::test]
#[ignore = "timing, for docs/performance.md"]
fn perf_typing_and_resizing_next_to_a_big_table(cx: &mut TestAppContext) {
    use std::time::Instant;
    let text = big_table(400);
    let (editor, cx) = open_editor(cx, &text);
    cx.simulate_keystrokes("ctrl-home down down end");
    let measured_before = measured(&editor, cx);
    let started = Instant::now();
    for _ in 0..20 {
        cx.simulate_input("y");
    }
    let typing = started.elapsed() / 20;
    let typing_measured = measured(&editor, cx) - measured_before;
    cx.simulate_keystrokes("ctrl-home");
    let measured_before = measured(&editor, cx);
    let started = Instant::now();
    for step in 0..20 {
        cx.simulate_resize(size(px(1100. - step as f32 * 7.), px(720.)));
        cx.run_until_parked();
    }
    let resizing = started.elapsed() / 20;
    let resize_measured = measured(&editor, cx) - measured_before;
    let long = big_table(5000);
    let started = Instant::now();
    editor.update(cx, |editor, cx| editor.set_text(&long, cx));
    cx.run_until_parked();
    let opening = started.elapsed();
    println!(
        "keystroke in the heading: {typing:?} ({typing_measured} measured in 20); \
         resize step: {resizing:?} ({resize_measured} measured in 20); \
         open a 5,000-row table: {opening:?}"
    );
}
