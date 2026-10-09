//! End-to-end tests of Go to line (Ctrl+G, ADR 0141).

mod common;

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext, px, size};
use scratchpad::AppWindow;
use scratchpad::editor_view::EditorView;

struct GoTo<'a> {
    root: Entity<AppWindow>,
    editor: Entity<EditorView>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> GoTo<'a> {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    GoTo { root, editor, cx }
}

/// `count` lines: "line 1", "line 2", ...
fn numbered_lines(count: usize) -> String {
    (1..=count)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl GoTo<'_> {
    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
    }

    fn is_open(&mut self) -> bool {
        self.root.read_with(self.cx, |root, cx| {
            root.editor_pane().read(cx).go_to_line_box().is_some()
        })
    }

    /// The cursor's line and column (0-based), which must be a cursor, not a selection.
    fn cursor(&mut self) -> (usize, usize) {
        self.editor.read_with(self.cx, |editor, _| {
            let editor = editor.editor();
            let selection = editor.selection();
            assert!(selection.is_empty(), "{selection:?}");
            let point = editor.buffer().offset_to_point(selection.head);
            (point.line, point.column)
        })
    }

    fn visible_lines(&mut self) -> std::ops::Range<usize> {
        self.editor
            .read_with(self.cx, |editor, _| editor.visible_lines())
    }

    fn editor_focused(&mut self) -> bool {
        let focus = self
            .editor
            .read_with(self.cx, |editor, cx| editor.focus_handle(cx));
        self.cx.update(|window, _| focus.is_focused(window))
    }
}

#[gpui::test]
fn ctrl_g_goes_to_the_start_of_the_line_typed(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    go.keys("ctrl-end ctrl-g");
    assert!(go.is_open());
    assert!(!go.editor_focused());
    // The current line's number is selected, so typing replaces it.
    go.cx.simulate_input("5");
    go.keys("enter");
    assert!(!go.is_open());
    assert!(go.editor_focused());
    assert_eq!(go.cursor(), (4, 0));
}

#[gpui::test]
fn the_line_is_put_in_the_middle_of_the_view(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(2000));
    assert_eq!(go.visible_lines().start, 0);
    go.keys("ctrl-g");
    go.cx.simulate_input("1500");
    go.keys("enter");
    assert_eq!(go.cursor(), (1499, 0));
    let visible = go.visible_lines();
    let (above, below) = (1499 - visible.start, visible.end - 1 - 1499);
    assert!(above.abs_diff(below) <= 1, "{visible:?}");

    // A line already in view does not move the view.
    go.keys("ctrl-g");
    go.cx.simulate_input(&(visible.start + 11).to_string());
    go.keys("enter");
    assert_eq!(go.cursor(), (visible.start + 10, 0));
    assert_eq!(go.visible_lines(), visible);
}

#[gpui::test]
fn the_field_starts_with_the_cursors_line(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    go.keys("down down right right ctrl-g enter");
    assert_eq!(go.cursor(), (2, 0));
}

#[gpui::test]
fn numbers_past_the_end_go_to_the_last_line_and_only_digits_are_typed(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    go.keys("ctrl-g");
    go.cx.simulate_input("99999999999999999999999");
    go.keys("enter");
    assert_eq!(go.cursor(), (9, 0));

    go.keys("ctrl-g");
    go.cx.simulate_input("0");
    go.keys("enter");
    assert_eq!(go.cursor(), (0, 0), "0 is the first line");

    go.keys("ctrl-g");
    go.cx.simulate_input("x7-");
    go.keys("enter");
    assert_eq!(go.cursor(), (6, 0));
}

#[gpui::test]
fn escape_cancel_and_an_empty_field_leave_the_cursor_alone(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    go.keys("down right ctrl-g");
    go.cx.simulate_input("8");
    go.keys("escape");
    assert!(!go.is_open());
    assert!(go.editor_focused());
    assert_eq!(go.cursor(), (1, 1));

    go.keys("ctrl-g backspace enter");
    assert!(go.is_open(), "waits for a number");
    common::click("go-to-line-cancel", go.cx);
    assert!(!go.is_open());
    assert!(go.editor_focused());
    assert_eq!(go.cursor(), (1, 1));
}

#[gpui::test]
fn the_box_closes_when_focus_moves_elsewhere(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    go.keys("ctrl-g");
    go.keys("ctrl-f");
    assert!(!go.is_open());

    // Ctrl+G also works from the note list's search.
    go.keys("escape ctrl-p ctrl-g");
    assert!(go.is_open());
    go.cx.simulate_input("3");
    common::click("go-to-line-go", go.cx);
    assert_eq!(go.cursor(), (2, 0));
    assert!(go.editor_focused());
}

#[gpui::test]
fn the_box_fits_in_a_narrow_editor(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10_000));
    go.cx.simulate_resize(size(px(560.), px(400.)));
    go.keys("ctrl-g");
    let pane = go.cx.debug_bounds("editor-pane").unwrap();
    for part in ["go-to-line", "go-to-line-cancel"] {
        let bounds = go.cx.debug_bounds(part).unwrap();
        assert!(
            pane.left() < bounds.left() && bounds.right() < pane.right(),
            "{part} {bounds:?} is not inside {pane:?}"
        );
    }
}

#[gpui::test]
fn escape_closes_the_box_before_the_find_bar(cx: &mut TestAppContext) {
    let mut go = open(cx, &numbered_lines(10));
    let find_open = |go: &mut GoTo| {
        go.root.read_with(go.cx, |root, cx| {
            root.editor_pane().read(cx).find_bar().read(cx).is_open()
        })
    };
    go.keys("ctrl-f");
    go.cx.simulate_input("line");
    // Asking again starts afresh rather than closing the box.
    go.keys("ctrl-g ctrl-g");
    assert!(go.is_open());

    go.keys("escape");
    assert!(!go.is_open());
    assert!(find_open(&mut go));
    assert!(go.editor_focused());
    go.keys("escape");
    assert!(!find_open(&mut go));

    go.keys("ctrl-g ctrl-g");
    go.cx.simulate_input("3");
    go.keys("enter");
    assert_eq!(go.cursor(), (2, 0));
}
