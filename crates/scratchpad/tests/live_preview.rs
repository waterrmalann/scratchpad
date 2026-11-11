//! End-to-end tests of Markdown rendering and live preview in the editor (PLAN §18–20, §36, §47–50).
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size, 16.2 px in a
//! 27 px H1. Positions are read back through the IME bounds query, which reports displayed positions.

mod common;

use std::time::Duration;

use gpui::{
    Bounds, Entity, EntityInputHandler, Focusable, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext, point, px, size,
};
use scratchpad::AppWindow;
use scratchpad::editor_view::EditorView;
use scratchpad::session::AUTOSAVE_DELAY;
use scratchpad_core::NoteEvent;

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

fn text(editor: &Entity<EditorView>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.editor().buffer().normalized_text())
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
    assert_eq!(round(body.size.height), 15. * 1.5);
}

#[gpui::test]
fn heading_markers_wider_than_a_narrow_margin_are_not_cut_off(cx: &mut TestAppContext) {
    // The text column has a 42 px margin; `###### ` is 63 px wide.
    let (editor, cx) = open_editor(cx, "body\n###### Six");
    cx.simulate_resize(size(px(560.), px(400.)));
    cx.simulate_keystrokes("ctrl-end");
    let pane_left = cx.debug_bounds("editor-pane").unwrap().left();
    let hash = bounds(&editor, 5..6, cx);
    let body = bounds(&editor, 0..1, cx);
    assert!(
        hash.left() >= pane_left,
        "{hash:?} starts left of {pane_left:?}"
    );
    assert_eq!(
        hash.left(),
        pane_left,
        "the markers hang as far as the margin allows"
    );
    assert!(body.left() > pane_left);
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
fn double_and_triple_clicks_select_around_the_first_click_although_it_revealed_markers(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(cx, "say **bold** now\nnext");
    cx.simulate_keystrokes("ctrl-end");
    let line = bounds(&editor, 0..0, cx);
    // On the "o" of "bold" in the displayed "say bold now". The first click reveals the `**`,
    // which moves "bold" two characters right, under the second click's position.
    let on_o = point(line.left() + px(5. * ADVANCE + 2.), line.center().y);
    for click_count in 1..=3 {
        cx.simulate_event(MouseDownEvent {
            position: on_o,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: on_o,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
        });
        let selected = editor.read_with(cx, |editor, _| editor.editor().selection().range());
        let expected = match click_count {
            1 => 7..7,
            2 => 6..10,
            _ => 0..17,
        };
        assert_eq!(
            selected.start.0..selected.end.0,
            expected,
            "click {click_count}"
        );
    }
}

#[gpui::test]
fn end_stays_on_its_row_when_the_row_ends_in_hidden_markers(cx: &mut TestAppContext) {
    // 86 characters fit in a row. Displayed, the first row is 16 × "word " and "abc "; with the
    // cursor touching it, "**abc**" no longer fits and moves to the second row.
    let line = format!("{}**abc** tail tail tail", "word ".repeat(16));
    let (editor, cx) = open_editor(cx, &format!("{line}\nnext"));
    let first_row_top = bounds(&editor, 0..0, cx).top();

    cx.simulate_keystrokes("ctrl-home end");
    assert_eq!(
        cursor(&editor, cx),
        79,
        "after the last word left on the row"
    );
    assert_eq!(bounds(&editor, 79..79, cx).top(), first_row_top);
    cx.simulate_keystrokes("end");
    assert_eq!(cursor(&editor, cx), 79, "End again stays");

    cx.simulate_keystrokes("ctrl-home shift-end");
    let selected = editor.read_with(cx, |editor, _| editor.editor().selection().range());
    assert_eq!(selected.start.0..selected.end.0, 0..79);
}

#[gpui::test]
fn up_and_down_keep_the_caret_x_on_lines_whose_markers_they_reveal(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(
        cx,
        "plain text line here\nsome **bold words** here\nmore plain text",
    );
    let caret_x = |cx: &mut VisualTestContext| {
        let head = cursor(&editor, cx);
        bounds(&editor, head..head, cx).left()
    };
    cx.simulate_keystrokes("ctrl-home");
    for _ in 0..10 {
        cx.simulate_keystrokes("right");
    }
    let x = caret_x(cx);

    // Column 10 of "some bold words" is before "words"; with the `**` shown it is in "bold".
    cx.simulate_keystrokes("down");
    assert_eq!(cursor(&editor, cx), 21 + 10, "some **bol|d");
    assert_eq!(caret_x(cx), x);
    cx.simulate_keystrokes("down");
    assert_eq!(caret_x(cx), x);
    cx.simulate_keystrokes("up");
    assert_eq!(caret_x(cx), x);
    cx.simulate_keystrokes("up");
    assert_eq!(cursor(&editor, cx), 10);
}

#[gpui::test]
fn home_goes_to_the_text_after_list_quote_and_heading_markers_then_to_the_line_start(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(cx, "  - item\n- [ ] task\n> quote\n## Head\nplain");
    let lines = [(0, 4), (9, 15), (20, 22), (28, 31), (36, 36)];
    for (line, (line_start, text_start)) in lines.into_iter().enumerate() {
        cx.simulate_keystrokes("ctrl-home");
        for _ in 0..line {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("end home");
        assert_eq!(cursor(&editor, cx), text_start, "line {line}");
        cx.simulate_keystrokes("home");
        assert_eq!(cursor(&editor, cx), line_start, "line {line}");
        cx.simulate_keystrokes("home");
        assert_eq!(cursor(&editor, cx), text_start, "line {line}");
    }
    cx.simulate_keystrokes("ctrl-home end shift-home");
    let selected = editor.read_with(cx, |editor, _| editor.editor().selection().range());
    assert_eq!(selected.start.0..selected.end.0, 4..8);
}

#[gpui::test]
fn ctrl_left_and_right_do_not_stop_at_hidden_markers(cx: &mut TestAppContext) {
    let text = "say **bold** now `code` end";
    let (editor, cx) = open_editor(cx, text);
    let mut stops = Vec::new();
    for _ in 0..7 {
        cx.simulate_keystrokes("ctrl-right");
        stops.push(cursor(&editor, cx));
    }
    // Hidden `**` and backticks are passed over; once the cursor reveals them they are stops.
    assert_eq!(stops, [3, 10, 12, 16, 22, 23, 27]);
    stops.clear();
    for _ in 0..7 {
        cx.simulate_keystrokes("ctrl-left");
        stops.push(cursor(&editor, cx));
    }
    assert_eq!(stops, [24, 18, 17, 13, 6, 4, 0]);

    cx.simulate_keystrokes("ctrl-shift-right ctrl-shift-right");
    let selected = editor.read_with(cx, |editor, _| editor.editor().selection().range());
    assert_eq!(selected.start.0..selected.end.0, 0..10);
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
fn formatting_shortcuts_edit_markdown_and_undo_in_one_step(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "hello");

    for (keys, formatted) in [
        ("ctrl-b", "**hello**"),
        ("ctrl-i", "*hello*"),
        ("ctrl-shift-x", "~~hello~~"),
        ("ctrl-e", "`hello`"),
        ("ctrl-k", "[hello]()"),
    ] {
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_keystrokes(keys);
        assert_eq!(text(&editor, cx), formatted, "{keys}");
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(text(&editor, cx), "hello", "{keys} undone");
    }
    cx.simulate_keystrokes("ctrl-a ctrl-b ctrl-b");
    assert_eq!(text(&editor, cx), "hello", "bold toggles off");
}

#[gpui::test]
fn enter_continues_a_list_and_ends_it_on_an_empty_item(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("- one");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "- one\n- ");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "- one\n\n");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "- one\n- ");

    cx.simulate_keystrokes("backspace backspace");
    cx.simulate_input("1. a");
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(
        text(&editor, cx),
        "- one\n1. a\n",
        "shift-enter does not continue"
    );
}

#[gpui::test]
fn tab_and_shift_tab_nest_list_items_while_typing_a_list(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("- one");
    cx.simulate_keystrokes("enter tab");
    cx.simulate_input("two");
    cx.simulate_keystrokes("enter shift-tab");
    cx.simulate_input("three");
    assert_eq!(text(&editor, cx), "- one\n  - two\n- three");

    // Outside lists Tab inserts spaces and Shift+Tab does nothing.
    cx.simulate_keystrokes("enter enter shift-tab tab");
    assert_eq!(text(&editor, cx), "- one\n  - two\n- three\n\n    ");
}

#[gpui::test]
fn typed_brackets_pair_and_backspace_removes_an_empty_pair(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "");

    cx.simulate_input("(");
    assert_eq!(text(&editor, cx), "()");
    assert_eq!(cursor(&editor, cx), 1);
    cx.simulate_input("x)");
    assert_eq!(text(&editor, cx), "(x)", "the closer steps over");
    cx.simulate_input(" [");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text(&editor, cx), "(x) ");
}

#[gpui::test]
fn typed_tildes_and_underscores_wrap_the_selection(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "say text now");
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let selection = editor.editor().selection();
            (selection.start().0, selection.end().0)
        })
    };

    cx.simulate_keystrokes("ctrl-right ctrl-right ctrl-shift-left");
    assert_eq!(selected(cx), (4, 8));
    cx.simulate_input("~");
    cx.simulate_input("~");
    assert_eq!(text(&editor, cx), "say ~~text~~ now");
    assert_eq!(selected(cx), (6, 10), "the selection stays on the text");
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(
        text(&editor, cx),
        "say ~text~ now",
        "one undo step per wrap"
    );
    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "say text now");

    cx.simulate_keystrokes("ctrl-home ctrl-right ctrl-right ctrl-shift-left");
    cx.simulate_input("_");
    assert_eq!(text(&editor, cx), "say _text_ now");

    // In a plain text file nothing wraps: the keys replace the selection.
    editor.update(cx, |editor, cx| editor.set_markdown(false, cx));
    cx.simulate_keystrokes("ctrl-home ctrl-shift-right");
    assert_eq!(selected(cx), (0, 3));
    cx.simulate_input("~");
    assert_eq!(text(&editor, cx), "~ _text_ now");
}

#[gpui::test]
fn a_typed_underscore_replaces_the_selection_in_code(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "```\nfor i in x\n```\nuse `a b` here");
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let selection = editor.editor().selection();
            (selection.start().0, selection.end().0)
        })
    };

    // `i` in the code block.
    cx.simulate_keystrokes("ctrl-home down ctrl-right ctrl-right ctrl-shift-left");
    assert_eq!(selected(cx), (8, 9));
    cx.simulate_input("_");
    assert_eq!(text(&editor, cx), "```\nfor _ in x\n```\nuse `a b` here");

    // `b` in the inline code span is replaced too, while `~` still wraps there.
    let b = "```\nfor _ in x\n```\nuse `a ".len();
    cx.simulate_keystrokes("ctrl-end ctrl-left left left shift-left");
    assert_eq!(selected(cx), (b, b + 1));
    cx.simulate_input("_");
    assert_eq!(text(&editor, cx), "```\nfor _ in x\n```\nuse `a _` here");
    cx.simulate_keystrokes("shift-left");
    cx.simulate_input("~");
    assert_eq!(text(&editor, cx), "```\nfor _ in x\n```\nuse `a ~_~` here");
}

#[gpui::test]
fn clicking_a_task_box_toggles_it_without_moving_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "- [ ] task\n- [x] done\nnext");
    cx.simulate_keystrokes("ctrl-end");
    let end = cursor(&editor, cx);
    let task = bounds(&editor, 0..0, cx);
    let done = bounds(&editor, 11..11, cx);

    click(
        point(task.left() + px(7.), task.center().y),
        Modifiers::none(),
        cx,
    );
    assert_eq!(text(&editor, cx), "- [x] task\n- [x] done\nnext");
    assert_eq!(cursor(&editor, cx), end);
    click(
        point(done.left() + px(7.), done.center().y),
        Modifiers::none(),
        cx,
    );
    assert_eq!(text(&editor, cx), "- [x] task\n- [ ] done\nnext");

    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&editor, cx), "- [x] task\n- [x] done\nnext");
    // Clicking the item's text places the cursor as usual.
    click(
        point(task.left() + px(40.), task.center().y),
        Modifiers::none(),
        cx,
    );
    assert_eq!(text(&editor, cx), "- [x] task\n- [x] done\nnext");
    assert!(cursor(&editor, cx) < 10);
}

#[gpui::test]
fn ctrl_click_opens_web_links_but_never_other_schemes(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(
        cx,
        "[site](https://example.com) [js](javascript:alert(1)) [exe](file:///C:/x.exe)\nnext",
    );
    cx.simulate_keystrokes("ctrl-end");
    let end = cursor(&editor, cx);
    let line = bounds(&editor, 0..0, cx);
    // Displayed "site js exe".
    let at_display = |column: f32| point(line.left() + px(column * ADVANCE + 4.), line.center().y);

    click(at_display(1.), Modifiers::none(), cx);
    assert_eq!(cx.opened_url(), None, "a plain click edits");
    assert_eq!(cursor(&editor, cx), 2);

    cx.simulate_keystrokes("ctrl-end");
    click(at_display(1.), Modifiers::secondary_key(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    assert_eq!(
        cursor(&editor, cx),
        end,
        "opening a link does not move the cursor"
    );

    for column in [5., 8.] {
        click(at_display(column), Modifiers::secondary_key(), cx);
        assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    }
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

// --- With the note session: formatting is saved, reloads restyle, read-only notes stay put ---

struct Note {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    root: Entity<AppWindow>,
    editor: Entity<EditorView>,
}

/// Opens the app on a folder holding one note, `Ideas.md` with `contents`, sized like
/// `open_editor`.
fn open_note<'a>(cx: &'a mut TestAppContext, contents: &[u8]) -> (Note, &'a mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Ideas.md");
    std::fs::write(&path, contents).unwrap();
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    cx.run_until_parked();
    let note = Note {
        _dir: dir,
        path,
        root,
        editor,
    };
    (note, cx)
}

fn on_disk(note: &Note) -> String {
    String::from_utf8_lossy(&std::fs::read(&note.path).unwrap()).into_owned()
}

/// The middle of the task box drawn at the start of the line starting at UTF-16 offset
/// `line_start`.
fn task_box(
    editor: &Entity<EditorView>,
    line_start: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let line = bounds(editor, line_start..line_start, cx);
    point(line.left() + px(7.), line.center().y)
}

/// A note written from scratch with the keyboard only, as a user would, then checked on disk.
#[gpui::test]
fn writing_a_note_from_scratch_by_keyboard(cx: &mut TestAppContext) {
    let (note, cx) = open_note(cx, b"");

    cx.simulate_input("# Trip");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Pack ");
    cx.simulate_keystrokes("ctrl-b");
    cx.simulate_input("light");
    cx.simulate_keystrokes("ctrl-b");
    cx.simulate_input(", see ");
    cx.simulate_keystrokes("ctrl-k");
    cx.simulate_input("list");
    cx.simulate_keystrokes("right right");
    cx.simulate_input("https://example.com");
    cx.simulate_keystrokes("end enter");
    cx.simulate_input("- [ ] tickets");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("train");
    cx.simulate_keystrokes("enter tab");
    cx.simulate_input("seats");
    cx.simulate_keystrokes("enter shift-tab");
    cx.simulate_input("hotel");
    cx.simulate_keystrokes("enter enter");
    cx.simulate_input("> done");

    let expected = "# Trip\nPack **light**, see [list](https://example.com)\n- [ ] tickets\n\
                    - [ ] train\n  - [ ] seats\n- [ ] hotel\n\n> done";
    assert_eq!(text(&note.editor, cx), expected);
    common::wait(AUTOSAVE_DELAY, cx);
    assert_eq!(on_disk(&note), expected);

    // Selecting what is shown copies the Markdown behind it.
    cx.simulate_keystrokes("ctrl-home down home shift-end ctrl-c");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Pack **light**, see [list](https://example.com)")
    );
}

#[gpui::test]
fn formatting_shortcuts_and_task_clicks_are_saved_like_typing(cx: &mut TestAppContext) {
    let (note, cx) = open_note(cx, b"Ideas\n- [ ] call\nfirst");
    cx.simulate_keystrokes("ctrl-end ctrl-shift-left ctrl-b");
    common::wait(AUTOSAVE_DELAY, cx);
    assert_eq!(on_disk(&note), "Ideas\n- [ ] call\n**first**");

    click(task_box(&note.editor, 6, cx), Modifiers::none(), cx);
    common::wait(AUTOSAVE_DELAY, cx);
    assert_eq!(on_disk(&note), "Ideas\n- [x] call\n**first**");
}

#[gpui::test]
fn a_note_reloaded_after_an_outside_change_is_styled_from_its_new_text(cx: &mut TestAppContext) {
    let (note, cx) = open_note(cx, b"Ideas\nplain");
    // As long as the old text, so Markdown structure left over from it would look current.
    std::fs::write(&note.path, "Ideas\n## Hi").unwrap();
    let session = common::session(&note.root, cx);
    session.update(cx, |session, cx| {
        session.disk_events(vec![NoteEvent::Changed(note.path.clone())], cx)
    });
    cx.run_until_parked();
    assert_eq!(text(&note.editor, cx), "Ideas\n## Hi");

    // The cursor is on the first line: `## ` is hidden and "Hi" is set as an H2.
    let h = bounds(&note.editor, 9..10, cx);
    assert_eq!(round(h.size.width), 0.6 * 15. * 1.5);
    assert_eq!(h.left(), bounds(&note.editor, 6..6, cx).left());
}

#[gpui::test]
fn a_read_only_note_ignores_task_clicks_and_markdown_commands(cx: &mut TestAppContext) {
    // Not UTF-8, so it opens read-only until the user agrees to edit it.
    let contents = b"Caf\xE9\n- [ ] task\nword";
    let (note, cx) = open_note(cx, contents);
    assert!(note.editor.read_with(cx, |editor, _| editor.is_read_only()));
    let shown = text(&note.editor, cx);
    // The task line starts after "Caf", U+FFFD (one UTF-16 unit, three bytes) and the break.
    click(task_box(&note.editor, 5, cx), Modifiers::none(), cx);
    assert_eq!(text(&note.editor, cx), shown, "the box is not toggled");
    assert_eq!(
        cursor(&note.editor, cx),
        "Caf\u{FFFD}\n- [ ] ".len(),
        "the click places the cursor after the box instead"
    );

    cx.simulate_keystrokes("ctrl-end ctrl-shift-left ctrl-b ctrl-e ctrl-k end enter");
    cx.simulate_input("(");
    assert_eq!(text(&note.editor, cx), shown);
    common::wait(AUTOSAVE_DELAY, cx);
    assert_eq!(std::fs::read(&note.path).unwrap(), contents);
}

#[gpui::test]
fn plain_text_shows_markdown_as_typed_and_edits_it_as_plain_text(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "x **bold** y\n# Title\n- [ ] task");
    editor.update(cx, |editor, cx| editor.set_markdown(false, cx));
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(x_of(&editor, 4, 0, cx), 4. * ADVANCE, "`**` are shown");
    assert_eq!(
        bounds(&editor, 15..16, cx).size.width,
        px(ADVANCE),
        "no heading size"
    );

    // The task box is text: a click places the cursor in it.
    click(task_box(&editor, 21, cx), Modifiers::none(), cx);
    assert_eq!(text(&editor, cx), "x **bold** y\n# Title\n- [ ] task");
    assert!(cursor(&editor, cx) > 21);

    cx.simulate_keystrokes("ctrl-end enter");
    cx.simulate_input("(");
    assert_eq!(
        text(&editor, cx),
        "x **bold** y\n# Title\n- [ ] task\n(",
        "no list item, no pair"
    );
    cx.simulate_keystrokes("shift-home ctrl-b ctrl-i ctrl-e ctrl-k end tab");
    assert_eq!(
        text(&editor, cx),
        "x **bold** y\n# Title\n- [ ] task\n(    "
    );

    // Back to Markdown, the same lines are styled again.
    editor.update(cx, |editor, cx| editor.set_markdown(true, cx));
    assert_eq!(x_of(&editor, 4, 0, cx), 2. * ADVANCE);
}

#[gpui::test]
fn images_look_and_open_like_links_and_load_nothing(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(
        cx,
        "![pic](https://example.com/p.png) ![local](file:///C:/x.png)\nnext",
    );
    cx.simulate_keystrokes("ctrl-end");
    // Displayed "pic local": `![` and `](...)` are hidden like a link's markers.
    assert_eq!(x_of(&editor, 2, 0, cx), 0.);
    assert_eq!(x_of(&editor, 36, 0, cx), 4. * ADVANCE);
    let line = bounds(&editor, 0..0, cx);
    let at_display = |column: f32| point(line.left() + px(column * ADVANCE + 4.), line.center().y);
    click(at_display(6.), Modifiers::secondary_key(), cx);
    assert_eq!(cx.opened_url(), None, "only web and mail links open");
    click(at_display(1.), Modifiers::secondary_key(), cx);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://example.com/p.png")
    );
}

#[gpui::test]
fn ctrl_click_opens_bare_addresses_at_their_resolved_destination(cx: &mut TestAppContext) {
    // Bare addresses hide nothing, so display columns are buffer columns.
    let (editor, cx) = open_editor(cx, "go www.example.com or a@b.org `www.c.org`\nnext");
    cx.simulate_keystrokes("ctrl-end");
    let end = cursor(&editor, cx);
    let line = bounds(&editor, 0..0, cx);
    let at_column = |column: f32| point(line.left() + px(column * ADVANCE + 4.), line.center().y);

    // Neither the text around them nor an address in code is a link, and a plain click edits.
    for column in [1., 20., 33.] {
        click(at_column(column), Modifiers::secondary_key(), cx);
        assert_eq!(cx.opened_url(), None, "column {column}");
    }
    click(at_column(5.), Modifiers::none(), cx);
    assert_eq!(cx.opened_url(), None);
    assert_eq!(cursor(&editor, cx), 5);

    cx.simulate_keystrokes("ctrl-end");
    click(at_column(5.), Modifiers::secondary_key(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://www.example.com"));
    assert_eq!(cursor(&editor, cx), end, "opening does not move the cursor");
    click(at_column(25.), Modifiers::secondary_key(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some("mailto:a@b.org"));
}

#[gpui::test]
fn a_callout_shows_its_label_in_place_of_its_tag_until_the_cursor_is_on_it(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = open_editor(cx, "> [!NOTE]\n> body\n\nnext");
    let tag_width = |cx: &mut VisualTestContext| {
        f32::from(bounds(&editor, 9..9, cx).left() - bounds(&editor, 2..2, cx).left())
    };
    cx.simulate_keystrokes("ctrl-end");
    // "Note" in semibold (4 × 9 px) and half an em of room after it.
    assert_eq!(tag_width(cx), 4. * ADVANCE + 8.);
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(tag_width(cx), 7. * ADVANCE, "on its line the tag is text");

    // Turned off, the quote is an ordinary one and the tag is text.
    cx.simulate_keystrokes("ctrl-end");
    editor.update(cx, |editor, cx| {
        let mut options = editor.markdown_options();
        options.callouts = false;
        editor.set_markdown_options(options, cx);
    });
    cx.run_until_parked();
    assert_eq!(tag_width(cx), 7. * ADVANCE);
}

#[gpui::test]
fn superscript_and_subscript_are_set_smaller_once_turned_on(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "a ^2^ b ~3~\nnext");
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(bounds(&editor, 3..4, cx).size.width, px(ADVANCE));
    editor.update(cx, |editor, cx| {
        let mut options = editor.markdown_options();
        options.superscript = true;
        options.subscript = true;
        editor.set_markdown_options(options, cx);
    });
    cx.run_until_parked();
    assert_eq!(round(bounds(&editor, 3..4, cx).size.width), 0.75 * ADVANCE);
    assert_eq!(round(bounds(&editor, 9..10, cx).size.width), 0.75 * ADVANCE);
    // Their markers are hidden: "a 2 b 3".
    assert_eq!(
        x_of(&editor, 9, 0, cx),
        4. * ADVANCE + 0.75 * ADVANCE + ADVANCE
    );
}

#[gpui::test]
fn an_emoji_shortcode_shows_its_emoji_until_the_cursor_is_on_it(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "a :smile: b :tada:\nnext");
    let shortcode_width = |cx: &mut VisualTestContext| {
        f32::from(bounds(&editor, 9..9, cx).left() - bounds(&editor, 2..2, cx).left())
    };
    cx.simulate_keystrokes("ctrl-end");
    // The test platform draws an emoji (a char outside the BMP) 2 em wide.
    assert_eq!(shortcode_width(cx), 2. * ADVANCE);
    assert_eq!(x_of(&editor, 10, 0, cx), 3. * ADVANCE + 2. * ADVANCE);

    // A click on the emoji never lands inside the shortcode: it puts the cursor at one end of it,
    // where the shortcode shows as typed and the other one stays an emoji.
    let line = bounds(&editor, 0..0, cx);
    click(
        point(line.left() + px(2. * ADVANCE + 14.), line.center().y),
        Modifiers::none(),
        cx,
    );
    assert!(
        [2, 9].contains(&cursor(&editor, cx)),
        "{}",
        cursor(&editor, cx)
    );
    assert_eq!(shortcode_width(cx), 7. * ADVANCE);
    assert_eq!(
        f32::from(bounds(&editor, 18..18, cx).left() - bounds(&editor, 12..12, cx).left()),
        2. * ADVANCE
    );
    assert_eq!(
        text(&editor, cx),
        "a :smile: b :tada:\nnext",
        "the text is unchanged"
    );

    // Turned off, shortcodes are text.
    cx.simulate_keystrokes("ctrl-end");
    editor.update(cx, |editor, cx| {
        let mut options = editor.markdown_options();
        options.emoji_shortcodes = false;
        editor.set_markdown_options(options, cx);
    });
    cx.run_until_parked();
    assert_eq!(shortcode_width(cx), 7. * ADVANCE);
}

#[gpui::test]
fn highlight_and_math_markers_hide_away_from_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = open_editor(cx, "a ==b== $c$\nnext");
    cx.simulate_keystrokes("ctrl-end");
    // "a b $c$": the `==` are hidden; with Math off by default, `$` is text.
    assert_eq!(x_of(&editor, 4, 0, cx), 2. * ADVANCE);
    assert_eq!(x_of(&editor, 9, 0, cx), 5. * ADVANCE);
    editor.update(cx, |editor, cx| {
        let mut options = editor.markdown_options();
        options.math = true;
        editor.set_markdown_options(options, cx);
    });
    cx.run_until_parked();
    // "a b c".
    assert_eq!(x_of(&editor, 9, 0, cx), 4. * ADVANCE);
    cx.simulate_keystrokes("ctrl-home right right right right");
    assert_eq!(
        x_of(&editor, 4, 0, cx),
        4. * ADVANCE,
        "on the highlight its markers show"
    );
}

/// Clicks on the text after something drawn narrower than its source (an emoji for `:smile:`, the
/// label `Tip` for `[!TIP]`) land on the char under the pointer, also on a wrapped row.
#[gpui::test]
fn clicks_after_emoji_and_short_callout_labels_hit_the_text_under_them(cx: &mut TestAppContext) {
    let wrapped = ":smile: a ".repeat(30);
    let (editor, cx) = open_editor(
        cx,
        &format!(":smile: abc\n> [!TIP]\n> body\n{wrapped}\nnext"),
    );
    cx.simulate_keystrokes("ctrl-end");
    let click_on = |offset: usize, cx: &mut VisualTestContext| {
        let at = bounds(&editor, offset..offset + 1, cx);
        click(
            point(at.left() + px(2.), at.center().y),
            Modifiers::none(),
            cx,
        );
        cursor(&editor, cx)
    };
    // "abc" after the emoji: each char is hit, the shortcode stays an emoji.
    for offset in [8, 9, 10] {
        assert_eq!(click_on(offset, cx), offset);
        cx.simulate_keystrokes("ctrl-end");
    }
    // Right of the short label, the click goes to the end of the tag's line.
    let label_line = bounds(&editor, 12..12, cx);
    click(
        point(label_line.left() + px(80.), label_line.center().y),
        Modifiers::none(),
        cx,
    );
    assert_eq!(cursor(&editor, cx), 20);
    cx.simulate_keystrokes("ctrl-end");
    // The 18th "a" of the wrapped line is on its second row, and a click on it lands on it.
    let line_start = 28;
    let a = line_start + 17 * 10 + 8;
    assert!(
        bounds(&editor, a..a + 1, cx).top() > bounds(&editor, line_start..line_start, cx).top()
    );
    assert_eq!(click_on(a, cx), a);
}
