//! Find in document: matching rules and stepping through matches.

use std::ops::Range;

use proptest::prelude::*;
use scratchpad_editor::search::{
    CaseSensitivity, adjust_matches, find_all, next_match, prev_match,
};
use scratchpad_editor::{ByteOffset, Editor};

fn ranges(text: &str, query: &str, case: CaseSensitivity) -> Vec<(usize, usize)> {
    find_all(text, query, case)
        .into_iter()
        .map(|r| (r.start.0, r.end.0))
        .collect()
}

fn matched<'a>(text: &'a str, query: &str) -> Vec<&'a str> {
    find_all(text, query, CaseSensitivity::default())
        .into_iter()
        .map(|r| &text[r.start.0..r.end.0])
        .collect()
}

#[test]
fn search_ignores_case_by_default() {
    assert_eq!(
        matched("Hello hello HELLO hElLo", "hello"),
        ["Hello", "hello", "HELLO", "hElLo"]
    );
    assert_eq!(matched("Straße STRASSE", "STRAßE"), ["Straße"]);
    assert_eq!(
        matched("ΟΔΥΣΣΕΥΣ οδυσσευσ", "Οδυσσευσ"),
        ["ΟΔΥΣΣΕΥΣ", "οδυσσευσ"]
    );
}

#[test]
fn case_sensitive_search_matches_exactly() {
    assert_eq!(
        ranges("Hello hello", "hello", CaseSensitivity::Sensitive),
        [(6, 11)]
    );
}

#[test]
fn matches_do_not_overlap() {
    assert_eq!(
        ranges("aaaaa", "aa", CaseSensitivity::Insensitive),
        [(0, 2), (2, 4)]
    );
}

#[test]
fn an_empty_query_matches_nothing() {
    assert!(find_all("text", "", CaseSensitivity::Insensitive).is_empty());
    assert!(find_all("", "x", CaseSensitivity::Insensitive).is_empty());
}

#[test]
fn matches_whose_lowercase_has_a_different_length_map_back_to_the_original_text() {
    // The Kelvin sign (3 bytes) lowercases to ASCII "k" (1 byte).
    assert_eq!(matched("5 \u{212a} or 5 k", "K"), ["\u{212a}", "k"]);
    // Capital sharp s (3 bytes) lowercases to "ß" (2 bytes).
    assert_eq!(matched("GROẞ groß", "ß"), ["ẞ", "ß"]);
    // "İ" (2 bytes) lowercases to "i" + combining dot (3 bytes).
    assert_eq!(matched("İstanbul", "i\u{307}stanbul"), ["İstanbul"]);
}

#[test]
fn a_query_cannot_match_part_of_a_characters_lowercase_expansion() {
    // "İ" lowercases to two chars; matching only the first would split the original char.
    assert!(find_all("İ", "i", CaseSensitivity::Insensitive).is_empty());
}

#[test]
fn search_handles_emoji_cjk_and_rtl() {
    assert_eq!(matched("日本語のテキスト、日本", "日本"), ["日本", "日本"]);
    assert_eq!(matched("I 👍🏽 this 👍🏽", "👍🏽"), ["👍🏽", "👍🏽"]);
    assert_eq!(matched("שלום עולם", "עולם"), ["עולם"]);
}

#[test]
fn next_and_previous_match_wrap_around() {
    let text = "one two one two one";
    let matches = find_all(text, "one", CaseSensitivity::Insensitive);
    assert_eq!(matches.len(), 3);

    // From the cursor at the start, then stepping from the end of each match.
    assert_eq!(next_match(&matches, ByteOffset(0)), Some(0));
    assert_eq!(next_match(&matches, matches[0].end), Some(1));
    assert_eq!(
        next_match(&matches, matches[2].end),
        Some(0),
        "wraps to the first match"
    );
    assert_eq!(
        next_match(&matches, ByteOffset(5)),
        Some(1),
        "cursor between matches"
    );

    assert_eq!(prev_match(&matches, matches[1].start), Some(0));
    assert_eq!(
        prev_match(&matches, matches[0].start),
        Some(2),
        "wraps to the last match"
    );
    assert_eq!(prev_match(&matches, ByteOffset(text.len())), Some(2));

    assert_eq!(next_match(&[], ByteOffset(0)), None);
    assert_eq!(prev_match(&[], ByteOffset(0)), None);
}

/// Searches `text`, applies `edits` (byte range, replacement) one after another and returns the adjusted
/// matches as strings of the edited text.
fn matches_after_edits(text: &str, query: &str, edits: &[(Range<usize>, &str)]) -> Vec<String> {
    let mut editor = Editor::from_text(text);
    let mut matches = find_all(text, query, CaseSensitivity::Insensitive);
    for (range, replacement) in edits {
        let version = editor.buffer().version();
        editor.replace_range(ByteOffset(range.start)..ByteOffset(range.end), replacement);
        for change in editor.buffer().changes_since(version).unwrap() {
            adjust_matches(&mut matches, change);
        }
    }
    let text = editor.buffer().normalized_text();
    matches
        .iter()
        .map(|m| format!("{}@{}", &text[m.start.0..m.end.0], m.start.0))
        .collect()
}

#[test]
fn matches_follow_edits_until_the_next_search() {
    let text = "cat. Cat? cat!";
    assert_eq!(
        matches_after_edits(text, "cat", &[(0..0, "a ")]),
        ["cat@2", "Cat@7", "cat@12"],
        "an insertion before every match moves them all"
    );
    assert_eq!(
        matches_after_edits(text, "cat", &[(6..7, "")]),
        ["cat@0", "cat@9"],
        "an edit inside a match drops it and moves the ones after it"
    );
    assert_eq!(
        matches_after_edits(text, "cat", &[(3..3, "s"), (6..6, "big ")]),
        ["cat@0", "Cat@10", "cat@15"],
        "edits right after or right before a match keep it"
    );
    assert_eq!(
        matches_after_edits(text, "cat", &[(2..12, "")]),
        Vec::<String>::new(),
        "a deletion across matches drops them"
    );
}

fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// Characters whose lowercase mapping has a different length or collides with another character's.
const CASE_TRAPS: &[&str] = &[
    "İ", "i", "I", "\u{307}", "\u{212a}", "k", "K", "ẞ", "ß", "s", "S", "Σ", "σ", "ς", "a", " ",
    "日",
];

fn case_traps(len: Range<usize>) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(CASE_TRAPS), len).prop_map(|s| s.concat())
}

/// A text and a query that is often (but not always) a substring of it with ASCII case flipped.
fn text_and_query() -> impl Strategy<Value = (String, String)> {
    (
        prop_oneof![any::<String>(), case_traps(0..20)],
        any::<prop::sample::Index>(),
        any::<prop::sample::Index>(),
        prop_oneof![any::<String>(), case_traps(1..4)],
        0..3u8,
    )
        .prop_map(|(text, a, b, random, mode)| {
            let bounds: Vec<usize> = text
                .char_indices()
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            let (a, b) = (a.get(&bounds), b.get(&bounds));
            let substring = &text[*a.min(b)..*a.max(b)];
            let query = match mode {
                0 => substring.to_string(),
                1 => substring
                    .chars()
                    .map(|c| {
                        if c.is_ascii_lowercase() {
                            c.to_ascii_uppercase()
                        } else {
                            c.to_ascii_lowercase()
                        }
                    })
                    .collect(),
                _ => random,
            };
            (text, query)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Every match is a valid, ordered, non-overlapping range of the text that equals the query ignoring
    /// case, and a query that occurs in the text is always found.
    #[test]
    fn matches_are_valid_ranges_equal_to_the_query_ignoring_case((text, query) in text_and_query()) {
        let matches = find_all(&text, &query, CaseSensitivity::Insensitive);
        let mut previous_end = 0;
        for m in &matches {
            prop_assert!(previous_end <= m.start.0 && m.start < m.end);
            prop_assert!(text.is_char_boundary(m.start.0) && text.is_char_boundary(m.end.0));
            prop_assert_eq!(fold(&text[m.start.0..m.end.0]), fold(&query));
            previous_end = m.end.0;
        }
        if !query.is_empty() && text.to_ascii_lowercase().contains(&query.to_ascii_lowercase()) {
            prop_assert!(!matches.is_empty());
        }
    }

    /// After any edits, undos and redos, exactly the matches whose text no edit has touched are left,
    /// where their text moved to, so they can be highlighted until the text is searched again. (A
    /// match an edit touched stays dropped even if undo brings its text back.)
    #[test]
    fn adjusted_matches_are_the_untouched_matches_where_their_text_went(
        (text, query) in text_and_query(),
        edits in prop::collection::vec(edit(), 1..8),
    ) {
        let mut editor = Editor::from_text(&text);
        let text = editor.buffer().normalized_text();
        let original = find_all(&text, &query, CaseSensitivity::Insensitive);
        let mut matches = original.clone();
        // For each byte of the text, the offset it had before the edits; `None` if inserted since.
        let mut origins: Vec<Option<usize>> = (0..text.len()).map(Some).collect();
        let mut untouched = vec![true; original.len()];
        for edit in edits {
            let version = editor.buffer().version();
            match edit {
                Edit::Replace(a, b, replacement) => {
                    let len = editor.buffer().len() + 1;
                    let (a, b) = (a.index(len), b.index(len));
                    editor.replace_range(ByteOffset(a.min(b))..ByteOffset(a.max(b)), &replacement);
                }
                Edit::Undo => _ = editor.undo(),
                Edit::Redo => _ = editor.redo(),
            }
            for change in editor.buffer().changes_since(version).unwrap() {
                adjust_matches(&mut matches, change);
                let inserted = change.new_end.0 - change.start.0;
                origins.splice(change.start.0..change.old_end.0, std::iter::repeat_n(None, inserted));
                for (m, untouched) in original.iter().zip(&mut untouched) {
                    *untouched &= moved_to(m, &origins).is_some();
                }
            }
        }
        let text = editor.buffer().normalized_text();
        prop_assert_eq!(origins.len(), text.len());
        let expected: Vec<_> = original
            .iter()
            .zip(untouched)
            .filter(|(_, untouched)| *untouched)
            .filter_map(|(m, _)| moved_to(m, &origins))
            .collect();
        prop_assert_eq!(&matches, &expected);
        for m in &matches {
            prop_assert_eq!(fold(&text[m.start.0..m.end.0]), fold(&query));
        }
    }
}

/// Where the bytes of `m` are now, if they are still side by side and in order.
fn moved_to(m: &Range<ByteOffset>, origins: &[Option<usize>]) -> Option<Range<ByteOffset>> {
    let start = origins.iter().position(|&o| o == Some(m.start.0))?;
    let end = start + (m.end.0 - m.start.0);
    let bytes = origins.get(start..end)?;
    bytes
        .iter()
        .zip(m.start.0..)
        .all(|(&o, i)| o == Some(i))
        .then_some(ByteOffset(start)..ByteOffset(end))
}

#[derive(Debug, Clone)]
enum Edit {
    Replace(prop::sample::Index, prop::sample::Index, String),
    Undo,
    Redo,
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        4 => (any::<prop::sample::Index>(), any::<prop::sample::Index>(), case_traps(0..3))
            .prop_map(|(a, b, replacement)| Edit::Replace(a, b, replacement)),
        1 => Just(Edit::Undo),
        1 => Just(Edit::Redo),
    ]
}
