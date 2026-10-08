//! Moving the cursor and selecting with keyboard and mouse.

mod common;

use common::{editor, state};
use scratchpad_editor::{ByteOffset, Editor, Goal, Motion};

/// Applies `motion` repeatedly and records the marked state after each step.
fn walk(editor: &mut Editor, motion: Motion, extend: bool, steps: usize) -> Vec<String> {
    (0..steps)
        .map(|_| {
            editor.move_cursor(motion, extend);
            state(editor)
        })
        .collect()
}

#[test]
fn arrows_step_over_whole_grapheme_clusters() {
    // "e" + combining acute, a thumbs-up with skin tone modifier, a flag, decomposed Hangul.
    let mut ed = editor("|ae\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}x");
    assert_eq!(
        walk(&mut ed, Motion::Right, false, 7),
        [
            "a|e\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}x",
            "ae\u{301}|👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}x",
            "ae\u{301}👍🏽|🇩🇪\u{1100}\u{1161}\u{11a8}x",
            "ae\u{301}👍🏽🇩🇪|\u{1100}\u{1161}\u{11a8}x",
            "ae\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}|x",
            "ae\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}x|",
            "ae\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}x|",
        ]
    );
    assert_eq!(
        walk(&mut ed, Motion::Left, false, 3),
        [
            "ae\u{301}👍🏽🇩🇪\u{1100}\u{1161}\u{11a8}|x",
            "ae\u{301}👍🏽🇩🇪|\u{1100}\u{1161}\u{11a8}x",
            "ae\u{301}👍🏽|🇩🇪\u{1100}\u{1161}\u{11a8}x",
        ]
    );
}

#[test]
fn arrows_move_in_logical_order_through_rtl_text() {
    // Arabic letters carry combining harakat, which must stay with their base letter.
    let mut ed = editor("|مَرْحَبًا");
    assert_eq!(walk(&mut ed, Motion::Right, false, 2), ["مَ|رْحَبًا", "مَرْ|حَبًا"]);
}

#[test]
fn plain_arrows_collapse_a_selection_to_its_edge() {
    let mut ed = editor("ab^cd|ef");
    ed.move_cursor(Motion::Left, false);
    assert_eq!(state(&ed), "ab|cdef");

    let mut ed = editor("ab|cd^ef");
    ed.move_cursor(Motion::Right, false);
    assert_eq!(state(&ed), "abcd|ef");
}

#[test]
fn shift_arrows_grow_and_shrink_the_selection_across_the_anchor() {
    let mut ed = editor("ab^cd|ef");
    assert_eq!(
        walk(&mut ed, Motion::Left, true, 3),
        ["ab^c|def", "ab|cdef", "a|b^cdef"]
    );
    assert_eq!(walk(&mut ed, Motion::Right, true, 1), ["ab|cdef"]);
}

#[test]
fn ctrl_right_stops_after_words_punctuation_runs_and_at_line_ends() {
    let mut ed = editor("|Hello, wörld! foo_bar 123\nnext");
    assert_eq!(
        walk(&mut ed, Motion::WordRight, false, 9),
        [
            "Hello|, wörld! foo_bar 123\nnext",
            "Hello,| wörld! foo_bar 123\nnext",
            "Hello, wörld|! foo_bar 123\nnext",
            "Hello, wörld!| foo_bar 123\nnext",
            "Hello, wörld! foo_bar| 123\nnext",
            "Hello, wörld! foo_bar 123|\nnext",
            "Hello, wörld! foo_bar 123\n|next",
            "Hello, wörld! foo_bar 123\nnext|",
            "Hello, wörld! foo_bar 123\nnext|",
        ]
    );
}

#[test]
fn ctrl_left_mirrors_ctrl_right() {
    let mut ed = editor("Hello, wörld! foo_bar 123\nnext|");
    assert_eq!(
        walk(&mut ed, Motion::WordLeft, false, 9),
        [
            "Hello, wörld! foo_bar 123\n|next",
            "Hello, wörld! foo_bar 123|\nnext",
            "Hello, wörld! foo_bar |123\nnext",
            "Hello, wörld! |foo_bar 123\nnext",
            "Hello, wörld|! foo_bar 123\nnext",
            "Hello, |wörld! foo_bar 123\nnext",
            "Hello|, wörld! foo_bar 123\nnext",
            "|Hello, wörld! foo_bar 123\nnext",
            "|Hello, wörld! foo_bar 123\nnext",
        ]
    );
}

#[test]
fn word_motions_halt_at_line_boundaries_before_crossing_them() {
    let mut ed = editor("foo|   \n   bar");
    assert_eq!(
        walk(&mut ed, Motion::WordRight, false, 3),
        ["foo   |\n   bar", "foo   \n|   bar", "foo   \n   bar|"]
    );
    assert_eq!(
        walk(&mut ed, Motion::WordLeft, false, 3),
        ["foo   \n   |bar", "foo   \n|   bar", "foo   |\n   bar"]
    );
}

#[test]
fn word_motions_keep_combining_marks_with_their_word() {
    let mut ed = editor("|cafe\u{301} au lait");
    ed.move_cursor(Motion::WordRight, false);
    assert_eq!(state(&ed), "cafe\u{301}| au lait");
}

#[test]
fn ctrl_shift_arrows_select_by_word() {
    let mut ed = editor("one |two three");
    ed.move_cursor(Motion::WordRight, true);
    ed.move_cursor(Motion::WordRight, true);
    assert_eq!(state(&ed), "one ^two three|");
}

#[test]
fn home_end_and_document_boundaries() {
    let mut ed = editor("first\n  second| line\nthird");
    ed.move_cursor(Motion::LineStart, false);
    assert_eq!(state(&ed), "first\n|  second line\nthird");
    ed.move_cursor(Motion::LineEnd, true);
    assert_eq!(state(&ed), "first\n^  second line|\nthird");
    ed.move_cursor(Motion::DocumentEnd, true);
    assert_eq!(state(&ed), "first\n^  second line\nthird|");
    ed.move_cursor(Motion::DocumentStart, false);
    assert_eq!(state(&ed), "|first\n  second line\nthird");
}

#[test]
fn vertical_movement_remembers_the_column_across_short_lines() {
    let mut ed = editor("long line |here\nab\nanother long line");
    assert_eq!(
        walk(&mut ed, Motion::Down, false, 3),
        [
            "long line here\nab|\nanother long line",
            "long line here\nab\nanother lo|ng line",
            "long line here\nab\nanother long line|",
        ]
    );
    assert_eq!(
        walk(&mut ed, Motion::Up, false, 3),
        [
            "long line here\nab|\nanother long line",
            "long line |here\nab\nanother long line",
            "|long line here\nab\nanother long line",
        ]
    );
    // The goal survives hitting the top of the document.
    ed.move_cursor(Motion::Down, false);
    ed.move_cursor(Motion::Down, false);
    assert_eq!(state(&ed), "long line here\nab\nanother lo|ng line");
}

#[test]
fn vertical_movement_counts_columns_in_graphemes_not_bytes() {
    let mut ed = editor("日本語|テキスト\nabcdefg");
    ed.move_cursor(Motion::Down, false);
    assert_eq!(state(&ed), "日本語テキスト\nabc|defg");
}

#[test]
fn any_other_motion_resets_the_goal() {
    let mut ed = editor("abcdef|\nab\nabcdef");
    ed.move_cursor(Motion::Down, false);
    ed.move_cursor(Motion::Left, false);
    ed.move_cursor(Motion::Down, false);
    assert_eq!(state(&ed), "abcdef\nab\na|bcdef");
}

#[test]
fn shift_down_extends_the_selection() {
    let mut ed = editor("a|bc\ndef");
    ed.move_cursor(Motion::Down, true);
    assert_eq!(state(&ed), "a^bc\nd|ef");
}

#[test]
fn page_up_and_down_move_by_the_given_number_of_lines() {
    let mut ed = editor("0\n1|\n2\n3\n4\n5\n6\n7\n8\n9");
    ed.move_cursor(Motion::PageDown(5), false);
    assert_eq!(ed.buffer().offset_to_point(ed.selection().head).line, 6);
    ed.move_cursor(Motion::PageDown(5), false);
    assert_eq!(ed.selection().head, ed.buffer().end());
    ed.move_cursor(Motion::PageUp(3), true);
    assert_eq!(state(&ed), "0\n1\n2\n3\n4\n5\n6|\n7\n8\n9^");
    ed.move_cursor(Motion::PageUp(usize::MAX), false);
    assert_eq!(ed.selection().head, ByteOffset(0));
}

#[test]
fn motions_in_an_empty_document_stay_put() {
    let mut ed = editor("|");
    for motion in [
        Motion::Left,
        Motion::Right,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::Up,
        Motion::Down,
        Motion::PageUp(10),
        Motion::PageDown(10),
        Motion::DocumentStart,
        Motion::DocumentEnd,
    ] {
        ed.move_cursor(motion, true);
        assert_eq!(state(&ed), "|", "{motion:?}");
    }
}

#[test]
fn double_click_selects_the_run_under_the_pointer() {
    let text = "Hello, wörld!  foo";
    let select_word_at = |offset| {
        let mut ed = Editor::from_text(text);
        ed.select_word_at(ByteOffset(offset));
        state(&ed)
    };
    assert_eq!(select_word_at(9), "Hello, ^wörld|!  foo");
    assert_eq!(
        select_word_at(13),
        "Hello, wörld^!|  foo",
        "prefers the grapheme after the click"
    );
    assert_eq!(select_word_at(15), "Hello, wörld!^  |foo");
    assert_eq!(select_word_at(text.len()), "Hello, wörld!  ^foo|");

    let mut ed = Editor::from_text("cafe\u{301} au");
    ed.select_word_at(ByteOffset(5)); // inside the combining mark
    assert_eq!(state(&ed), "^cafe\u{301}| au");

    let mut ed = Editor::from_text("a\n\nb");
    ed.select_word_at(ByteOffset(2));
    assert_eq!(state(&ed), "a\n|\nb", "an empty line has nothing to select");
}

#[test]
fn triple_click_selects_the_whole_line_with_its_break() {
    let mut ed = Editor::from_text("one\ntwo\nthree");
    ed.select_line_at(ByteOffset(5));
    assert_eq!(state(&ed), "one\n^two\n|three");
    ed.select_line_at(ByteOffset(10));
    assert_eq!(state(&ed), "one\ntwo\n^three|");
}

#[test]
fn select_all_spans_the_document() {
    let mut ed = editor("one\ntw|o");
    ed.select_all();
    assert_eq!(state(&ed), "^one\ntwo|");
}

#[test]
fn mouse_drag_keeps_the_anchor_and_snaps_to_graphemes() {
    let mut ed = Editor::from_text("hello wörld");
    ed.move_to(ByteOffset(2), false);
    ed.move_to(ByteOffset(8), true); // inside "ö"
    assert_eq!(state(&ed), "he^llo w|örld");
    ed.move_to(ByteOffset(0), true);
    assert_eq!(state(&ed), "|he^llo wörld");
    ed.move_to(ByteOffset(100), false);
    assert_eq!(state(&ed), "hello wörld|");
}

#[test]
fn view_defined_goal_is_kept_until_the_next_non_vertical_change() {
    let mut ed = Editor::from_text("abc\ndef");
    ed.move_to_with_goal(ByteOffset(5), false, Goal::Horizontal(42.5));
    assert_eq!(ed.goal(), Goal::Horizontal(42.5));
    ed.move_cursor(Motion::Right, false);
    assert_eq!(ed.goal(), Goal::None);

    // Logical vertical movement replaces a view goal with its own column.
    ed.move_to_with_goal(ByteOffset(6), false, Goal::Horizontal(1.0));
    ed.move_cursor(Motion::Up, false);
    assert_eq!(state(&ed), "ab|c\ndef");
    assert_eq!(ed.goal(), Goal::Column(2));
}

#[test]
fn motions_through_a_long_line_of_flags_keep_flags_whole() {
    // Thousands of flags span many rope chunks, and where one flag ends depends on every regional indicator
    // before it.
    let flags = "🇩🇪".repeat(5000);
    let mut ed = editor(&format!("|{flags} end\n{flags}"));
    let second_line = ByteOffset(flags.len() + 5);

    ed.move_cursor(Motion::WordRight, false);
    assert_eq!(ed.selection().head, ByteOffset(flags.len()));
    ed.move_cursor(Motion::Down, false);
    assert_eq!(ed.selection().head, ed.buffer().end());
    ed.move_cursor(Motion::Left, false);
    assert_eq!(ed.selection().head.0, ed.buffer().len() - 8);
    ed.move_cursor(Motion::Up, false);
    assert_eq!(ed.selection().head.0, flags.len() - 8);
    ed.move_cursor(Motion::WordLeft, false);
    assert_eq!(ed.selection().head, ByteOffset(0));
    ed.move_to(second_line, false);
    ed.move_cursor(Motion::WordRight, false);
    assert_eq!(ed.selection().head, ed.buffer().end());
}
