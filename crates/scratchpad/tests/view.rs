//! End-to-end tests of how the note is shown: zoom and the width of the text.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size. The window is
//! 1100 × 720, so the editor pane is 840 px wide. The text starts 42 px (2.8 em) right of
//! the pane's left edge and ends 24 px left of its right edge.

mod common;

use gpui::{
    Bounds, Entity, EntityInputHandler, Modifiers, Pixels, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, point, px, size,
};
use scratchpad::editor_view::EditorView;
use scratchpad::settings::SAVE_DELAY;
use scratchpad_core::Config;

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

fn zoom(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> u16 {
    editor.read_with(cx, |editor, _| editor.zoom_percent())
}

fn cursor(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| editor.editor().selection().head.0)
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

fn wheel(delta: ScrollDelta, modifiers: Modifiers, cx: &mut VisualTestContext) {
    let position = pane(cx).center();
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta,
        modifiers,
        touch_phase: TouchPhase::Moved,
    });
}

fn numbered_lines(count: usize) -> String {
    (0..count).map(|i| format!("line {i}\n")).collect()
}

#[gpui::test]
fn ctrl_plus_minus_and_zero_scale_the_text_within_limits(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "hello");
    let size_at = |cx: &mut VisualTestContext| char_bounds(&editor, 0, cx).size;
    assert_eq!(size_at(cx), size(px(ADVANCE), px(15. * 1.5)));

    cx.simulate_keystrokes("ctrl-=");
    assert_eq!(zoom(&editor, cx), 110);
    let zoomed = size_at(cx);
    assert!(
        (zoomed.width - px(ADVANCE * 1.1)).abs() < px(0.01),
        "{zoomed:?}"
    );
    assert!(
        (zoomed.height - px(15. * 1.1 * 1.5)).abs() < px(0.01),
        "{zoomed:?}"
    );

    // Ctrl+Shift+= and Ctrl+numpad-plus arrive as `ctrl-+` on Windows.
    cx.simulate_keystrokes("ctrl-+");
    assert_eq!(zoom(&editor, cx), 120);
    cx.simulate_keystrokes("ctrl--");
    assert_eq!(zoom(&editor, cx), 110);
    cx.simulate_keystrokes("ctrl-0");
    assert_eq!(zoom(&editor, cx), 100);
    assert_eq!(size_at(cx), size(px(ADVANCE), px(15. * 1.5)));

    for _ in 0..40 {
        cx.simulate_keystrokes("ctrl-=");
    }
    assert_eq!(zoom(&editor, cx), 400);
    assert_eq!(size_at(cx), size(px(4. * ADVANCE), px(4. * 15. * 1.5)));
    for _ in 0..40 {
        cx.simulate_keystrokes("ctrl--");
    }
    assert_eq!(zoom(&editor, cx), 50);
    assert_eq!(size_at(cx), size(px(ADVANCE / 2.), px(15. * 1.5 / 2.)));
}

#[gpui::test]
fn a_zoom_off_the_steps_moves_to_the_nearest_step_each_way(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "hello");
    editor.update(cx, |editor, cx| editor.set_zoom_percent(155, cx));
    cx.simulate_keystrokes("ctrl--");
    assert_eq!(zoom(&editor, cx), 150);
    editor.update(cx, |editor, cx| editor.set_zoom_percent(155, cx));
    cx.simulate_keystrokes("ctrl-=");
    assert_eq!(zoom(&editor, cx), 160);
}

#[gpui::test]
fn ctrl_wheel_zooms_by_notches_and_the_plain_wheel_still_scrolls(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(500));
    let ctrl = Modifiers::control();
    let visible = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.visible_lines());

    // A mouse wheel notch is three lines on Windows; forward zooms in.
    wheel(ScrollDelta::Lines(point(0., 3.)), ctrl, cx);
    assert_eq!(zoom(&editor, cx), 110);
    wheel(ScrollDelta::Lines(point(0., -3.)), ctrl, cx);
    wheel(ScrollDelta::Lines(point(0., -3.)), ctrl, cx);
    assert_eq!(zoom(&editor, cx), 90);
    assert_eq!(visible(cx).start, 0, "zooming does not scroll");

    // A touchpad's small deltas add up to a notch: 5 px is 2/9 of a 22.5 px line.
    for _ in 0..13 {
        wheel(ScrollDelta::Pixels(point(px(0.), px(5.))), ctrl, cx);
    }
    assert_eq!(zoom(&editor, cx), 90, "13 × 5 px is less than a notch");
    wheel(ScrollDelta::Pixels(point(px(0.), px(5.))), ctrl, cx);
    assert_eq!(zoom(&editor, cx), 100);

    wheel(
        ScrollDelta::Pixels(point(px(0.), px(-450.))),
        Modifiers::none(),
        cx,
    );
    assert!((15..25).contains(&visible(cx).start), "{:?}", visible(cx));
    assert_eq!(zoom(&editor, cx), 100);
}

#[gpui::test]
fn zooming_keeps_the_cursor_row_where_it_is_on_screen(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, &numbered_lines(500));
    // Line 20, in the lower part of the viewport.
    for _ in 0..20 {
        cx.simulate_keystrokes("down");
    }
    let offset = cursor(&editor, cx);
    let top = bounds(&editor, offset..offset, cx).top();
    assert!(top > pane(cx).top() + px(400.), "{top:?}");

    // At 150% the line would be about 225 px further down if the top line stayed.
    for _ in 0..5 {
        cx.simulate_keystrokes("ctrl-=");
    }
    assert_eq!(zoom(&editor, cx), 150);
    let caret = bounds(&editor, offset..offset, cx);
    assert!(
        (caret.top() - top).abs() < px(1.),
        "{caret:?} moved from {top:?}"
    );
    assert!((caret.size.height - px(15. * 1.5 * 1.5)).abs() < px(0.01));

    cx.simulate_keystrokes("ctrl-0");
    let caret = bounds(&editor, offset..offset, cx);
    assert!(
        (caret.top() - top).abs() < px(1.),
        "{caret:?} moved from {top:?}"
    );
}

#[gpui::test]
fn the_zoom_is_remembered(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();

    let (root, cx) = common::open_with(storage.clone(), cx);
    let editor = common::editor(&root, cx);
    assert_eq!(zoom(&editor, cx), 100);
    for _ in 0..5 {
        cx.simulate_keystrokes("ctrl-=");
    }
    common::wait(SAVE_DELAY, cx);
    let saved = Config::load(&config_path);
    assert_eq!(saved.zoom_percent, Some(150));
    common::close(cx);

    let (root, cx) = common::open_with(storage.clone(), cx);
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text("a", cx));
    cx.run_until_parked();
    assert_eq!(zoom(&editor, cx), 150);
    let height = char_bounds(&editor, 0, cx).size.height;
    assert!(
        (height - px(15. * 1.5 * 1.5)).abs() < px(0.01),
        "{height:?}"
    );
    common::close(cx);

    // A zoom out of range, e.g. edited by hand, is brought within the limits.
    Config {
        zoom_percent: Some(1000),
        ..Config::default()
    }
    .save(&config_path)
    .unwrap();
    let (root, cx) = common::open_with(storage, cx);
    let editor = common::editor(&root, cx);
    assert_eq!(zoom(&editor, cx), 400);
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
fn heading_markers_hang_in_the_margin_unclipped_at_every_zoom(cx: &mut TestAppContext) {
    // `# ` is 2.16 em of body text in the test font: it fits the 2.8 em margin. `### ` is 3 em
    // and hangs as far as the margin allows.
    let (editor, cx) = open_editor(cx, "body\n# One\n### Three");
    let pane_left = pane(cx).left();
    for percent in [50, 100, 200, 400] {
        editor.update(cx, |editor, cx| editor.set_zoom_percent(percent, cx));
        // On a heading, its markers are shown.
        cx.simulate_keystrokes("ctrl-end");
        let body = char_bounds(&editor, 0, cx);
        let hash = char_bounds(&editor, 11, cx);
        assert!(hash.left() >= pane_left, "{percent}%: {hash:?} is cut off");
        assert!(
            hash.left() < body.left(),
            "{percent}%: the markers hang left of the text"
        );

        cx.simulate_keystrokes("up");
        let hash = char_bounds(&editor, 5, cx);
        let title = char_bounds(&editor, 7, cx);
        assert!(hash.left() >= pane_left, "{percent}%: {hash:?} is cut off");
        assert_eq!(
            title.left(),
            body.left(),
            "{percent}%: the H1's text stays in line with the body text"
        );
    }
}
