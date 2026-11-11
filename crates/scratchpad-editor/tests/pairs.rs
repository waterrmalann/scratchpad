//! Typing brackets and quotes: conservative auto-pairing, skipping over closers and wrapping selections.

mod common;

use common::{editor, state};
use scratchpad_editor::markdown::MarkdownState;
use scratchpad_editor::{ByteOffset, Editor};

/// Types `keys` one at a time, as the keyboard delivers them, with one Markdown state that follows the edits.
fn type_keys(editor: &mut Editor, keys: &str) {
    let mut markdown = MarkdownState::new(editor.buffer());
    for c in keys.chars() {
        editor.insert_typed(c.encode_utf8(&mut [0; 4]), &mut markdown);
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
    assert_eq!(typed("a|", "~"), "a~|");
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
fn underscore_needs_the_same_underscores_on_both_sides_and_no_word_past_them() {
    // Stacking: `__strong__`, then `___both___`.
    assert_eq!(typed("__^text|__", "_"), "___^text|___");
    // Between two `_` inside an identifier, `_` would not be emphasis.
    assert_eq!(typed("foo_^bar|_baz", "_"), "foo__|_baz");
    assert_eq!(typed("x__^bar|__", "_"), "x___|__");
    assert_eq!(typed("_^bar|__", "_"), "__|__");
}

#[test]
fn underscore_replaces_a_selection_with_whitespace_at_either_end() {
    assert_eq!(typed("^text |now", "_"), "_|now");
    assert_eq!(typed("a^ text| b", "_"), "a_| b");
    // A whole line selected with its line break, as a triple click does.
    assert_eq!(typed("^line\n|next", "_"), "_|next");
    // `~` and `*` still wrap such selections, as before.
    assert_eq!(typed("^text |now", "~"), "~^text |~now");
}

#[test]
fn underscore_reads_multibyte_and_combining_neighbours_as_letters() {
    assert_eq!(typed("é^x|", "_"), "é_|");
    assert_eq!(typed("^x|é", "_"), "_|é");
    // `e` and a combining acute accent (U+0301) are one letter.
    assert_eq!(typed("e\u{301}^x|", "_"), "e\u{301}_|");
    assert_eq!(typed("ñ ^x| ü", "_"), "ñ _^x|_ ü");
}

#[test]
fn underscore_wraps_multi_line_and_backward_selections() {
    assert_eq!(
        typed("one ^two\nthree| four", "_"),
        "one _^two\nthree|_ four"
    );
    assert_eq!(typed("|text^", "_"), "_|text^_");
    assert_eq!(typed("say |text^ now", "__"), "say __|text^__ now");
}

#[test]
fn in_code_underscore_replaces_while_brackets_star_and_tilde_still_wrap() {
    assert_eq!(
        typed("```\nfor ^i| in x\n```", "_"),
        "```\nfor _| in x\n```"
    );
    assert_eq!(typed("`a ^b| c`", "_"), "`a _| c`");
    assert_eq!(typed("    ^x| = 1", "_"), "    _| = 1");
    // An unclosed fence runs to the end of the note.
    assert_eq!(typed("```\n^x|", "_"), "```\n_|");
    // Outside the span, and in prose, the same selection wraps.
    assert_eq!(typed("`a` ^b| c", "_"), "`a` _^b|_ c");
    assert_eq!(typed("for ^i| in x", "_"), "for _^i|_ in x");
    for key in ["*", "~", "("] {
        let close = if key == "(" { ")" } else { key };
        assert_eq!(typed("`a ^b| c`", key), format!("`a {key}^b|{close} c`"));
        assert_eq!(
            typed("```\n^x|\n```", key),
            format!("```\n{key}^x|{close}\n```")
        );
    }
}

#[test]
fn in_code_covers_the_text_between_the_markers() {
    let text = "a `code` b\n```\nblock\n```\nc";
    let ed = Editor::from_text(text);
    let mut markdown = MarkdownState::new(ed.buffer());
    let at = |offset: usize| ByteOffset(offset)..ByteOffset(offset);
    let mut in_code = |range| markdown.in_code(ed.buffer(), range);
    // Just inside each backtick is in code; just outside is not.
    assert!(!in_code(at(2)));
    assert!(in_code(at(3)));
    assert!(in_code(at(7)));
    assert!(!in_code(at(8)));
    // A selection that reaches into the span is in code.
    assert!(in_code(ByteOffset(0)..ByteOffset(4)));
    assert!(!in_code(ByteOffset(0)..ByteOffset(3)));
    // The block's text is in code, its fence lines and the lines around it are not.
    let block = text.find("block").unwrap();
    assert!(in_code(at(block)));
    assert!(in_code(at(block + "block".len())));
    assert!(!in_code(at(block - 1)));
    assert!(!in_code(at(text.len())));
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
    let mut markdown = MarkdownState::new(ed.buffer());
    ed.insert_typed("(x", &mut markdown);
    assert_eq!(state(&ed), "(x|");

    let mut ed = editor("|");
    let mut markdown = MarkdownState::new(ed.buffer());
    ed.replace_and_mark(None, "か", None);
    ed.insert_typed("(", &mut markdown);
    assert_eq!(state(&ed), "(|");
}
