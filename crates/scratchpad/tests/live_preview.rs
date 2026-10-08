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
    // 75 characters fit in a row. Displayed, the first row is 14 × "word " and "abc "; with the
    // cursor touching it, "**abc**" no longer fits and moves to the second row.
    let line = format!("{}**abc** tail tail tail", "word ".repeat(14));
    let (editor, cx) = open_editor(cx, &format!("{line}\nnext"));
    let first_row_top = bounds(&editor, 0..0, cx).top();

    cx.simulate_keystrokes("ctrl-home end");
    assert_eq!(
        cursor(&editor, cx),
        69,
        "after the last word left on the row"
    );
    assert_eq!(bounds(&editor, 69..69, cx).top(), first_row_top);
    cx.simulate_keystrokes("end");
    assert_eq!(cursor(&editor, cx), 69, "End again stays");

    cx.simulate_keystrokes("ctrl-home shift-end");
    let selected = editor.read_with(cx, |editor, _| editor.editor().selection().range());
    assert_eq!(selected.start.0..selected.end.0, 0..69);
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
