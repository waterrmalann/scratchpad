//! Typing, deleting, line operations and clipboard.

mod common;

use common::{editor, state};
use scratchpad_editor::{ByteOffset, Editor, Motion};

fn type_chars(editor: &mut Editor, text: &str) {
    for c in text.chars() {
        editor.insert_text(c.encode_utf8(&mut [0; 4]));
    }
}

#[test]
fn typing_a_sentence_and_fixing_a_typo() {
    let mut ed = editor("|");
    type_chars(&mut ed, "Helo");
    ed.move_cursor(Motion::Left, false);
    type_chars(&mut ed, "l");
    ed.move_cursor(Motion::LineEnd, false);
    type_chars(&mut ed, ", world");
    ed.insert_newline();
    type_chars(&mut ed, "日本語 👋🏽");
    assert_eq!(state(&ed), "Hello, world\n日本語 👋🏽|");
}

#[test]
fn typing_replaces_the_selection() {
    let mut ed = editor("Hello ^world|!");
    ed.insert_text("there");
    assert_eq!(state(&ed), "Hello there|!");

    let mut ed = editor("a|bc^d");
    ed.insert_newline();
    assert_eq!(state(&ed), "a\n|d");
}

#[test]
fn backspace_removes_whole_grapheme_clusters() {
    let mut ed = editor("x🇩🇪🇫🇷👨‍👩‍👧e\u{301}|");
    for expected in ["x🇩🇪🇫🇷👨‍👩‍👧|", "x🇩🇪🇫🇷|", "x🇩🇪|", "x|", "|", "|"]
    {
        ed.backspace();
        assert_eq!(state(&ed), expected);
    }
}

#[test]
fn delete_forward_removes_whole_grapheme_clusters_and_joins_lines() {
    let mut ed = editor("|e\u{301}\u{5e9}\u{5c1}\nb");
    for expected in ["|\u{5e9}\u{5c1}\nb", "|\nb", "|b", "|", "|"] {
        ed.delete_forward();
        assert_eq!(state(&ed), expected);
    }
}

#[test]
fn backspace_at_line_start_joins_lines() {
    let mut ed = editor("one\n|two");
    ed.backspace();
    assert_eq!(state(&ed), "one|two");
}

#[test]
fn deleting_with_a_selection_removes_only_the_selection() {
    for delete in [
        Editor::backspace,
        Editor::delete_forward,
        Editor::delete_word_backward,
        Editor::delete_word_forward,
    ] {
        let mut ed = editor("one t|wo th^ree");
        delete(&mut ed);
        assert_eq!(state(&ed), "one t|ree");
    }
}

#[test]
fn ctrl_backspace_deletes_words_and_punctuation_runs() {
    let mut ed = editor("let x = foo.bar(baz);  |");
    for expected in [
        "let x = foo.bar(baz|",
        "let x = foo.bar(|",
        "let x = foo.bar|",
        "let x = foo.|",
        "let x = foo|",
        "let x = |",
    ] {
        ed.delete_word_backward();
        assert_eq!(state(&ed), expected);
    }
}

#[test]
fn ctrl_delete_deletes_to_the_next_word_end_and_joins_lines_at_line_end() {
    let mut ed = editor("|alpha beta\ngamma");
    for expected in ["| beta\ngamma", "|\ngamma", "|gamma", "|"] {
        ed.delete_word_forward();
        assert_eq!(state(&ed), expected);
    }
}

#[test]
fn editing_at_document_boundaries_is_harmless() {
    let mut ed = editor("|abc");
    ed.backspace();
    ed.delete_word_backward();
    ed.move_lines_up();
    assert_eq!(state(&ed), "|abc");
    assert_eq!(
        ed.buffer().version(),
        0,
        "no-op edits must not count as changes"
    );

    let mut ed = editor("abc|");
    ed.delete_forward();
    ed.delete_word_forward();
    ed.move_lines_down();
    assert_eq!(state(&ed), "abc|");
    assert_eq!(ed.buffer().version(), 0);
}

#[test]
fn duplicating_lines_moves_the_selection_onto_the_copy() {
    let mut ed = editor("a\nb|c\nd");
    ed.duplicate_lines();
    assert_eq!(state(&ed), "a\nbc\nb|c\nd");

    let mut ed = editor("^a\nb|\nc");
    ed.duplicate_lines();
    assert_eq!(state(&ed), "a\nb\n^a\nb|\nc");

    // The last line has no line break to copy.
    let mut ed = editor("x\ny|");
    ed.duplicate_lines();
    assert_eq!(state(&ed), "x\ny\ny|");
}

#[test]
fn a_selection_ending_at_a_line_start_does_not_include_that_line() {
    let mut ed = editor("^a\n|b");
    ed.duplicate_lines();
    assert_eq!(state(&ed), "a\n^a\n|b");

    let mut ed = editor("x\n^a\n|b");
    ed.move_lines_up();
    assert_eq!(state(&ed), "^a\n|x\nb");
}

#[test]
fn moving_lines_up_and_down_carries_the_selection() {
    let mut ed = editor("one\ntw|o\nthree");
    ed.move_lines_up();
    assert_eq!(state(&ed), "tw|o\none\nthree");
    ed.move_lines_up();
    assert_eq!(state(&ed), "tw|o\none\nthree");
    ed.move_lines_down();
    ed.move_lines_down();
    assert_eq!(state(&ed), "one\nthree\ntw|o");
    ed.move_lines_down();
    assert_eq!(state(&ed), "one\nthree\ntw|o");

    let mut ed = editor("^a\nb|\nc\nd");
    ed.move_lines_down();
    assert_eq!(state(&ed), "c\n^a\nb|\nd");
}

#[test]
fn copy_cut_and_paste_multiple_lines() {
    let mut ed = editor("|first\nsecond\nthird");
    assert_eq!(ed.copy(), None, "nothing selected");
    assert_eq!(ed.cut(), None);
    assert_eq!(ed.buffer().version(), 0);

    ed.move_cursor(Motion::Down, true);
    let clip = ed.cut().unwrap();
    assert_eq!(clip, "first\n");
    assert_eq!(state(&ed), "|second\nthird");

    ed.move_cursor(Motion::DocumentEnd, false);
    ed.insert_newline();
    ed.paste(&clip);
    assert_eq!(state(&ed), "second\nthird\nfirst\n|");

    ed.select_all();
    assert_eq!(ed.copy().unwrap(), "second\nthird\nfirst\n");
}

#[test]
fn pasted_windows_and_old_mac_line_breaks_are_normalized() {
    let mut ed = editor("^x|");
    ed.paste("one\r\ntwo\rthree\n");
    assert_eq!(state(&ed), "one\ntwo\nthree\n|");
    assert_eq!(ed.buffer().line_count(), 4);
}

#[test]
fn cursor_never_ends_inside_a_cluster_formed_by_an_insertion() {
    // A lone combining mark at the start of the document: inserting a base letter before it fuses both
    // into one cluster, so the cursor goes after the cluster.
    let mut ed = editor("|\u{301}x");
    ed.insert_text("e");
    assert_eq!(state(&ed), "e\u{301}|x");

    // Typing a combining mark after its base (dead keys) leaves the cursor after the composed character.
    let mut ed = editor("caf|");
    type_chars(&mut ed, "e\u{301}");
    assert_eq!(state(&ed), "cafe\u{301}|");
}

#[test]
fn replace_range_clamps_bad_ranges_instead_of_panicking() {
    let mut ed = Editor::from_text("héllo");
    ed.replace_range(ByteOffset(2)..ByteOffset(99), "!"); // starts inside "é", ends past the end
    assert_eq!(state(&ed), "h!|");
    ed.replace_range(ByteOffset(2)..ByteOffset(1), "?"); // inverted ranges are empty
    assert_eq!(state(&ed), "h!?|");
}

#[test]
fn edits_are_reported_to_change_consumers() {
    let mut ed = editor("ab\n|cd");
    let seen = ed.buffer().version();
    ed.insert_text("X");
    ed.backspace();
    ed.insert_newline();
    let changes: Vec<_> = ed
        .buffer()
        .changes_since(seen)
        .unwrap()
        .map(|c| (c.start.0, c.old_end.0, c.new_end.0, c.new_end_point.line))
        .collect();
    assert_eq!(changes, [(3, 3, 4, 1), (3, 4, 3, 1), (3, 3, 4, 2)]);
}
