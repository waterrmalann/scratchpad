//! Undo/redo as a log of edit operations grouped into transactions (PLAN §16, ADR 0005).

use crate::buffer::Buffer;
use crate::selection::Selection;

/// `deleted` was replaced by `inserted` at byte `start`, in the coordinates of the text at the time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Edit {
    pub start: usize,
    pub deleted: String,
    pub inserted: String,
}

/// What produced an edit; consecutive edits of the same mergeable kind form one undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditKind {
    Typing,
    DeleteBackward,
    DeleteForward,
    Composition,
    /// Never merged with anything: newlines, paste, line operations, programmatic replacements, ...
    Other,
}

#[derive(Debug, Clone)]
struct Transaction {
    edits: Vec<Edit>,
    kind: EditKind,
    selection_before: Selection,
    selection_after: Selection,
}

impl Transaction {
    /// Appends an edit, folding it into the previous one when the two touch, so that typing a paragraph
    /// stores one string rather than one edit per keystroke.
    fn push(&mut self, edit: Edit) {
        if let Some(last) = self.edits.last_mut() {
            let last_is_deletion = last.inserted.is_empty();
            let is_deletion = edit.inserted.is_empty();
            if edit.deleted.is_empty() && edit.start == last.start + last.inserted.len() {
                // Typing right after the previous insertion.
                last.inserted.push_str(&edit.inserted);
                return;
            }
            if last_is_deletion && is_deletion && edit.start + edit.deleted.len() == last.start {
                // Backspacing further.
                last.deleted.insert_str(0, &edit.deleted);
                last.start = edit.start;
                return;
            }
            if last_is_deletion && is_deletion && edit.start == last.start {
                // Deleting forward further.
                last.deleted.push_str(&edit.deleted);
                return;
            }
            if edit.start == last.start && edit.deleted == last.inserted {
                // Replacing exactly what was just inserted, as IME composition does.
                last.inserted = edit.inserted;
                if last.inserted == last.deleted {
                    // The two cancel out, e.g. a composition that was abandoned.
                    self.edits.pop();
                }
                return;
            }
        }
        self.edits.push(edit);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Whether the next edit may join the last transaction.
    group_open: bool,
}

impl History {
    pub fn record(&mut self, edit: Edit, kind: EditKind, before: Selection, after: Selection) {
        self.redo.clear();
        let mergeable = kind != EditKind::Other;
        match self.undo.last_mut() {
            Some(last)
                if self.group_open
                    && mergeable
                    && last.kind == kind
                    && last.selection_after == before =>
            {
                last.push(edit);
                last.selection_after = after;
                if last.edits.is_empty() {
                    // Nothing left to undo. Closing the group keeps the next edit from joining the step before.
                    self.undo.pop();
                    self.group_open = false;
                    return;
                }
            }
            _ => self.undo.push(Transaction {
                edits: vec![edit],
                kind,
                selection_before: before,
                selection_after: after,
            }),
        }
        self.group_open = mergeable;
    }

    /// Makes the next edit start a new undo step.
    pub fn break_group(&mut self) {
        self.group_open = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the last transaction and returns the selection from before it.
    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<Selection> {
        let transaction = self.undo.pop()?;
        for edit in transaction.edits.iter().rev() {
            buffer.replace(edit.start..edit.start + edit.inserted.len(), &edit.deleted);
        }
        let selection = transaction.selection_before;
        self.redo.push(transaction);
        self.group_open = false;
        Some(selection)
    }

    /// Re-applies the last undone transaction and returns the selection from after it.
    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<Selection> {
        let transaction = self.redo.pop()?;
        for edit in &transaction.edits {
            buffer.replace(edit.start..edit.start + edit.deleted.len(), &edit.inserted);
        }
        let selection = transaction.selection_after;
        self.undo.push(transaction);
        self.group_open = false;
        Some(selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::ByteOffset;

    fn edit(start: usize, deleted: &str, inserted: &str) -> Edit {
        Edit {
            start,
            deleted: deleted.into(),
            inserted: inserted.into(),
        }
    }

    /// Records `edits` as one transaction on `text`, then checks that the folded edits still undo to the
    /// original text and redo to the edited one.
    fn assert_folds(text: &str, edits: &[Edit], expected_edits: usize) {
        let mut buffer = Buffer::from_text(text);
        let mut history = History::default();
        let cursor = Selection::cursor(ByteOffset(0));
        for e in edits {
            buffer.replace(e.start..e.start + e.deleted.len(), &e.inserted);
            history.record(e.clone(), EditKind::Typing, cursor, cursor);
        }
        let edited = buffer.normalized_text();
        assert_eq!(history.undo.len(), 1);
        assert_eq!(history.undo[0].edits.len(), expected_edits);

        history.undo(&mut buffer);
        assert_eq!(buffer.normalized_text(), text);
        history.redo(&mut buffer);
        assert_eq!(buffer.normalized_text(), edited);
    }

    #[test]
    fn typing_folds_into_one_edit() {
        let typed: Vec<_> = "hello"
            .char_indices()
            .map(|(i, c)| edit(i, "", &c.to_string()))
            .collect();
        assert_folds("", &typed, 1);
    }

    #[test]
    fn backspacing_and_forward_deleting_fold_into_one_edit() {
        assert_folds(
            "abcd",
            &[edit(3, "d", ""), edit(2, "c", ""), edit(1, "b", "")],
            1,
        );
        assert_folds(
            "abcd",
            &[edit(1, "b", ""), edit(1, "c", ""), edit(1, "d", "")],
            1,
        );
    }

    #[test]
    fn replacing_the_previous_insertion_folds_into_one_edit() {
        assert_folds(
            "xy",
            &[edit(1, "", "n"), edit(1, "n", "ni"), edit(1, "ni", "に")],
            1,
        );
    }

    #[test]
    fn edits_that_do_not_touch_stay_separate() {
        assert_folds(
            "abcdef",
            &[edit(0, "a", ""), edit(3, "", "X"), edit(1, "c", "")],
            3,
        );
    }
}
