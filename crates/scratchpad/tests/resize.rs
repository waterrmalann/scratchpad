mod common;

use gpui::{Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext, point, px};
use scratchpad::sidebar::{MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH};

/// Drags the sidebar's right edge horizontally to `x` and returns the sidebar's new width.
fn drag_edge_to(x: Pixels, cx: &mut VisualTestContext) -> Pixels {
    let start = common::center_of("sidebar-resize-handle", cx);
    let left = Some(MouseButton::Left);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    // The first move past GPUI's drag threshold starts the drag; later moves resize.
    cx.simulate_mouse_move(point(start.x + px(10.), start.y), left, Modifiers::none());
    cx.simulate_mouse_move(point(x, start.y + px(40.)), left, Modifiers::none());
    cx.simulate_mouse_up(
        point(x, start.y + px(40.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.debug_bounds("sidebar").unwrap().size.width
}

#[gpui::test]
fn dragging_the_edge_resizes_the_sidebar_within_limits(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    assert_eq!(drag_edge_to(px(400.), cx), px(400.));
    let editor = cx.debug_bounds("editor-pane").unwrap();
    assert_eq!(editor.left(), px(400.));

    assert_eq!(drag_edge_to(px(1000.), cx), MAX_SIDEBAR_WIDTH);
    assert_eq!(drag_edge_to(px(20.), cx), MIN_SIDEBAR_WIDTH);
    // The integration reads the width to remember it.
    let width = root.read_with(cx, |root, cx| root.sidebar().read(cx).width());
    assert_eq!(width, MIN_SIDEBAR_WIDTH);
}
