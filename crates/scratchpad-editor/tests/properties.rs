//! Randomized editing sessions over hostile Unicode.

use proptest::prelude::*;
use scratchpad_editor::{ByteOffset, Editor, Motion, Selection};
use unicode_segmentation::UnicodeSegmentation;

/// Pieces that interact badly when concatenated: lone combining marks and joiners, regional indicators that
/// pair up, Hangul jamo that compose, RTL text, and every line break flavour.
const ATOMS: &[&str] = &[
    "a",
    "Z",
    "é",
    "e\u{301}",
    "\u{301}",
    "\u{200d}",
    "👨‍👩‍👧",
    "👍",
    "🏽",
    "🇩🇪",
    "\u{1f1eb}",
    "日本",
    "שלום",
    "مَ",
    "\u{1100}",
    "\u{1161}",
    "\u{11a8}",
    "\r\n",
    "\r",
    "\n",
    " ",
    "\t",
    ".",
    "_",
    "(",
];

fn hostile_text() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::collection::vec(prop::sample::select(ATOMS), 0..30).prop_map(|atoms| atoms.concat()),
        any::<String>(),
    ]
}

fn motion() -> impl Strategy<Value = Motion> {
    prop_oneof![
        Just(Motion::Left),
        Just(Motion::Right),
        Just(Motion::WordLeft),
        Just(Motion::WordRight),
        Just(Motion::LineStart),
        Just(Motion::LineEnd),
        Just(Motion::DocumentStart),
        Just(Motion::DocumentEnd),
        Just(Motion::Up),
        Just(Motion::Down),
        (0..5usize).prop_map(Motion::PageUp),
        (0..5usize).prop_map(Motion::PageDown),
    ]
}

#[derive(Debug, Clone)]
enum Op {
    Type(String),
    Newline,
    Backspace,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    DuplicateLines,
    MoveLinesUp,
    MoveLinesDown,
    Cut,
    Paste(String),
    Replace(usize, usize, String),
    Compose(String, Option<(usize, usize)>),
    CommitComposition(String),
    Unmark,
    BreakUndoGroup,
    Move(Motion, bool),
    MoveTo(usize, bool),
    SelectWordAt(usize),
    SelectLineAt(usize),
    SelectAll,
    /// Edits grouped with `Editor::transact`.
    Transaction(Vec<Op>),
}

/// Edits and selection changes, sometimes grouped into a transaction; no undo/redo.
fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        20 => single_op(),
        1 => prop::collection::vec(single_op(), 0..6).prop_map(Op::Transaction),
    ]
}

fn single_op() -> impl Strategy<Value = Op> {
    let offset = 0..120usize;
    prop_oneof![
        4 => prop::sample::select(ATOMS).prop_map(|s| Op::Type(s.to_string())),
        1 => Just(Op::Newline),
        2 => Just(Op::Backspace),
        2 => Just(Op::DeleteForward),
        1 => Just(Op::DeleteWordBackward),
        1 => Just(Op::DeleteWordForward),
        1 => Just(Op::DuplicateLines),
        1 => Just(Op::MoveLinesUp),
        1 => Just(Op::MoveLinesDown),
        1 => Just(Op::Cut),
        1 => hostile_text().prop_map(Op::Paste),
        1 => (offset.clone(), offset.clone(), hostile_text()).prop_map(|(a, b, t)| Op::Replace(a, b, t)),
        1 => (hostile_text(), prop::option::of((0..8usize, 0..8usize)))
            .prop_map(|(t, sel)| Op::Compose(t, sel)),
        1 => hostile_text().prop_map(Op::CommitComposition),
        1 => Just(Op::Unmark),
        1 => Just(Op::BreakUndoGroup),
        4 => (motion(), any::<bool>()).prop_map(|(m, extend)| Op::Move(m, extend)),
        1 => (offset.clone(), any::<bool>()).prop_map(|(o, extend)| Op::MoveTo(o, extend)),
        1 => offset.clone().prop_map(Op::SelectWordAt),
        1 => offset.prop_map(Op::SelectLineAt),
        1 => Just(Op::SelectAll),
    ]
}

fn apply(editor: &mut Editor, op: &Op) {
    match op {
        Op::Type(text) => editor.insert_text(text),
        Op::Newline => editor.insert_newline(),
        Op::Backspace => editor.backspace(),
        Op::DeleteForward => editor.delete_forward(),
        Op::DeleteWordBackward => editor.delete_word_backward(),
        Op::DeleteWordForward => editor.delete_word_forward(),
        Op::DuplicateLines => editor.duplicate_lines(),
        Op::MoveLinesUp => editor.move_lines_up(),
        Op::MoveLinesDown => editor.move_lines_down(),
        Op::Cut => {
            editor.cut();
        }
        Op::Paste(text) => editor.paste(text),
        Op::Replace(a, b, text) => editor.replace_range(ByteOffset(*a)..ByteOffset(*b), text),
        Op::Compose(text, selected) => {
            editor.replace_and_mark(None, text, selected.map(|(a, b)| a.min(b)..a.max(b)))
        }
        Op::CommitComposition(text) => editor.insert_text(text),
        Op::Unmark => editor.unmark(),
        Op::BreakUndoGroup => editor.break_undo_group(),
        Op::Move(motion, extend) => editor.move_cursor(*motion, *extend),
        Op::MoveTo(offset, extend) => editor.move_to(ByteOffset(*offset), *extend),
        Op::SelectWordAt(offset) => editor.select_word_at(ByteOffset(*offset)),
        Op::SelectLineAt(offset) => editor.select_line_at(ByteOffset(*offset)),
        Op::SelectAll => editor.select_all(),
        Op::Transaction(ops) => editor.transact(|editor| {
            for op in ops {
                apply(editor, op);
            }
        }),
    }
}

/// Checks the invariants every editor state must satisfy, using `str` segmentation as the oracle.
fn check_invariants(editor: &Editor) -> Result<(), TestCaseError> {
    let text = editor.buffer().normalized_text();
    prop_assert!(
        !text.contains('\r'),
        "the buffer never contains CR: {:?}",
        text
    );
    let boundaries: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let selection = editor.selection();
    for offset in [selection.anchor, selection.head] {
        prop_assert!(
            boundaries.contains(&offset.0),
            "{:?} is not a grapheme boundary in {:?}",
            offset,
            text
        );
    }
    if let Some(marked) = editor.marked_range() {
        prop_assert!(marked.start <= marked.end && marked.end.0 <= text.len());
        prop_assert!(text.is_char_boundary(marked.start.0) && text.is_char_boundary(marked.end.0));
    }
    Ok(())
}

/// How many steps undo can take.
fn undo_depth(editor: &Editor) -> usize {
    let mut editor = editor.clone();
    let mut depth = 0;
    while editor.undo() {
        depth += 1;
    }
    depth
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Undoing everything restores the original text and the selection from before the first edit; redoing
    /// everything restores the final text and the selection right after the last edit.
    #[test]
    fn undo_all_restores_the_original_and_redo_all_the_final_state(
        text in hostile_text(),
        start in 0..60usize,
        ops in prop::collection::vec(op(), 0..40),
    ) {
        let mut editor = Editor::from_text(&text);
        editor.move_to(ByteOffset(start), false);
        let original = editor.buffer().normalized_text();

        // The selection before and after each undo step. Edits that cancel out (typing and backspacing in one
        // transaction, a composition committed as nothing) leave no step, so the steps are tracked by watching
        // the undo depth: an op either adds a step, extends the last one (never a transaction), removes it, or
        // leaves no trace.
        let mut steps: Vec<(Selection, Selection)> = Vec::new();
        for op in &ops {
            let selection = editor.selection();
            let version = editor.buffer().version();
            apply(&mut editor, op);
            let depth = undo_depth(&editor);
            if depth > steps.len() {
                steps.push((selection, editor.selection()));
            } else if depth < steps.len() {
                steps.pop();
            } else if editor.buffer().version() != version
                && !matches!(op, Op::Transaction(_))
                && let Some(last) = steps.last_mut()
            {
                last.1 = editor.selection();
            }
        }
        let edited = editor.buffer().normalized_text();

        while editor.undo() {}
        prop_assert_eq!(editor.buffer().normalized_text(), original);
        if let Some((before_first, _)) = steps.first() {
            prop_assert_eq!(editor.selection(), *before_first);
        }

        while editor.redo() {}
        prop_assert_eq!(editor.buffer().normalized_text(), edited);
        if let Some((_, after_last)) = steps.last() {
            prop_assert_eq!(editor.selection(), *after_last);
        }
    }

    /// The selection stays on grapheme boundaries through any mix of edits, motions, undo and redo.
    #[test]
    fn selection_always_lies_on_grapheme_boundaries(
        text in hostile_text(),
        steps in prop::collection::vec(
            prop_oneof![8 => op().prop_map(Some), 1 => Just(None)],
            0..60,
        ),
        undo_or_redo in prop::collection::vec(any::<bool>(), 60),
    ) {
        let mut editor = Editor::from_text(&text);
        check_invariants(&editor)?;
        // `None` steps undo or redo, so history replay is checked too.
        for (step, undo) in steps.iter().zip(&undo_or_redo) {
            match step {
                Some(op) => apply(&mut editor, op),
                None if *undo => {
                    editor.undo();
                }
                None => {
                    editor.redo();
                }
            }
            check_invariants(&editor)?;
        }
    }
}
