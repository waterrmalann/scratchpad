//! Markdown-aware Enter: continuing lists and quotes while typing.

mod common;

use common::{editor, state, type_chars};
use scratchpad_editor::markdown::MarkdownState;

/// Presses Enter on marked text and returns the resulting marked text.
fn enter(marked: &str) -> String {
    let mut ed = editor(marked);
    let mut markdown = MarkdownState::new(ed.buffer());
    ed.insert_markdown_newline(&mut markdown);
    state(&ed)
}

#[test]
fn typing_a_list_and_pressing_enter_repeatedly() {
    let mut ed = editor("|");
    let mut markdown = MarkdownState::new(ed.buffer());
    type_chars(&mut ed, "- one");
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "- one\n- |");
    type_chars(&mut ed, "two");
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "- one\n- two\n- |");
    // Enter on the empty item ends the list, leaving a blank line before the next paragraph.
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "- one\n- two\n\n|");
    type_chars(&mut ed, "after");
    assert_eq!(state(&ed), "- one\n- two\n\nafter|");

    // Each Enter is one undo step.
    assert!(ed.undo());
    assert_eq!(state(&ed), "- one\n- two\n\n|");
    assert!(ed.undo());
    assert_eq!(state(&ed), "- one\n- two\n- |");
    assert!(ed.undo());
    assert_eq!(state(&ed), "- one\n- two|");
}

#[test]
fn ordered_items_count_up_and_keep_their_delimiter() {
    assert_eq!(enter("1. first|"), "1. first\n2. |");
    assert_eq!(enter("9) nine|"), "9) nine\n10) |");
    assert_eq!(enter("1.  wide|"), "1.  wide\n2.  |");
}

#[test]
fn task_items_continue_unchecked() {
    assert_eq!(enter("- [x] done|"), "- [x] done\n- [ ] |");
    assert_eq!(enter("* [ ] todo|"), "* [ ] todo\n* [ ] |");
    assert_eq!(enter("- [ ] |"), "\n|");
}

#[test]
fn quotes_and_nested_items_keep_their_prefix() {
    assert_eq!(enter("> quoted|"), "> quoted\n> |");
    assert_eq!(enter("> > deep|"), "> > deep\n> > |");
    assert_eq!(enter("> - item|"), "> - item\n> - |");
    assert_eq!(enter("- a\n  + sub|"), "- a\n  + sub\n  + |");
    assert_eq!(enter("> |"), "\n|");
}

#[test]
fn enter_in_the_middle_of_an_item_moves_the_rest_to_a_new_item() {
    assert_eq!(enter("- fo|o"), "- fo\n- |o");
}

#[test]
fn enter_before_the_marker_or_on_plain_text_is_a_plain_line_break() {
    assert_eq!(enter("|- item"), "\n|- item");
    assert_eq!(enter("-| item"), "-\n| item");
    assert_eq!(enter("hello|"), "hello\n|");
    assert_eq!(enter("  indented|"), "  indented\n|");
    assert_eq!(enter("* * *|"), "* * *\n|");
}

#[test]
fn no_continuation_inside_code_blocks() {
    assert_eq!(enter("```\n- item|\n```"), "```\n- item\n|\n```");
    assert_eq!(enter("- ```\n  > q|\n  ```"), "- ```\n  > q\n  |\n  ```");
}

#[test]
fn a_selection_is_replaced_first_in_the_same_undo_step() {
    let mut ed = editor("- a^bc|d");
    let mut markdown = MarkdownState::new(ed.buffer());
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "- a\n- |d");
    assert!(ed.undo());
    assert_eq!(state(&ed), "- a^bc|d");
}

#[test]
fn the_markdown_state_follows_edits_made_between_enters() {
    let mut ed = editor("|");
    let mut markdown = MarkdownState::new(ed.buffer());
    type_chars(&mut ed, "```\n- x");
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "```\n- x\n|");
    ed.select_all();
    type_chars(&mut ed, "- x");
    ed.insert_markdown_newline(&mut markdown);
    assert_eq!(state(&ed), "- x\n- |");
}

#[test]
fn a_new_ordered_item_renumbers_the_items_after_it() {
    assert_eq!(
        enter("1. a|\n2. b\n   more b\n   - nested\n3. c\n\nafter\n4. x"),
        "1. a\n2. |\n3. b\n   more b\n   - nested\n4. c\n\nafter\n4. x"
    );
    assert_eq!(enter("> 9. a|\n> 10. b"), "> 9. a\n> 10. |\n> 11. b");
    // Lists numbered 1. throughout, other delimiters and other lists are left alone.
    assert_eq!(enter("1. a|\n1. b"), "1. a\n2. |\n1. b");
    assert_eq!(enter("1. a|\n2) b"), "1. a\n2. |\n2) b");
}

#[test]
fn enter_on_an_empty_nested_item_moves_it_out_to_the_parent_list() {
    assert_eq!(enter("- a\n  - b\n  - |"), "- a\n  - b\n- |");
    assert_eq!(
        enter("1. a\n   - b\n   - |\n2. c"),
        "1. a\n   - b\n2. |\n3. c"
    );
    assert_eq!(enter("> - [x] a\n>   - |"), "> - [x] a\n> - [ ] |");
    // Without a parent item it ends the list as at the top level.
    assert_eq!(enter("text\n  - |"), "text\n\n|");
}

#[test]
fn enter_in_a_code_block_keeps_the_indentation_and_enclosing_quotes() {
    assert_eq!(
        enter("```\n    let a;|\n```"),
        "```\n    let a;\n    |\n```"
    );
    assert_eq!(enter("> ```\n> x|\n> ```"), "> ```\n> x\n> |\n> ```");
    assert_eq!(enter("- ```\n  x|\n  ```"), "- ```\n  x\n  |\n  ```");
    // Code that only looks like a quote is not one.
    assert_eq!(enter("```\n>>> x|\n```"), "```\n>>> x\n|\n```");
}
