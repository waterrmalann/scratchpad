//! End-to-end tests of find in the open note (PLAN §29): the find bar, matching, stepping through
//! matches, highlights and searching large notes in the background.
//!
//! The test platform shapes every BMP char 0.6 em wide: 9 px at the 15 px body size.

mod common;

use std::ops::Range;

use gpui::{
    Bounds, Entity, EntityInputHandler, Focusable, Pixels, TestAppContext, VisualTestContext, px,
    size,
};
use scratchpad::AppWindow;
use scratchpad::editor_view::{BACKGROUND_SEARCH_BYTES, EditorView, SEARCH_DEBOUNCE};
use scratchpad::find_bar::FindBar;

const ADVANCE: f32 = 9.;

struct Find<'a> {
    editor: Entity<EditorView>,
    bar: Entity<FindBar>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> Find<'a> {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let editor = common::editor(&root, cx);
    let bar = find_bar(&root, cx);
    editor.update(cx, |editor, cx| editor.set_text(text, cx));
    cx.run_until_parked();
    Find { editor, bar, cx }
}

fn find_bar(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Entity<FindBar> {
    root.read_with(cx, |root, cx| {
        root.editor_pane().read(cx).find_bar().clone()
    })
}

impl Find<'_> {
    fn status(&mut self) -> String {
        self.bar.read_with(self.cx, |bar, cx| bar.status(cx))
    }

    fn query(&mut self) -> String {
        self.bar.read_with(self.cx, |bar, cx| bar.query(cx))
    }

    fn is_open(&mut self) -> bool {
        self.bar.read_with(self.cx, |bar, _| bar.is_open())
    }

    /// The selected byte range.
    fn selection(&mut self) -> Range<usize> {
        self.editor.read_with(self.cx, |editor, _| {
            let range = editor.editor().selection().range();
            range.start.0..range.end.0
        })
    }

    fn selected_text(&mut self) -> String {
        self.editor.read_with(self.cx, |editor, _| {
            let editor = editor.editor();
            editor
                .buffer()
                .text_for_range(editor.selection().range())
                .into_owned()
        })
    }

    fn editor_focused(&mut self) -> bool {
        let focus = self
            .editor
            .read_with(self.cx, |editor, cx| editor.focus_handle(cx));
        self.cx.update(|window, _| focus.is_focused(window))
    }

    fn focus_editor(&mut self) {
        let focus = self
            .editor
            .read_with(self.cx, |editor, cx| editor.focus_handle(cx));
        self.cx.update(|window, _| window.focus(&focus));
        self.cx.run_until_parked();
    }

    fn highlights(&mut self) -> Vec<Bounds<Pixels>> {
        self.editor
            .read_with(self.cx, |editor, _| editor.match_highlights().to_vec())
    }

    fn visible_lines(&mut self) -> Range<usize> {
        self.editor
            .read_with(self.cx, |editor, _| editor.visible_lines())
    }

    fn background_searches(&mut self) -> usize {
        self.editor
            .read_with(self.cx, |editor, _| editor.background_search_count())
    }

    /// Lets the search debounce pass.
    fn wait(&mut self) {
        common::wait(SEARCH_DEBOUNCE, self.cx);
    }

    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
    }

    /// Screen bounds of the text at UTF-16 `range`, as the IME sees them.
    fn text_bounds(&mut self, range: Range<usize>) -> Bounds<Pixels> {
        self.editor.update_in(self.cx, |editor, window, cx| {
            editor
                .bounds_for_range(range, Bounds::default(), window, cx)
                .expect("range is visible")
        })
    }
}

#[gpui::test]
fn ctrl_f_opens_the_bar_with_the_selected_text_and_finds_it(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta");
    find.keys("shift-right shift-right shift-right shift-right");
    assert_eq!(find.selected_text(), "beta");

    find.keys("ctrl-f");
    assert!(find.is_open());
    assert!(!find.editor_focused(), "focus is in the query");
    assert_eq!(find.query(), "beta");
    find.wait();
    assert_eq!(find.status(), "1 of 2");
    assert_eq!(find.selection(), 0..4);

    // Typing replaces the whole query: Ctrl+F selected it.
    find.cx.simulate_input("alpha");
    assert_eq!(find.query(), "alpha");
}

#[gpui::test]
fn a_selection_over_several_lines_or_a_long_one_is_not_used_as_the_query(cx: &mut TestAppContext) {
    let mut find = open(cx, "one\ntwo");
    find.keys("ctrl-a ctrl-f");
    assert!(find.is_open());
    assert_eq!(find.query(), "");

    find.keys("escape");
    let line = "x".repeat(2000);
    find.editor
        .update(find.cx, |editor, cx| editor.set_text(&line, cx));
    find.keys("ctrl-a ctrl-f");
    assert_eq!(find.query(), "");
    find.keys("escape ctrl-home shift-right ctrl-f");
    assert_eq!(find.query(), "x", "a short one is");
}

#[gpui::test]
fn typing_a_query_finds_the_first_match_after_the_cursor_with_every_key(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta gamma beta");
    find.keys("right right right right right right ctrl-f");
    find.cx.simulate_input("B");
    assert_eq!(find.status(), "2 of 3", "a short note is searched at once");
    find.cx.simulate_input("ETA");
    assert_eq!(find.status(), "2 of 3");
    assert_eq!(find.selection(), 11..15);

    find.cx.simulate_input("x");
    assert_eq!(find.status(), "No results");
    assert_eq!(find.selection(), 11..15, "the selection stays");
}

#[gpui::test]
fn enter_and_f3_step_through_the_matches_and_wrap_around(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta gamma beta");
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    assert_eq!((find.status(), find.selection()), ("1 of 3".into(), 0..4));

    let mut step = |keys: &str| {
        find.keys(keys);
        (find.status(), find.selection())
    };
    assert_eq!(step("enter"), ("2 of 3".into(), 11..15));
    assert_eq!(step("enter"), ("3 of 3".into(), 22..26));
    assert_eq!(step("enter"), ("1 of 3".into(), 0..4), "wraps to the first");
    assert_eq!(
        step("shift-enter"),
        ("3 of 3".into(), 22..26),
        "wraps to the last"
    );
    assert_eq!(step("shift-enter"), ("2 of 3".into(), 11..15));
    assert_eq!(step("f3"), ("3 of 3".into(), 22..26));
    assert_eq!(step("shift-f3"), ("2 of 3".into(), 11..15));
}

#[gpui::test]
fn escape_closes_the_bar_keeps_the_match_selected_and_f3_finds_again(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta");
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    find.keys("enter");
    assert_eq!(find.selection(), 11..15);

    find.keys("escape");
    assert!(!find.is_open());
    assert!(find.editor_focused());
    assert_eq!(find.selection(), 11..15);
    assert!(find.highlights().is_empty(), "no highlights once closed");

    // F3 in the note brings the last query back and goes on from the selection.
    find.keys("f3");
    assert!(find.is_open());
    assert!(find.editor_focused(), "focus stays in the note");
    find.wait();
    assert_eq!((find.status(), find.selection()), ("1 of 2".into(), 0..4));

    // Escape in the note closes the bar too.
    find.keys("escape");
    assert!(!find.is_open());
    assert_eq!(find.selection(), 0..4);
}

#[gpui::test]
fn editing_the_note_while_the_bar_is_open_updates_the_matches(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta");
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    assert_eq!(find.status(), "1 of 2");

    find.focus_editor();
    find.keys("ctrl-end");
    find.cx.simulate_input(" beta");
    // Until the next search, the matches found move with the text.
    assert_eq!(find.status(), "2 matches");
    find.wait();
    assert_eq!(find.status(), "3 matches");
    assert_eq!(find.selection(), 20..20, "editing does not move the cursor");

    // Undo removes a match; the one the edit touched is dropped at once.
    find.keys("ctrl-z");
    assert_eq!(find.status(), "2 matches");
    find.wait();
    assert_eq!(find.status(), "2 matches");
}

#[gpui::test]
fn opening_another_note_with_the_bar_open_searches_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    common::write_note(
        dir.path(),
        "First",
        "beta alpha beta",
        common::days_ago(0, 10),
    );
    let second = common::write_note(
        dir.path(),
        "Second",
        "beta beta beta",
        common::days_ago(1, 10),
    );
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    cx.simulate_resize(size(px(1100.), px(720.)));
    let notes = common::notes(&root, cx);
    let mut find = Find {
        editor: common::editor(&root, cx),
        bar: find_bar(&root, cx),
        cx,
    };
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    find.keys("enter");
    assert_eq!((find.status(), find.selection()), ("2 of 2".into(), 11..15));
    assert_eq!(find.highlights().len(), 1);
    // Asked for while the search of the changed query is pending; the other note makes it moot.
    find.keys("backspace a enter");

    notes.update(find.cx, |notes, cx| notes.select(&second, cx));
    find.cx.run_until_parked();
    assert_eq!(common::editor_text(&root, find.cx), "beta beta beta");
    assert_eq!(find.status(), "", "not searched yet");
    assert!(find.highlights().is_empty(), "nothing from the other note");
    find.wait();
    assert_eq!(find.status(), "3 matches");
    assert_eq!(find.selection(), 0..0, "the cursor stays at the start");
}

#[gpui::test]
fn typing_in_the_note_after_f3_wins_over_the_pending_jump(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta");
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    find.focus_editor();
    find.keys("ctrl-end");
    find.cx.simulate_input(" x");
    // The edit's search is pending, so the step waits for it, but typing goes on.
    find.keys("f3");
    find.cx.simulate_input("y");
    find.wait();
    assert_eq!(find.status(), "2 matches");
    assert_eq!(
        find.selection(),
        18..18,
        "the cursor stays where typing left it"
    );
}

#[gpui::test]
fn a_match_inside_a_letter_with_an_accent_selects_the_whole_letter(cx: &mut TestAppContext) {
    // "é" written as "e" and a combining accent (as on macOS): "e" finds the e inside it.
    let mut find = open(cx, "cafe\u{301} cafe\u{301}");
    find.keys("ctrl-f");
    find.cx.simulate_input("e");
    find.wait();
    assert_eq!((find.status(), find.selection()), ("1 of 2".into(), 3..6));
    find.keys("enter");
    assert_eq!((find.status(), find.selection()), ("2 of 2".into(), 10..13));
    find.keys("enter");
    assert_eq!(find.selection(), 3..6, "wraps around");
}

#[gpui::test]
fn letters_whose_lowercase_differs_in_length_match_with_exact_byte_ranges(cx: &mut TestAppContext) {
    // ADR 0006: lowercase comparison, not full case folding, so "ß" never matches "ss".
    let mut find = open(cx, "Straße, STRASSE, STRAẞE");
    find.keys("ctrl-f");
    find.cx.simulate_input("straße");
    find.wait();
    assert_eq!(find.status(), "1 of 2");
    assert_eq!(find.selected_text(), "Straße");
    assert_eq!(find.selection(), 0..7);
    find.keys("enter");
    // "ẞ" (capital sharp s) is 3 bytes, its lowercase "ß" 2.
    assert_eq!(find.selected_text(), "STRAẞE");
    assert_eq!(find.selection(), 18..26);

    find.keys("ctrl-a");
    find.cx.simulate_input("strasse");
    find.wait();
    assert_eq!(find.status(), "1 of 1");
    assert_eq!(find.selected_text(), "STRASSE");
}

#[gpui::test]
fn match_case_finds_only_the_same_case(cx: &mut TestAppContext) {
    let mut find = open(cx, "Beta beta BETA");
    find.keys("ctrl-f");
    find.cx.simulate_input("beta");
    find.wait();
    assert_eq!(find.status(), "1 of 3");

    find.keys("alt-c");
    find.wait();
    assert_eq!(find.status(), "1 of 1");
    assert_eq!(find.selection(), 5..9);
}

#[gpui::test]
fn matches_in_text_with_hidden_markers_are_highlighted_where_they_are_shown(
    cx: &mut TestAppContext,
) {
    let mut find = open(cx, "word here\n\na **word** b");
    find.keys("ctrl-f");
    find.cx.simulate_input("word");
    find.wait();
    assert_eq!(find.selection(), 0..4);

    // The second match is not selected, so its line shows "a word b" without the asterisks.
    let line_start = 11;
    let left = find.text_bounds(line_start..line_start).left();
    let highlights = find.highlights();
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].left() - left, px(2. * ADVANCE));
    assert_eq!(highlights[0].size.width, px(4. * ADVANCE));

    find.keys("enter");
    assert_eq!(find.selection(), 15..19, "the word between the markers");
    assert_eq!(find.selected_text(), "word");
}

#[gpui::test]
fn only_matches_on_visible_lines_are_highlighted(cx: &mut TestAppContext) {
    let text = vec!["find me"; 2000].join("\n");
    let mut find = open(cx, &text);
    find.keys("ctrl-f");
    find.cx.simulate_input("find");
    find.wait();
    assert_eq!(find.status(), "1 of 2000");

    let visible = find.visible_lines();
    assert_eq!(visible.start, 0);
    assert!(visible.len() < 40, "{visible:?}");
    // One per visible line except the selected match on the first, which shows as the selection.
    assert_eq!(find.highlights().len(), visible.len() - 1);

    // The last match is far off screen: the view jumps to it and puts it in the middle (as far
    // as the end of the note allows, which is the middle too).
    find.keys("shift-enter");
    assert_eq!(find.status(), "2000 of 2000");
    let visible = find.visible_lines();
    assert!(visible.contains(&1999), "{visible:?}");
    assert_eq!(find.highlights().len(), visible.len() - 1);
}

#[gpui::test]
fn a_match_off_screen_is_scrolled_to_the_middle(cx: &mut TestAppContext) {
    let mut lines = vec!["filler"; 1000];
    lines[0] = "target";
    lines[500] = "target";
    let mut find = open(cx, &lines.join("\n"));
    find.keys("ctrl-f");
    find.cx.simulate_input("target");
    find.wait();
    find.keys("enter");
    assert_eq!(find.status(), "2 of 2");
    let visible = find.visible_lines();
    let (above, below) = (500 - visible.start, visible.end - 1 - 500);
    assert!(above.abs_diff(below) <= 1, "{visible:?}");
}

// Each seed orders the pending timers and tasks differently.
#[gpui::test(iterations = 10)]
fn large_notes_are_searched_in_the_background_once_typing_pauses(cx: &mut TestAppContext) {
    let line = "lorem ipsum dolor sit amet\n";
    let mut text = line.repeat(BACKGROUND_SEARCH_BYTES / line.len() + 1);
    text.push_str("needle");
    let mut find = open(cx, &text);
    find.keys("ctrl-f");
    find.cx.simulate_input("need");
    assert_eq!(find.background_searches(), 0);
    find.wait();
    assert_eq!(find.background_searches(), 1);
    assert_eq!(find.status(), "1 of 1");

    // Changes in quick succession, to both the note and the query, are searched once, and only
    // as they are at the end.
    find.focus_editor();
    find.keys("ctrl-home");
    find.cx.simulate_input("needles ");
    common::wait(SEARCH_DEBOUNCE / 2, find.cx);
    find.keys("ctrl-f");
    find.cx.simulate_input("needle");
    common::wait(SEARCH_DEBOUNCE / 2, find.cx);
    find.focus_editor();
    find.keys("ctrl-end");
    find.cx.simulate_input(" needle");
    assert_eq!(find.background_searches(), 1);
    find.wait();
    assert_eq!(find.background_searches(), 2);
    assert_eq!(find.status(), "3 matches");
}

#[gpui::test]
fn small_notes_are_searched_on_the_ui_thread(cx: &mut TestAppContext) {
    let mut find = open(cx, "needle");
    find.keys("ctrl-f");
    find.cx.simulate_input("needle");
    find.wait();
    assert_eq!(find.status(), "1 of 1");
    assert_eq!(find.background_searches(), 0);
}

#[gpui::test]
fn the_bar_fits_in_a_narrow_editor(cx: &mut TestAppContext) {
    let mut find = open(cx, "text");
    find.cx.simulate_resize(size(px(560.), px(400.)));
    find.keys("ctrl-f");
    let pane = find.cx.debug_bounds("editor-pane").unwrap();
    let bar = find.cx.debug_bounds("find-bar").unwrap();
    assert!(
        pane.left() < bar.left() && bar.right() < pane.right(),
        "{bar:?} is not inside {pane:?}"
    );
}

#[gpui::test]
fn ctrl_f_with_the_settings_open_closes_them_and_opens_the_bar(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    let bar = find_bar(&root, cx);
    let settings_open =
        |cx: &mut VisualTestContext| root.read_with(cx, |root, _| root.settings_panel().is_some());
    cx.simulate_keystrokes("ctrl-,");
    assert!(settings_open(cx));

    cx.simulate_keystrokes("ctrl-f");
    assert!(!settings_open(cx));
    assert!(bar.read_with(cx, |bar, _| bar.is_open()));
    cx.simulate_keystrokes("escape");
    assert!(!bar.read_with(cx, |bar, _| bar.is_open()));
}

// --- Replace (ADR 0140) ---

impl Find<'_> {
    fn text(&mut self) -> String {
        self.editor.read_with(self.cx, |editor, _| editor.text())
    }

    fn replacement(&mut self) -> String {
        self.bar.read_with(self.cx, |bar, cx| bar.replacement(cx))
    }

    fn is_replacing(&mut self) -> bool {
        self.bar.read_with(self.cx, |bar, _| bar.is_replacing())
    }

    /// Searches for `query`, then opens the replace row and types `replacement`.
    fn replace_with(&mut self, query: &str, replacement: &str) {
        self.keys("ctrl-f");
        self.cx.simulate_input(query);
        self.wait();
        self.keys("ctrl-h");
        self.cx.simulate_input(replacement);
    }
}

/// The find bar of the window on the notes in `dir`.
fn open_in<'a>(dir: &std::path::Path, cx: &'a mut TestAppContext) -> Find<'a> {
    let (root, cx) = common::open_main_window_in(dir, cx);
    Find {
        editor: common::editor(&root, cx),
        bar: find_bar(&root, cx),
        cx,
    }
}

#[gpui::test]
fn ctrl_h_opens_the_replace_row_and_tab_goes_between_the_fields(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta");
    find.keys("shift-right shift-right shift-right shift-right ctrl-h");
    assert!(find.is_open() && find.is_replacing());
    assert_eq!(
        find.query(),
        "beta",
        "the selection is the query, as with Ctrl+F"
    );
    find.cx.simulate_input("gamma");
    assert_eq!(
        (find.query(), find.replacement()),
        ("beta".into(), "gamma".into())
    );

    // Each field is selected when Tab goes to it, so typing replaces it.
    find.keys("tab");
    find.cx.simulate_input("alpha");
    find.keys("shift-tab");
    find.cx.simulate_input("delta");
    assert_eq!(
        (find.query(), find.replacement()),
        ("alpha".into(), "delta".into())
    );

    // Ctrl+F goes back to finding only; the chevron shows the replace row again.
    find.keys("ctrl-f");
    assert!(find.is_open() && !find.is_replacing());
    find.keys("tab");
    find.cx.simulate_input("x");
    assert_eq!(find.query(), "x", "with one field, Tab stays in it");
    common::click("find-toggle-replace", find.cx);
    assert!(find.is_replacing());
    assert_eq!(find.replacement(), "delta");

    find.keys("tab escape");
    assert!(!find.is_open());
    assert!(find.editor_focused());
}

#[gpui::test]
fn replace_replaces_the_selected_match_and_selects_the_next(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta alpha beta gamma beta");
    find.replace_with("beta", "x");
    assert_eq!(find.selection(), 0..4);

    find.keys("enter");
    assert_eq!(find.text(), "x alpha beta gamma beta");
    assert_eq!((find.status(), find.selection()), ("1 of 2".into(), 8..12));
    find.keys("enter");
    assert_eq!(find.text(), "x alpha x gamma beta");
    assert_eq!(find.selected_text(), "beta");
    find.keys("enter");
    assert_eq!(find.text(), "x alpha x gamma x");
    find.wait();
    assert_eq!(find.status(), "No results");
    find.keys("enter");
    assert_eq!(find.text(), "x alpha x gamma x", "nothing left to replace");
}

#[gpui::test]
fn replace_without_a_match_selected_only_selects_the_next_one(cx: &mut TestAppContext) {
    let mut find = open(cx, "beta one beta two");
    find.replace_with("beta", "x");
    find.focus_editor();
    find.keys("ctrl-home shift-right shift-right shift-right shift-right shift-right");
    assert_eq!(
        find.selected_text(),
        "beta ",
        "more than a match is not the match"
    );

    common::click("replace-next", find.cx);
    assert_eq!(find.text(), "beta one beta two");
    assert_eq!(find.selection(), 9..13);
    common::click("replace-next", find.cx);
    assert_eq!(find.text(), "beta one x two");
    assert_eq!(find.selection(), 0..4, "wraps around to the first");
}

#[gpui::test]
fn replace_all_ignores_case_unless_asked_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let original = "beta Beta BETA gamma";
    let mut find = open(cx, original);
    find.replace_with("beta", "x");
    find.keys("ctrl-alt-enter");
    assert_eq!(find.text(), "x x x gamma");
    assert_eq!(find.status(), "Replaced 3");
    find.wait();
    assert_eq!(find.status(), "Replaced 3", "until something else happens");

    find.focus_editor();
    find.keys("ctrl-z");
    assert_eq!(find.text(), original, "one undo brings back every match");
    assert_eq!(find.selection(), 0..4, "and the selection");
    find.wait();
    assert_eq!(find.status(), "1 of 3");

    // Without a selection Ctrl+H keeps the query.
    find.keys("ctrl-end ctrl-h alt-c alt-a");
    assert_eq!(find.text(), "x Beta BETA gamma");
    assert_eq!(find.status(), "Replaced 1");
    find.keys("enter");
    assert_eq!(find.status(), "No results");
}

#[gpui::test]
fn a_replacement_containing_the_query_is_not_replaced_again(cx: &mut TestAppContext) {
    let mut find = open(cx, "ab ab");
    find.replace_with("ab", "abab");
    find.keys("enter");
    assert_eq!(find.text(), "abab ab");
    assert_eq!(
        find.selection(),
        5..7,
        "the next match after the replacement"
    );
    find.keys("alt-a");
    assert_eq!(find.text(), "abababab abab");
    assert_eq!(find.status(), "Replaced 3");
}

#[gpui::test]
fn replacing_text_between_hidden_markers_keeps_the_markers(cx: &mut TestAppContext) {
    let mut find = open(cx, "a **word** b\n\nword and *word*");
    find.replace_with("word", "term");
    assert_eq!(find.selection(), 4..8);
    find.keys("enter");
    assert_eq!(find.text(), "a **term** b\n\nword and *word*");
    assert_eq!(find.selection(), 14..18);
    find.keys("alt-a");
    assert_eq!(find.text(), "a **term** b\n\nterm and *term*");
}

#[gpui::test]
fn replace_changes_only_the_matched_part_of_a_letter_with_an_accent(cx: &mut TestAppContext) {
    // "é" written as "e" and a combining accent: the whole letter is selected (ADR 0100), but
    // Replace, like Replace All, replaces the "e" and leaves the accent.
    let mut find = open(cx, "cafe\u{301} cafe\u{301}");
    // Not `replace_with`: Ctrl+H on the selected letter would make all of it the query.
    find.keys("ctrl-h");
    find.cx.simulate_input("x");
    find.keys("shift-tab");
    find.cx.simulate_input("e");
    find.keys("tab");
    assert_eq!(find.selection(), 3..6);
    find.keys("enter");
    assert_eq!(find.text(), "cafx\u{301} cafe\u{301}");
    assert_eq!(find.selection(), 10..13);
    find.keys("enter");
    assert_eq!(find.text(), "cafx\u{301} cafx\u{301}");
}

#[gpui::test]
fn replace_in_a_large_note_never_uses_an_older_search(cx: &mut TestAppContext) {
    let line = "lorem ipsum dolor sit amet\n";
    let filler = line.repeat(BACKGROUND_SEARCH_BYTES / line.len() + 1);
    let mut find = open(cx, &format!("{filler}needle"));
    find.keys("ctrl-h");
    find.cx.simulate_input("pin");
    find.keys("shift-tab");
    find.cx.simulate_input("needle");
    assert_eq!(find.status(), "", "searched once typing pauses");
    find.keys("tab enter");
    assert_eq!(find.selected_text(), "needle", "searched for Replace");

    // The edit's search is pending too: Replace searches again rather than trust the matches
    // the edit moved, which miss the one it made.
    find.focus_editor();
    find.keys("ctrl-home");
    find.cx.simulate_input("needle ");
    find.keys("ctrl-home");
    common::click("replace-next", find.cx);
    assert_eq!(find.selection(), 0..6);
    common::click("replace-next", find.cx);
    assert!(find.text().starts_with("pin lorem"));
    assert_eq!(find.selected_text(), "needle");
    common::click("replace-all", find.cx);
    assert!(find.text().ends_with("amet\npin"));
}

#[gpui::test]
fn replace_all_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let text = "Pets\ncat and cat food";
    let path = common::write_note(dir.path(), "Pets", text, common::days_ago(0, 10));
    let mut find = open_in(dir.path(), cx);
    find.replace_with("cat", "dog");
    find.keys("alt-a");
    common::wait(scratchpad::session::AUTOSAVE_DELAY, find.cx);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "Pets\ndog and dog food"
    );
}

#[gpui::test]
fn a_read_only_note_is_not_replaced(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Latin1.md");
    std::fs::write(&path, b"caf\xE9 cat cat").unwrap();
    let mut find = open_in(dir.path(), cx);
    find.replace_with("cat", "dog");
    let selection = find.selection();
    find.keys("enter");
    common::click("replace-next", find.cx);
    find.keys("alt-a ctrl-alt-enter");
    common::click("replace-all", find.cx);
    assert_eq!(find.text(), "caf\u{FFFD} cat cat");
    assert_eq!(find.selection(), selection);
    assert_eq!(find.status(), "1 of 2");
    common::wait(scratchpad::session::AUTOSAVE_DELAY, find.cx);
    assert_eq!(std::fs::read(&path).unwrap(), b"caf\xE9 cat cat");
}

#[gpui::test]
fn the_bar_with_the_replace_row_fits_in_a_narrow_editor(cx: &mut TestAppContext) {
    let mut find = open(cx, "text");
    find.cx.simulate_resize(size(px(560.), px(400.)));
    find.keys("ctrl-h");
    let pane = find.cx.debug_bounds("editor-pane").unwrap();
    for part in ["find-bar", "replace-all", "find-close"] {
        let bounds = find.cx.debug_bounds(part).unwrap();
        assert!(
            pane.left() < bounds.left() && bounds.right() < pane.right(),
            "{part} {bounds:?} is not inside {pane:?}"
        );
    }
}
