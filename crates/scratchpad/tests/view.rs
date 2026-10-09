//! End-to-end tests of how the note is shown: the width of the text.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size. The window is
//! 1100 × 720, so the editor pane is 840 px wide. The text starts 42 px (2.8 em) right of
//! the pane's left edge and ends 24 px left of its right edge.

mod common;

use gpui::{
    Bounds, Entity, EntityInputHandler, Pixels, TestAppContext, VisualTestContext, px, size,
};
use scratchpad::editor_view::EditorView;

const ADVANCE: f32 = 9.;
const LEFT_PADDING: f32 = 42.;
const RIGHT_PADDING: f32 = 24.;

fn open_editor<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (Entity<EditorView>, &'a mut VisualTestContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Screen bounds of the text at UTF-16 `range` (its first row), as the IME sees them: where the
/// text is laid out and painted. Panics if the line is not on screen.
fn bounds(
    editor: &Entity<EditorView>,
    range: std::ops::Range<usize>,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    editor.update_in(cx, |editor, window, cx| {
        editor
            .bounds_for_range(range, Bounds::default(), window, cx)
            .expect("the line is on screen")
    })
}

fn char_bounds(
    editor: &Entity<EditorView>,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    bounds(editor, offset..offset + 1, cx)
}

fn pane(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds("editor-pane").unwrap()
}

#[gpui::test]
fn the_text_starts_at_the_left_padding_and_fills_the_width(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &"word ".repeat(200));
    for window_width in [1100., 1700.] {
        cx.simulate_resize(size(px(window_width), px(720.)));
        let pane = pane(cx);
        let first = char_bounds(&editor, 0, cx);
        assert_eq!(
            first.left(),
            pane.left() + px(LEFT_PADDING),
            "{window_width}"
        );

        let row_end = (0..1000)
            .find(|&i| char_bounds(&editor, i, cx).top() > first.top())
            .unwrap();
        let last_glyph = char_bounds(&editor, row_end - 2, cx);
        let text_right = pane.right() - px(RIGHT_PADDING);
        assert!(
            last_glyph.right() <= text_right && last_glyph.right() > text_right - px(5. * ADVANCE),
            "the row ends within a word of the right padding: {last_glyph:?}, {text_right:?}"
        );
    }
}

#[gpui::test]
fn heading_markers_hang_in_the_margin_unclipped(cx: &mut TestAppContext) {
    // `# ` is 2.16 em of body text in the test font: it fits the 2.8 em margin. `### ` is 3 em
    // and hangs as far as the margin allows.
    let (editor, cx) = open_editor(cx, "body\n# One\n### Three");
    let pane_left = pane(cx).left();
    // On a heading, its markers are shown.
    cx.simulate_keystrokes("ctrl-end");
    let body = char_bounds(&editor, 0, cx);
    let hash = char_bounds(&editor, 11, cx);
    assert!(hash.left() >= pane_left, "{hash:?} is cut off");
    assert!(
        hash.left() < body.left(),
        "the markers hang left of the text"
    );

    cx.simulate_keystrokes("up");
    let hash = char_bounds(&editor, 5, cx);
    let title = char_bounds(&editor, 7, cx);
    assert!(hash.left() >= pane_left, "{hash:?} is cut off");
    assert_eq!(
        title.left(),
        body.left(),
        "the H1's text stays in line with the body text"
    );
}
