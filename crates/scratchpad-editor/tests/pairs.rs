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
    assert_eq!(typed("|", "```"), "```|");
    assert_eq!(typed("|", "`code`"), "`code`|");
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
