//! Undo/redo sessions, undo grouping and IME composition.

mod common;

use common::{editor, state};
use scratchpad_editor::{ByteOffset, Editor, Motion, Selection, UNDO_HISTORY_BUDGET_BYTES};

fn type_chars(editor: &mut Editor, text: &str) {
    for c in text.chars() {
        editor.insert_text(c.encode_utf8(&mut [0; 4]));
    }
}

/// Undoes until there is nothing left, recording the state after each step.
fn undo_all(editor: &mut Editor) -> Vec<String> {
    let mut states = Vec::new();
    while editor.undo() {
        states.push(state(editor));
    }
    states
}

#[test]
fn typing_a_phrase_undoes_in_one_step_and_redoes() {
    let mut ed = editor("|");
    type_chars(&mut ed, "hello world");
    assert!(ed.undo());
    assert_eq!(state(&ed), "|");
    assert!(!ed.undo());
    assert!(ed.redo());
    assert_eq!(state(&ed), "hello world|");
    assert!(!ed.redo());
}

#[test]
fn a_newline_is_its_own_undo_step() {
    let mut ed = editor("|");
    type_chars(&mut ed, "one");
    ed.insert_newline();
    type_chars(&mut ed, "two");
    assert_eq!(undo_all(&mut ed), ["one\n|", "one|", "|"]);
}

#[test]
fn moving_the_cursor_starts_a_new_undo_step() {
    let mut ed = editor("|");
    type_chars(&mut ed, "ab");
    ed.move_cursor(Motion::Left, false);
    type_chars(&mut ed, "X");
    assert_eq!(undo_all(&mut ed), ["a|b", "|"]);

    // Even if the cursor comes back to where it was.
    let mut ed = editor("|");
    type_chars(&mut ed, "ab");
    ed.move_cursor(Motion::Left, false);
    ed.move_cursor(Motion::Right, false);
    type_chars(&mut ed, "c");
    assert_eq!(undo_all(&mut ed), ["ab|", "|"]);
}

#[test]
fn switching_between_typing_and_deleting_starts_a_new_undo_step() {
    let mut ed = editor("|");
    type_chars(&mut ed, "abc");
    ed.backspace();
    ed.backspace();
    type_chars(&mut ed, "xy");
    assert_eq!(state(&ed), "axy|");
    assert_eq!(undo_all(&mut ed), ["a|", "abc|", "|"]);
}

#[test]
fn runs_of_backspace_or_delete_undo_together() {
    let mut ed = editor("hello |world");
    ed.backspace();
    ed.backspace();
    ed.delete_forward();
    ed.delete_forward();
    assert_eq!(state(&ed), "hell|rld");
    assert_eq!(undo_all(&mut ed), ["hell|world", "hello |world"]);
}

#[test]
fn undo_restores_a_replaced_selection_and_the_typing_that_replaced_it() {
    let mut ed = editor("Hello ^world|!");
    type_chars(&mut ed, "there");
    assert_eq!(state(&ed), "Hello there|!");
    assert!(ed.undo());
    assert_eq!(state(&ed), "Hello ^world|!");
    assert!(ed.redo());
    assert_eq!(state(&ed), "Hello there|!");
}

#[test]
fn backward_selection_is_restored_with_its_direction() {
    let mut ed = editor("a|bc^d");
    ed.backspace();
    ed.undo();
    assert_eq!(state(&ed), "a|bc^d");
}

#[test]
fn a_new_edit_after_undo_discards_the_redo_history() {
    let mut ed = editor("|");
    type_chars(&mut ed, "a");
    ed.undo();
    assert!(ed.can_redo());
    type_chars(&mut ed, "b");
    assert!(!ed.can_redo());
    assert!(!ed.redo());
    assert_eq!(state(&ed), "b|");
}

#[test]
fn breaking_the_undo_group_explicitly() {
    let mut ed = editor("|");
    type_chars(&mut ed, "ab");
    ed.break_undo_group(); // e.g. the note was saved
    type_chars(&mut ed, "cd");
    assert_eq!(undo_all(&mut ed), ["ab|", "|"]);
}

#[test]
fn paste_cut_and_line_operations_are_separate_steps_that_restore_the_selection() {
    let mut ed = editor("one\ntw|o\nthree");
    type_chars(&mut ed, "!");
    ed.paste("\r\npasted\r\n");
    type_chars(&mut ed, "?");
    ed.move_lines_up();
    ed.duplicate_lines();
    ed.select_all();
    ed.cut();
    assert_eq!(state(&ed), "|");
    assert_eq!(
        undo_all(&mut ed),
        [
            "^one\ntw!\n?o\n?o\npasted\nthree|",
            "one\ntw!\n?|o\npasted\nthree",
            "one\ntw!\npasted\n?|o\nthree",
            "one\ntw!\npasted\n|o\nthree",
            "one\ntw!|o\nthree",
            "one\ntw|o\nthree",
        ]
    );
    while ed.redo() {}
    assert_eq!(state(&ed), "|");
}

#[test]
fn nothing_to_undo_or_redo_in_a_fresh_document() {
    let mut ed = Editor::from_text("text");
    assert!(!ed.can_undo());
    assert!(!ed.undo());
    assert!(!ed.redo());
    assert_eq!(ed.buffer().version(), 0);
}

#[test]
fn undo_and_redo_are_reported_as_changes() {
    let mut ed = editor("ab|");
    type_chars(&mut ed, "cd");
    let seen = ed.buffer().version();
    ed.undo();
    ed.redo();
    let changes: Vec<_> = ed
        .buffer()
        .changes_since(seen)
        .unwrap()
        .map(|c| (c.start.0, c.old_end.0, c.new_end.0))
        .collect();
    assert_eq!(changes, [(2, 4, 2), (2, 2, 4)]);
}

#[test]
fn ime_composition_updates_and_commit_undo_as_one_step() {
    let mut ed = editor("a|b");
    ed.replace_and_mark(None, "n", None);
    assert_eq!(ed.marked_range(), Some(ByteOffset(1)..ByteOffset(2)));
    ed.replace_and_mark(None, "に", None);
    ed.replace_and_mark(None, "にほ", None);
    // The IME converts and highlights the first clause.
    ed.replace_and_mark(None, "日本", Some(0..3));
    assert_eq!(state(&ed), "a^日|本b");
    assert_eq!(ed.marked_range(), Some(ByteOffset(1)..ByteOffset(7)));

    ed.insert_text("日本");
    assert_eq!(ed.marked_range(), None);
    assert_eq!(state(&ed), "a日本|b");

    type_chars(&mut ed, "!");
    assert_eq!(undo_all(&mut ed), ["a日本|b", "a|b"]);
    assert!(ed.redo());
    assert_eq!(state(&ed), "a日本|b");
}

#[test]
fn ime_composition_replacing_a_selection_restores_it_on_undo() {
    let mut ed = editor("^old|");
    ed.replace_and_mark(None, "ä", None);
    ed.insert_text("ä");
    assert_eq!(state(&ed), "ä|");
    ed.undo();
    assert_eq!(state(&ed), "^old|");
}

#[test]
fn composed_text_is_marked_by_its_normalized_length() {
    let mut ed = editor("|");
    ed.replace_and_mark(None, "a\r\nb", None);
    assert_eq!(state(&ed), "a\nb|");
    assert_eq!(ed.marked_range(), Some(ByteOffset(0)..ByteOffset(3)));
}

#[test]
fn cancelling_or_abandoning_a_composition() {
    let mut ed = editor("x|");
    ed.replace_and_mark(None, "ka", None);
    ed.replace_and_mark(None, "", None);
    assert_eq!(state(&ed), "x|");
    assert_eq!(ed.marked_range(), None);

    // Clicking elsewhere keeps the composed text but ends the composition.
    ed.replace_and_mark(None, "ka", None);
    ed.move_to(ByteOffset(0), false);
    assert_eq!(ed.marked_range(), None);
    assert_eq!(state(&ed), "|xka");

    // The platform may also end it explicitly.
    ed.replace_and_mark(None, "y", None);
    ed.unmark();
    assert_eq!(ed.marked_range(), None);
    type_chars(&mut ed, "z");
    assert_eq!(state(&ed), "yz|xka");
    ed.undo();
    assert_eq!(
        state(&ed),
        "y|xka",
        "typing after unmark is a separate undo step"
    );
}

#[test]
fn a_cancelled_composition_is_not_an_undo_step() {
    let mut ed = editor("|");
    type_chars(&mut ed, "ab");
    ed.replace_and_mark(None, "ka", None);
    ed.replace_and_mark(None, "", None);
    assert_eq!(undo_all(&mut ed), ["|"]);

    // Nor does it glue the compositions before and after it into one step.
    let mut ed = editor("|");
    ed.replace_and_mark(None, "日本", None);
    ed.insert_text("日本");
    ed.replace_and_mark(None, "ka", None);
    ed.replace_and_mark(None, "", None);
    ed.replace_and_mark(None, "x", None);
    ed.insert_text("x");
    assert_eq!(undo_all(&mut ed), ["日本|", "|"]);
}

#[test]
fn an_ime_selection_past_the_composed_text_is_clamped_to_it() {
    let mut ed = editor("a|b");
    ed.replace_and_mark(None, "に", Some(0..10));
    assert_eq!(state(&ed), "a^に|b");
}

#[test]
fn a_transaction_of_several_edits_undoes_in_one_step() {
    // Making the selected word bold: two insertions, then reselecting the word between the markers.
    let mut ed = editor("make ^bold| text");
    ed.transact(|ed| {
        let word = ed.selection().range();
        ed.replace_range(word.end..word.end, "**");
        ed.replace_range(word.start..word.start, "**");
        ed.set_selection(Selection::new(
            ByteOffset(word.start.0 + 2),
            ByteOffset(word.end.0 + 2),
        ));
    });
    assert_eq!(state(&ed), "make **^bold|** text");
    type_chars(&mut ed, "!");
    assert_eq!(
        undo_all(&mut ed),
        ["make **^bold|** text", "make ^bold| text"]
    );
    ed.redo();
    assert_eq!(state(&ed), "make **^bold|** text");
}

#[test]
fn a_transaction_without_edits_leaves_nothing_to_undo() {
    let mut ed = editor("ab|");
    type_chars(&mut ed, "c");
    ed.transact(|ed| ed.move_cursor(Motion::Left, false));
    type_chars(&mut ed, "x");
    assert_eq!(undo_all(&mut ed), ["ab|c", "ab|"]);
}

#[test]
fn nested_transactions_join_the_outer_one() {
    let mut ed = editor("|");
    ed.transact(|ed| {
        ed.insert_text("a");
        ed.transact(|ed| ed.insert_newline());
        ed.insert_text("b");
    });
    assert_eq!(state(&ed), "a\nb|");
    assert_eq!(undo_all(&mut ed), ["|"]);
}

#[test]
fn the_oldest_steps_are_forgotten_beyond_the_history_budget() {
    let over_half = UNDO_HISTORY_BUDGET_BYTES / 2 + 1;
    let mut ed = editor("|");
    type_chars(&mut ed, "note");
    ed.paste(&"a".repeat(over_half));
    ed.paste(&"b".repeat(over_half));

    // The two pastes are over the budget together, so only the newest can be undone; the text
    // stays.
    assert!(ed.undo());
    assert_eq!(ed.buffer().len(), "note".len() + over_half);
    assert!(!ed.undo());
    assert!(ed.redo());
    assert_eq!(ed.buffer().len(), "note".len() + 2 * over_half);
}

#[test]
fn a_step_larger_than_the_budget_can_still_be_undone() {
    let mut ed = editor("keep|");
    ed.paste(&"x".repeat(UNDO_HISTORY_BUDGET_BYTES + 1));
    assert!(ed.undo());
    assert_eq!(state(&ed), "keep|");
}
