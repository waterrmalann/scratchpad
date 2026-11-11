//! Typing brackets and quotes: conservative auto-pairing, skipping over closers and wrapping selections.

mod common;

use common::{editor, state};
use scratchpad_editor::Editor;

fn type_keys(editor: &mut Editor, keys: &str) {
    for c in keys.chars() {
        editor.insert_typed(c.encode_utf8(&mut [0; 4]));
    }
}

/// Types `keys` into marked text and returns the resulting marked text.
fn typed(marked: &str, keys: &str) -> String {
    let mut ed = editor(marked);
    type_keys(&mut ed, keys);
    state(&ed)
}

#[test]
fn typing_a_call_closes_and_then_steps_over_the_parenthesis() {
    let mut ed = editor("|");
    type_keys(&mut ed, "f(");
    assert_eq!(state(&ed), "f(|)");
    type_keys(&mut ed, "x");
    assert_eq!(state(&ed), "f(x|)");
    type_keys(&mut ed, ")");
    assert_eq!(state(&ed), "f(x)|");
    assert_eq!(typed("|", "[{a}]"), "[{a}]|");
}

#[test]
fn openers_pair_only_before_whitespace_closers_punctuation_or_the_line_end() {
    assert_eq!(typed("a | b", "("), "a (|) b");
    assert_eq!(typed("|)", "["), "[|])");
    for punctuation in [".", ",", ";", ":", "!", "?", "]", "}"] {
        assert_eq!(
            typed(&format!("|{punctuation}"), "{"),
            format!("{{|}}{punctuation}")
        );
    }
    assert_eq!(typed("|-", "("), "(|-");
    assert_eq!(typed("|\nx", "("), "(|)\nx");
    assert_eq!(typed("fo|o", "("), "fo(|o");
    assert_eq!(typed("|word", "\""), "\"|word");
}

#[test]
fn quotes_after_letters_stay_single_so_apostrophes_work() {
    assert_eq!(typed("don|t", "'"), "don'|t");
    assert_eq!(typed("it|", "'s"), "it's|");
    assert_eq!(typed("say|", "\""), "say\"|");
    assert_eq!(typed("say |", "\"hi\""), "say \"hi\"|");
}

#[test]
fn three_backticks_type_a_fence() {
    assert_eq!(typed("|", "`"), "`|`");
    assert_eq!(typed("|", "`code`"), "`code`|");
}

#[test]
fn the_third_backtick_opens_and_closes_a_code_block() {
    assert_eq!(typed("|", "```"), "```\n|\n```");
    assert_eq!(typed("  |", "```"), "  ```\n  |\n  ```");
    // Not alone on the line: an inline span.
    assert_eq!(typed("a |", "```"), "a ```|");
    assert_eq!(typed("|x", "```"), "```|x");
    let mut ed = editor("|");
    type_keys(&mut ed, "```");
    assert!(ed.undo());
    assert_eq!(state(&ed), "``|");
}

#[test]
fn closers_step_over_only_while_the_line_stays_balanced() {
    assert_eq!(typed("a\"b|\"", "\""), "a\"b\"|");
    // Stepping over would leave `f(` or the first quote unclosed.
    assert_eq!(typed("f(g(x|)", ")"), "f(g(x)|)");
    assert_eq!(typed("\"a\"b|\"", "\""), "\"a\"b\"|\"");
    // A stray closer is typed as asked.
    assert_eq!(typed("foo|)", ")"), "foo)|)");
}

#[test]
fn backspace_inside_an_empty_pair_deletes_both() {
    let mut ed = editor("x(|)");
    ed.backspace_typed();
    assert_eq!(state(&ed), "x|");
    ed.backspace_typed();
    assert_eq!(state(&ed), "|");

    let mut ed = editor("(a|)");
    ed.backspace_typed();
    assert_eq!(state(&ed), "(|)");
    let mut ed = editor("(^a|)");
    ed.backspace_typed();
    assert_eq!(state(&ed), "(|)");
}

#[test]
fn typing_an_opener_or_star_over_a_selection_wraps_it() {
    assert_eq!(typed("say ^hi| now", "("), "say (^hi|) now");
    assert_eq!(typed("|hi^", "\""), "\"|hi^\"");
    assert_eq!(typed("^bold|", "**"), "**^bold|**");
    assert_eq!(typed("^x|", "a"), "a|");
}

#[test]
fn tilde_over_a_selection_wraps_it_and_twice_strikes_it_through() {
    let mut ed = editor("say ^text| now");
    type_keys(&mut ed, "~");
    assert_eq!(state(&ed), "say ~^text|~ now");
    type_keys(&mut ed, "~");
    assert_eq!(state(&ed), "say ~~^text|~~ now");
    // A backward selection stays backward.
    assert_eq!(typed("|text^", "~~"), "~~|text^~~");
    // Typing on replaces the struck-through text, inside the markers.
    assert_eq!(typed("^text|", "~~x"), "~~x|~~");
}

#[test]
fn each_wrap_is_one_undo_step() {
    let mut ed = editor("a ^text| b");
    type_keys(&mut ed, "~~");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a ~^text|~ b");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a ^text| b");
    assert!(!ed.undo());

    let mut ed = editor("^word|");
    type_keys(&mut ed, "_");
    assert!(ed.undo());
    assert_eq!(state(&ed), "^word|");
}

#[test]
fn tilde_without_a_selection_is_typed_as_it_is() {
    assert_eq!(typed("|", "~"), "~|");
    assert_eq!(typed("a |b", "~~"), "a ~~|b");
    assert_eq!(typed("|", "~~struck~~"), "~~struck~~|");
}

#[test]
fn underscore_wraps_a_selection_that_it_would_make_emphasis() {
    assert_eq!(typed("say ^text| now", "_"), "say _^text|_ now");
    assert_eq!(typed("^text|", "__"), "__^text|__");
    assert_eq!(typed("(^two words|)", "_"), "(_^two words|_)");
}

#[test]
fn underscore_replaces_identifiers_spaces_and_parts_of_words() {
    // An identifier is replaced, not wrapped.
    assert_eq!(typed("^foo_bar| = 1", "_"), "_| = 1");
    // Joining words with `_`.
    assert_eq!(typed("foo^ |bar", "_"), "foo_|bar");
    assert_eq!(typed("foo^-|bar", "_"), "foo_|bar");
    // Inside a word, or next to an `_` on one side only, `_` would not be emphasis.
    assert_eq!(typed("un^bel|ievable", "_"), "un_|ievable");
    assert_eq!(typed("^foo|_bar", "_"), "_|_bar");
    assert_eq!(typed("foo_^bar|", "_"), "foo__|");
    // Without a selection `_` is typed as it is.
    assert_eq!(typed("snake|", "_case"), "snake_case|");
}

#[test]
fn wrapping_ignores_code_spans_and_blocks_like_star() {
    // No Markdown context is consulted: in code, `~` and `_` wrap as `*` does.
    for key in ["*", "~", "_"] {
        assert_eq!(typed("`a ^b| c`", key), format!("`a {key}^b|{key} c`"));
        assert_eq!(
            typed("```\n^x|\n```", key),
            format!("```\n{key}^x|{key}\n```")
        );
    }
    // ...and the identifier rule for `_` is the same in code as in prose.
    assert_eq!(typed("`^foo_bar|`", "_"), "`_|`");
}

#[test]
fn star_is_not_paired_without_a_selection() {
    assert_eq!(typed("|", "* item"), "* item|");
    assert_eq!(typed("2 |", "*"), "2 *|");
}

#[test]
fn pairing_undoes_in_one_step_and_plain_typing_still_groups() {
    let mut ed = editor("|");
    type_keys(&mut ed, "ab");
    assert!(ed.undo());
    assert_eq!(state(&ed), "|");

    let mut ed = editor("a |");
    type_keys(&mut ed, "(");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a |");
    assert!(!ed.undo());
}

#[test]
fn multi_char_text_and_compositions_are_inserted_verbatim() {
    let mut ed = editor("|");
    ed.insert_typed("(x");
    assert_eq!(state(&ed), "(x|");

    let mut ed = editor("|");
    ed.replace_and_mark(None, "か", None);
    ed.insert_typed("(");
    assert_eq!(state(&ed), "(|");
}
