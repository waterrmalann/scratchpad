//! Replace All: every match of a literal query replaced in one undo step.

use proptest::prelude::*;
use scratchpad_editor::search::CaseSensitivity::{self, Insensitive, Sensitive};
use scratchpad_editor::{Bias, ByteOffset, Editor, Selection};

fn contents(editor: &Editor) -> String {
    editor.buffer().normalized_text()
}

#[test]
fn every_match_is_replaced_ignoring_case_unless_asked() {
    let mut editor = Editor::from_text("Cat cat CAT dog");
    assert_eq!(editor.replace_all("cat", Insensitive, "fox"), 3);
    assert_eq!(contents(&editor), "fox fox fox dog");

    let mut editor = Editor::from_text("Cat cat CAT dog");
    assert_eq!(editor.replace_all("cat", Sensitive, "fox"), 1);
    assert_eq!(contents(&editor), "Cat fox CAT dog");
}

#[test]
fn one_undo_brings_back_the_text_and_the_selection() {
    let mut editor = Editor::from_text("a b a b a");
    editor.set_selection(Selection::new(ByteOffset(4), ByteOffset(5)));
    editor.replace_all("a", Insensitive, "xyz");
    assert_eq!(contents(&editor), "xyz b xyz b xyz");

    assert!(editor.undo());
    assert_eq!(contents(&editor), "a b a b a");
    assert_eq!(
        editor.selection(),
        Selection::new(ByteOffset(4), ByteOffset(5))
    );
    assert!(!editor.can_undo(), "it was a single step");
    assert!(editor.redo());
    assert_eq!(contents(&editor), "xyz b xyz b xyz");
}

#[test]
fn a_replacement_containing_the_query_is_not_replaced_again() {
    let mut editor = Editor::from_text("ab ab");
    assert_eq!(editor.replace_all("ab", Insensitive, "abab"), 2);
    assert_eq!(contents(&editor), "abab abab");
}

#[test]
fn nothing_changes_without_matches() {
    let mut editor = Editor::from_text("text");
    assert_eq!(editor.replace_all("x y", Insensitive, "z"), 0);
    assert_eq!(editor.replace_all("", Insensitive, "z"), 0);
    assert_eq!(contents(&editor), "text");
    assert!(!editor.can_undo());
}

#[test]
fn letters_whose_lowercase_differs_in_length_are_replaced_whole() {
    // "ẞ" is 3 bytes and its lowercase "ß" 2; Kelvin sign "K" is 3 bytes, "k" 1.
    let mut editor = Editor::from_text("STRAẞE \u{212a}ilo");
    editor.replace_all("straße", Insensitive, "street");
    editor.replace_all("kilo", Insensitive, "k");
    assert_eq!(contents(&editor), "street k");
}

#[test]
fn the_cursor_stays_with_its_text() {
    let cursor_after = |offset: usize| {
        let mut editor = Editor::from_text("one aa two aa three");
        editor.set_selection(Selection::cursor(ByteOffset(offset)));
        editor.replace_all("aa", Insensitive, "bbbb");
        assert_eq!(contents(&editor), "one bbbb two bbbb three");
        editor.selection()
    };
    assert_eq!(cursor_after(2), Selection::cursor(ByteOffset(2)), "before");
    assert_eq!(
        cursor_after(4),
        Selection::cursor(ByteOffset(4)),
        "at a start"
    );
    assert_eq!(cursor_after(5), Selection::cursor(ByteOffset(8)), "inside");
    assert_eq!(
        cursor_after(6),
        Selection::cursor(ByteOffset(8)),
        "at an end"
    );
    assert_eq!(
        cursor_after(9),
        Selection::cursor(ByteOffset(11)),
        "between"
    );
    assert_eq!(cursor_after(19), Selection::cursor(ByteOffset(23)), "after");
}

#[test]
fn matches_far_apart_and_close_together_are_all_replaced() {
    // Replace All edits matches that are close together as one; these are on both sides of that.
    let far = "x".repeat(3000);
    let text = format!("Ab{far}ab ab{far}AB");
    let mut editor = Editor::from_text(&text);
    let between_the_close_ones = 2 + far.len() + 2;
    editor.set_selection(Selection::cursor(ByteOffset(between_the_close_ones)));
    assert_eq!(editor.replace_all("ab", Insensitive, "c"), 4);
    assert_eq!(contents(&editor), format!("c{far}c c{far}c"));
    assert_eq!(
        editor.selection(),
        Selection::cursor(ByteOffset(between_the_close_ones - 2))
    );
    assert!(editor.undo());
    assert_eq!(contents(&editor), text);
}

#[test]
fn line_breaks_in_the_replacement_are_normalized() {
    let mut editor = Editor::from_text("a-b-c");
    editor.set_selection(Selection::cursor(ByteOffset(3)));
    editor.replace_all("-", Sensitive, "\r\n");
    assert_eq!(contents(&editor), "a\nb\nc");
    assert_eq!(editor.selection(), Selection::cursor(ByteOffset(3)));
}

/// Mostly a few letters, so the query often occurs, sometimes overlapping with itself. Some letters
/// change length when lowercased ("İ", Kelvin sign "K", "ẞ"), "e" can take a combining accent, line
/// breaks may be CRLF or a lone CR, and a long run of dashes now and then puts matches too far apart
/// for Replace All to merge them into one edit.
fn letters(len: std::ops::Range<usize>) -> impl Strategy<Value = String> {
    let dashes: &'static str = "-".repeat(1100).leak();
    let letter = prop_oneof![
        30 => prop::sample::select(
            &["a", "A", "b", "e", "\u{301}", "é", "İ", "i", "\u{212a}", "k", "ẞ", "ß", "日"][..],
        ),
        4 => prop::sample::select(&["\n", "\r\n", "\r"][..]),
        1 => Just(dashes),
    ];
    prop::collection::vec(letter, len).prop_map(|s| s.concat())
}

/// Replace All done the slow, obvious way: each match on its own, found by comparing lowercase
/// mappings char by char at every char boundary (ADR 0006). Also returns where a cursor at
/// `cursor` goes: after the replacement of a match it is inside, else with the text around it.
fn replace_naively(
    text: &str,
    query: &str,
    case: CaseSensitivity,
    replacement: &str,
    cursor: usize,
) -> (String, usize, usize) {
    let fold = |c: char| -> Vec<char> {
        match case {
            Sensitive => vec![c],
            Insensitive => c.to_lowercase().collect(),
        }
    };
    let needle: Vec<char> = query.chars().flat_map(fold).collect();
    // The end of the match starting at `start`: the text from there folds to the needle.
    let match_at = |start: usize| {
        let mut folded = Vec::new();
        for (i, c) in text[start..].char_indices() {
            folded.extend(fold(c));
            if folded.len() >= needle.len() {
                return (folded == needle).then_some(start + i + c.len_utf8());
            }
            if !needle.starts_with(&folded) {
                return None;
            }
        }
        None
    };
    let (mut out, mut count, mut new_cursor) = (String::new(), 0, None);
    let mut at = 0;
    loop {
        if at == cursor {
            new_cursor = Some(out.len());
        }
        if at == text.len() {
            break;
        }
        if let Some(end) = match_at(at) {
            out.push_str(replacement);
            if (at..end).contains(&cursor) && new_cursor.is_none() {
                new_cursor = Some(out.len());
            }
            count += 1;
            at = end;
        } else {
            let c = text[at..].chars().next().unwrap();
            out.push(c);
            at += c.len_utf8();
        }
    }
    let new_cursor = new_cursor.expect("the cursor is on a char boundary");
    (out, count, new_cursor)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Replace All, which merges nearby matches into one edit, gives exactly the text of replacing
    /// each match on its own: the standard library's literal replacement when matching case, the
    /// naive reference either way. The cursor goes with its text, undo brings back the text and
    /// the selection, and redo the replaced text.
    #[test]
    fn replace_all_replaces_each_match_on_its_own(
        text in letters(0..40),
        query in letters(1..4),
        replacement in letters(0..5),
        case in prop::sample::select(&[Sensitive, Insensitive][..]),
        cursor in any::<prop::sample::Index>(),
    ) {
        let mut editor = Editor::from_text(&text);
        let text = contents(&editor);
        editor.set_selection(Selection::cursor(ByteOffset(cursor.index(text.len() + 1))));
        let before = editor.selection();
        let replacement_text = replacement.replace("\r\n", "\n").replace('\r', "\n");
        let (expected, expected_count, cursor) =
            replace_naively(&text, &query, case, &replacement_text, before.head.0);
        if case == Sensitive {
            prop_assert_eq!(&expected, &text.replace(&query, &replacement_text));
        }

        let count = editor.replace_all(&query, case, &replacement);
        prop_assert_eq!(&contents(&editor), &expected);
        prop_assert_eq!(count, expected_count);
        if count == 0 {
            prop_assert!(!editor.can_undo());
            return Ok(());
        }
        let after = editor.selection();
        let cursor = editor.buffer().clip_offset(ByteOffset(cursor), Bias::Left);
        prop_assert_eq!(after, Selection::cursor(cursor));

        prop_assert!(editor.undo());
        prop_assert_eq!(contents(&editor), text);
        prop_assert_eq!(editor.selection(), before);
        prop_assert!(!editor.can_undo());
        prop_assert!(editor.redo());
        prop_assert_eq!(contents(&editor), expected);
        prop_assert_eq!(editor.selection(), after);
    }
}
