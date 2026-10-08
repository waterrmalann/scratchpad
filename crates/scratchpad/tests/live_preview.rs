//! End-to-end tests of Markdown rendering and live preview in the editor (PLAN §18–20, §36, §47–50).
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size, 16.2 px in a
//! 27 px H1. Positions are read back through the IME bounds query, which reports displayed positions.

mod common;

use std::time::Duration;

use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Point, TestAppContext, VisualTestContext, point, px, size,
};
use scratchpad::editor_view::EditorView;

const ADVANCE: f32 = 9.;

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

/// Screen bounds of the text at UTF-16 `range` (the first row of it), as the IME sees them.
fn bounds(
    editor: &Entity<EditorView>,
    range: std::ops::Range<usize>,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    editor.update_in(cx, |editor, window, cx| {
        editor
            .bounds_for_range(range, Bounds::default(), window, cx)
            .expect("range is visible")
    })
}

/// x of the character at `offset` (ASCII text) relative to the left edge of the text column,
/// taken as where the line starting at `line_start` begins.
fn x_of(
    editor: &Entity<EditorView>,
    offset: usize,
    line_start: usize,
    cx: &mut VisualTestContext,
) -> f32 {
    let left = bounds(editor, line_start..line_start, cx).left();
    (bounds(editor, offset..offset + 1, cx).left() - left).into()
}

fn click(position: Point<Pixels>, modifiers: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers,
        button: MouseButton::Left,
        click_count: 1,
    });
}

/// Pixels to hundredths, to compare positions computed from scaled font sizes.
fn round(pixels: Pixels) -> f32 {
    (f32::from(pixels) * 100.).round() / 100.
}

fn shaped(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> u64 {
    editor.read_with(cx, |editor, _| editor.shaped_line_count())
}

#[gpui::test]
fn a_typed_heading_is_set_large_and_its_marker_hangs_in_the_margin(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("# Title");
    // The cursor is on the heading: `# ` is shown, in the margin left of the text.
    let title = bounds(&editor, 2..3, cx);
    assert_eq!(
        round(title.size.width),
        0.6 * 27.,
        "H1 is 1.8 × the body size"
    );
    let hash = bounds(&editor, 0..1, cx);
    assert_eq!(round(title.left() - hash.left()), 2. * 0.6 * 27.);

    cx.simulate_keystrokes("enter");
    cx.simulate_input("body");
    let body = bounds(&editor, 8..9, cx);
    assert_eq!(body.size.width, px(ADVANCE));
    assert_eq!(
        bounds(&editor, 2..3, cx).left(),
        body.left(),
        "with `# ` hidden the title starts where body text does"
    );
    assert_eq!(
        title.left(),
        body.left(),
        "so revealing `# ` did not move it"
    );
    assert_eq!(
        round(title.size.height),
        27. * 1.3,
        "a heading's rows fit its size"
    );
    assert_eq!(body.size.height, px(15. * 1.5));
}

#[gpui::test]
fn markers_hide_away_from_the_cursor_and_show_when_it_touches_the_span(cx: &mut TestAppContext) {
    // "x **bold** y" displays as "x bold y" unless the cursor touches the strong span (2..10).
    let (editor, cx) = open_editor(cx, "x **bold** y\nnext");
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(x_of(&editor, 4, 0, cx), 2. * ADVANCE, "`**` take no space");
    assert_eq!(x_of(&editor, 11, 0, cx), 7. * ADVANCE);

    cx.simulate_keystrokes("ctrl-home right");
    assert_eq!(
        x_of(&editor, 4, 0, cx),
        2. * ADVANCE,
        "next to the span, not touching"
    );
    cx.simulate_keystrokes("right");
    assert_eq!(cursor(&editor, cx), 2);
    assert_eq!(
        x_of(&editor, 4, 0, cx),
        4. * ADVANCE,
        "touching: `**` are shown"
    );
    assert_eq!(x_of(&editor, 11, 0, cx), 11. * ADVANCE);

    // Source mode shows every marker wherever the cursor is.
    cx.simulate_keystrokes("ctrl-end ctrl-/");
    assert!(editor.read_with(cx, |editor, _| editor.source_mode()));
    assert_eq!(x_of(&editor, 4, 0, cx), 4. * ADVANCE);
    cx.simulate_keystrokes("ctrl-/");
    assert_eq!(x_of(&editor, 4, 0, cx), 2. * ADVANCE);
}

#[gpui::test]
fn clicking_text_with_hidden_markers_puts_the_cursor_on_the_clicked_character(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(cx, "x **bold** [link](https://example.com) y\nnext");
    cx.simulate_keystrokes("ctrl-end");
    let line = bounds(&editor, 0..0, cx);
    let at_display = |column: f32| point(line.left() + px(column * ADVANCE + 2.), line.center().y);

    // Displayed "x bold link y": "o" is display column 3 and buffer offset 5.
    click(at_display(3.), Modifiers::none(), cx);
    assert_eq!(cursor(&editor, cx), 5);
    // Before "b": after the hidden `**`, in the bold text.
    cx.simulate_keystrokes("ctrl-end");
    click(at_display(2.), Modifiers::none(), cx);
    assert_eq!(cursor(&editor, cx), 4);
    // "n" of "link" (display column 9) is offset 14, after the hidden `[`.
    cx.simulate_keystrokes("ctrl-end");
    click(at_display(9.), Modifiers::none(), cx);
    assert_eq!(cursor(&editor, cx), 14);
}

#[gpui::test]
fn left_and_right_step_through_markers_one_character_at_a_time(cx: &mut TestAppContext) {
    let text = "a **b** `c` d";
    let (editor, cx) = open_editor(cx, text);
    cx.simulate_keystrokes("end");
    for expected in (0..text.len()).rev() {
        cx.simulate_keystrokes("left");
        assert_eq!(cursor(&editor, cx), expected);
    }
    for expected in 1..=text.len() {
        cx.simulate_keystrokes("right");
        assert_eq!(cursor(&editor, cx), expected);
    }
}

#[gpui::test]
fn ime_bounds_and_selections_use_displayed_positions(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "x **bold** y\nnext");
    cx.simulate_keystrokes("ctrl-end");
    let left = bounds(&editor, 0..0, cx).left();

    let ol = bounds(&editor, 5..7, cx);
    assert_eq!(ol.left() - left, px(3. * ADVANCE));
    assert_eq!(ol.size.width, px(2. * ADVANCE));
    // " y" after the hidden closing `**`.
    let y = bounds(&editor, 10..12, cx);
    assert_eq!(y.left() - left, px(6. * ADVANCE));
}

#[gpui::test]
fn moving_the_cursor_and_blinking_reshape_only_lines_whose_markers_change(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "plain one\nplain two\n**bold** three\nplain four");
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let before = shaped(&editor, cx);

    cx.simulate_keystrokes("down");
    cx.executor().advance_clock(Duration::from_millis(1_200));
    cx.run_until_parked();
    assert_eq!(
        shaped(&editor, cx),
        before,
        "moving between plain lines and blinking"
    );

    cx.simulate_keystrokes("down");
    assert_eq!(
        shaped(&editor, cx),
        before + 1,
        "entering the bold line reveals its markers"
    );
    cx.simulate_keystrokes("right");
    assert_eq!(
        shaped(&editor, cx),
        before + 1,
        "still in the span: nothing changes"
    );
    cx.simulate_keystrokes("down");
    assert_eq!(
        shaped(&editor, cx),
        before + 2,
        "leaving it hides them again"
    );
}
