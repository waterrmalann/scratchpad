//! Typed replacements (ADR 0180): `->` becomes → as it is typed, Backspace or undo takes it back, and code,
//! pasted text and Markdown syntax are left alone.

mod common;

use common::{editor, state};
use scratchpad_editor::markdown::MarkdownState;
use scratchpad_editor::replacements::RULES;
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
fn every_rule_is_replaced_on_its_last_typed_char() {
    let table = [
        ("->", "→"),
        ("<-", "←"),
        ("<->", "↔"),
        ("=>", "⇒"),
        ("<=", "≤"),
        ("<=>", "⇔"),
        (">=", "≥"),
        ("!=", "≠"),
        ("--", "—"),
        ("...", "…"),
        ("(c)", "©"),
        ("(r)", "®"),
        ("(tm)", "™"),
        ("+-", "±"),
    ];
    assert_eq!(table.len(), RULES.len(), "every rule is in this table");
    for (from, to) in table {
        assert_eq!(typed("a |", from), format!("a {to}|"), "{from}");
        assert_eq!(
            typed("a |", &format!("{from} b")),
            format!("a {to} b|"),
            "{from}"
        );
    }
    // Not before the last char.
    assert_eq!(typed("a |", "<"), "a <|");
    assert_eq!(typed("a |", ".."), "a ..|");
    assert_eq!(typed("a |", "(t"), "a (t|)");
    // In the middle of a line and between words.
    assert_eq!(typed("x| y", "->"), "x→| y");
    assert_eq!(typed("|", "1...9"), "1…9|");
    assert_eq!(typed("|", "word--word"), "word—word|");
}

#[test]
fn symbols_in_brackets_are_replaced_when_the_paired_closer_is_stepped_over() {
    // `(` pairs, `c` goes inside, and `)` steps over the closer.
    let mut ed = editor("Copyright |");
    type_keys(&mut ed, "(c");
    assert_eq!(state(&ed), "Copyright (c|)");
    type_keys(&mut ed, ")");
    assert_eq!(state(&ed), "Copyright ©|");
    assert_eq!(typed("|", "(tm) and (r)"), "™ and ®|");
}

#[test]
fn the_longest_match_wins_even_after_a_shorter_one_was_replaced() {
    // `<-` is replaced first; `>` then makes it `<->`.
    let mut ed = editor("a |");
    type_keys(&mut ed, "<-");
    assert_eq!(state(&ed), "a ←|");
    type_keys(&mut ed, ">");
    assert_eq!(state(&ed), "a ↔|");

    let mut ed = editor("a |");
    type_keys(&mut ed, "<=");
    assert_eq!(state(&ed), "a ≤|");
    type_keys(&mut ed, ">");
    assert_eq!(state(&ed), "a ⇔|");

    // A char that makes no longer rule is typed after the symbol.
    assert_eq!(typed("a |", "<--"), "a ←-|");
    assert_eq!(typed("a |", "->>"), "a →>|");
    assert_eq!(typed("a |", "-->"), "a —>|");
    assert_eq!(typed("a |", "...."), "a ….|");
}

#[test]
fn undo_brings_back_what_was_typed() {
    let mut ed = editor("a |");
    type_keys(&mut ed, "->");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a ->|");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a |");
    assert!(ed.redo());
    assert!(ed.redo());
    assert_eq!(state(&ed), "a →|");

    // `<->` typed as `<-` then `>` undoes to `<->` in one step, not to `←>`.
    let mut ed = editor("a |");
    type_keys(&mut ed, "<->");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a <->|");

    // Typing on after a replacement is a step of its own.
    let mut ed = editor("|");
    type_keys(&mut ed, "x -> y");
    assert!(ed.undo());
    assert_eq!(state(&ed), "x →|");
    assert!(ed.undo());
    assert_eq!(state(&ed), "x ->|");
}

#[test]
fn backspace_right_after_a_replacement_brings_back_the_text_in_one_step() {
    let mut ed = editor("a |");
    type_keys(&mut ed, "->");
    ed.backspace_typed();
    assert_eq!(state(&ed), "a ->|");
    // Then it deletes as usual.
    ed.backspace_typed();
    assert_eq!(state(&ed), "a -|");

    for rule in RULES {
        let mut ed = editor("a |");
        type_keys(&mut ed, rule.from);
        ed.backspace_typed();
        assert_eq!(state(&ed), format!("a {}|", rule.from));
    }

    // Undo after it brings the symbol back.
    let mut ed = editor("a |");
    type_keys(&mut ed, "<->");
    ed.backspace_typed();
    assert_eq!(state(&ed), "a <->|");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a ↔|");
}

#[test]
fn the_char_typed_after_a_backspace_revert_replaces_nothing() {
    let mut ed = editor("a |");
    type_keys(&mut ed, "--");
    ed.backspace_typed();
    type_keys(&mut ed, "-");
    assert_eq!(state(&ed), "a ---|");
    // Later ones do again.
    type_keys(&mut ed, " ->");
    assert_eq!(state(&ed), "a --- →|");

    let mut ed = editor("a |");
    type_keys(&mut ed, "<-");
    ed.backspace_typed();
    type_keys(&mut ed, ">");
    assert_eq!(state(&ed), "a <->|");
}

#[test]
fn backspace_deletes_as_usual_once_the_cursor_moved_or_the_text_changed() {
    let mut ed = editor("a |");
    type_keys(&mut ed, "->");
    ed.move_to(ByteOffset(0), false);
    ed.move_to(ByteOffset("a →".len()), false);
    ed.backspace_typed();
    assert_eq!(state(&ed), "a |");

    let mut ed = editor("a |");
    type_keys(&mut ed, "->");
    ed.replace_range(ByteOffset(0)..ByteOffset(1), "b");
    ed.move_to(ByteOffset("b →".len()), false);
    ed.backspace_typed();
    assert_eq!(state(&ed), "b |");

    let mut ed = editor("a |");
    type_keys(&mut ed, "-> x");
    ed.backspace_typed();
    ed.backspace_typed();
    ed.backspace_typed();
    assert_eq!(state(&ed), "a |");
}

#[test]
fn nothing_is_replaced_in_code() {
    // A fenced block, also unclosed, and an indented one.
    assert_eq!(typed("```\n|\n```", "a -> b"), "```\na -> b|\n```");
    assert_eq!(typed("```rust\n|", "x != y..."), "```rust\nx != y...|");
    assert_eq!(typed("text\n\n    |", "a => b"), "text\n\n    a => b|");
    // A span, between its backticks.
    assert_eq!(typed("use `|` here", "a->b"), "use `a->b|` here");
    // A span whose backtick was paired, and one being typed whose closing backtick is not there yet.
    assert_eq!(typed("say |", "`x->"), "say `x->|`");
    assert_eq!(typed("say|", "`x->"), "say`x->|");
    // Past a closed span, prose again.
    assert_eq!(typed("`code` |", "->"), "`code` →|");
    assert_eq!(typed("```\ncode\n```\n|", "->"), "```\ncode\n```\n→|");
}

#[test]
fn pasted_composed_and_programmatic_text_is_not_replaced() {
    let mut ed = editor("a |");
    ed.paste("-> ... (c)");
    assert_eq!(state(&ed), "a -> ... (c)|");

    let mut ed = editor("a -|");
    ed.insert_text(">");
    assert_eq!(state(&ed), "a ->|");

    // An IME composition, committed.
    let mut ed = editor("a -|");
    ed.replace_and_mark(None, ">", None);
    ed.insert_text(">");
    assert_eq!(state(&ed), "a ->|");
    // Typed while composing goes in as it is.
    let mut ed = editor("a -|");
    ed.replace_and_mark(None, "x", None);
    type_keys(&mut ed, ">");
    assert_eq!(state(&ed), "a ->|");

    let mut ed = editor("a -|");
    ed.replace_range(ByteOffset(3)..ByteOffset(3), ">");
    assert_eq!(state(&ed), "a ->|");

    // Several chars at once are not a keystroke.
    let mut ed = editor("a |");
    ed.insert_typed("->", &mut MarkdownState::new(ed.buffer()));
    assert_eq!(state(&ed), "a ->|");
}

#[test]
fn typing_over_a_selection_still_wraps_or_replaces_it() {
    assert_eq!(typed("a ^b|", "("), "a (^b|)");
    assert_eq!(typed("a ^b|", "~"), "a ~^b|~");
    // The selection is replaced by the char, and nothing else.
    assert_eq!(typed("a -^b|", ">"), "a ->|");
    assert_eq!(typed("a .^.|", "."), "a ..|");
}

#[test]
fn dashes_wait_while_the_line_could_become_a_thematic_break_or_front_matter() {
    assert_eq!(typed("|", "---"), "---|");
    assert_eq!(typed("text\n|", "---"), "text\n---|");
    assert_eq!(typed("|", "- - -"), "- - -|");
    // Table delimiter rows (`|` is the cursor in marked text, so these are typed into an empty note).
    for row in ["|---|", "| -- | :--: |"] {
        let mut ed = editor("|");
        type_keys(&mut ed, row);
        assert_eq!(ed.buffer().normalized_text(), row);
    }
    // Elsewhere on a line they are a dash.
    assert_eq!(typed("|", "a -- b"), "a — b|");
    assert_eq!(typed("|", "- item -- x"), "- item — x|");
}

#[test]
fn front_matter_is_left_alone() {
    let mut ed = editor("|");
    type_keys(&mut ed, "---\ntitle: a -> b...\n");
    assert_eq!(state(&ed), "---\ntitle: a -> b...\n|");
    type_keys(&mut ed, "---\nNow -> here");
    assert_eq!(state(&ed), "---\ntitle: a -> b...\n---\nNow → here|");
    // Closed by `...` too.
    assert_eq!(typed("---\na: 1\n...\n|", "x->"), "---\na: 1\n...\nx→|");
}

#[test]
fn html_comments_autolinks_and_addresses_are_left_alone() {
    assert_eq!(typed("|", "<!-- a -> b -->"), "<!-- a -> b -->|");
    assert_eq!(typed("<!--\n|", "a -> b\n-->"), "<!--\na -> b\n-->|");
    // After the comment, prose.
    assert_eq!(typed("|", "<!-- x --> ->"), "<!-- x --> →|");
    assert_eq!(
        typed("|", "<https://example.com/a->"),
        "<https://example.com/a->|"
    );
    assert_eq!(
        typed("|", "see https://example.com/a...b"),
        "see https://example.com/a...b|"
    );
    assert_eq!(typed("|", "path src/.../x"), "path src/.../x|");
    assert_eq!(typed("|", "mail me@x-->"), "mail me@x-->|");
}

#[test]
fn symbols_in_code_like_text_and_enumerations_are_left_alone() {
    assert_eq!(typed("|", "f(c"), "f(c|)");
    let mut ed = editor("f(c|)");
    type_keys(&mut ed, ")");
    assert_eq!(state(&ed), "f(c)|");
    assert_eq!(typed("[link](|", "c)"), "[link](c)|");
    assert_eq!(
        typed("(a) one, (b) two, |", "(c)"),
        "(a) one, (b) two, (c)|"
    );
}

#[test]
fn a_letter_right_after_a_dash_that_starts_a_word_makes_it_a_flag() {
    assert_eq!(typed("use |", "--verbose"), "use --verbose|");
    assert_eq!(typed("use |", "--2"), "use --2|");
    // After a word, or followed by a space, it stays a dash.
    assert_eq!(typed("|", "well--yes"), "well—yes|");
    assert_eq!(typed("|", "a -- b"), "a — b|");
    // One undo step back is the flag as typed.
    let mut ed = editor("use |");
    type_keys(&mut ed, "--v");
    assert!(ed.undo());
    assert_eq!(state(&ed), "use --|");
}

#[test]
fn replacements_can_be_turned_off() {
    let mut ed = editor("a |");
    assert!(ed.typed_replacements());
    ed.set_typed_replacements(false);
    assert!(!ed.typed_replacements());
    type_keys(&mut ed, "-> <-> -- ... (c)");
    assert_eq!(state(&ed), "a -> <-> -- ... (c)|");

    // Turned off right after one, Backspace deletes as usual.
    let mut ed = editor("a |");
    type_keys(&mut ed, "->");
    ed.set_typed_replacements(false);
    ed.backspace_typed();
    assert_eq!(state(&ed), "a |");
}

#[test]
fn inside_a_transaction_the_replacement_joins_it() {
    let mut ed = editor("a |");
    ed.transact(|ed| type_keys(ed, "<->"));
    assert_eq!(state(&ed), "a ↔|");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a |");
}

#[test]
fn link_destinations_are_left_alone() {
    assert_eq!(
        typed("|", "[draft](notes--v2.md)"),
        "[draft](notes--v2.md)|"
    );
    assert_eq!(typed("|", "[x](a->b)"), "[x](a->b)|");
    // After the link, prose again.
    assert_eq!(typed("|", "[x](a) -> b"), "[x](a) → b|");
}

#[test]
fn a_leading_rule_without_front_matter_after_it_does_not_stop_replacements() {
    assert_eq!(
        typed("---\nIntro paragraph\n|", "a -> b"),
        "---\nIntro paragraph\na → b|"
    );
    assert_eq!(typed("---\n|", "a -> b"), "---\na → b|");
    // Front matter still being typed, before its closing line, is left alone.
    assert_eq!(
        typed("|", "---\ntitle: a -> b\ntags:\n  - x -> y"),
        "---\ntitle: a -> b\ntags:\n  - x -> y|"
    );
    // A blank line ends it.
    assert_eq!(
        typed("---\ntitle: a\n\n|", "x -> y"),
        "---\ntitle: a\n\nx → y|"
    );
}

#[test]
fn escaped_backticks_and_longer_code_spans_are_counted_as_markdown_does() {
    assert_eq!(typed(r"a \` b |", "->"), r"a \` b →|");
    assert_eq!(typed("``a ` b`` |", "->"), "``a ` b`` →|");
    assert_eq!(typed("``a ` b |", "->"), "``a ` b ->|");
    // The whole line is read, even when long: a span closed far back, and one still open.
    let long = "x".repeat(600);
    assert_eq!(typed(&format!("`{long}` |"), "->"), format!("`{long}` →|"));
    assert_eq!(typed(&format!("`{long} |"), "->"), format!("`{long} ->|"));
}

#[test]
fn plus_minus_after_a_plus_stays() {
    assert_eq!(typed("|", "C++-style"), "C++-style|");
    assert_eq!(typed("|", "5 +- 1"), "5 ± 1|");
}

#[test]
fn a_dash_held_off_on_a_list_marker_line_becomes_one_once_text_follows() {
    assert_eq!(typed("|", "- -- item"), "- — item|");
    assert_eq!(typed("|", "-- signed"), "— signed|");
    // Still a thematic break, a flag, or a table's delimiter row.
    assert_eq!(typed("|", "- -- -"), "- -- -|");
    assert_eq!(typed("|", "- --flag"), "- --flag|");
    let mut ed = editor("|");
    type_keys(&mut ed, "| -- | x");
    assert_eq!(ed.buffer().normalized_text(), "| -- | x");
    // Its own undo step, after the char that made it a dash.
    let mut ed = editor("|");
    type_keys(&mut ed, "- -- i");
    assert!(ed.undo());
    assert_eq!(state(&ed), "- -- i|");
}

#[test]
fn after_undo_and_redo_a_replacement_is_no_longer_extended() {
    // Redo brings the symbol back, but not the memory of what it replaced (ADR 0180).
    let mut ed = editor("a |");
    type_keys(&mut ed, "<-");
    assert!(ed.undo());
    assert!(ed.redo());
    assert_eq!(state(&ed), "a ←|");
    type_keys(&mut ed, ">");
    assert_eq!(state(&ed), "a ←>|");
}

#[test]
fn toggling_the_setting_keeps_a_backspace_revert_from_being_replaced_again() {
    let mut ed = editor("a |");
    type_keys(&mut ed, "--");
    ed.backspace_typed();
    ed.set_typed_replacements(false);
    ed.set_typed_replacements(true);
    type_keys(&mut ed, "-");
    assert_eq!(state(&ed), "a ---|");
}
