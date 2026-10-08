//! Undo/redo as a log of edit operations grouped into transactions (PLAN §16, ADR 0005).

use crate::buffer::Buffer;
use crate::selection::Selection;

/// The most text (deleted plus inserted) the undo history keeps. Beyond it the oldest steps are forgotten, but
/// never the newest one, however large: a few pastes of megabytes must not stay in memory for as long as the
/// note is open (ADR 0112).
pub const UNDO_HISTORY_BUDGET_BYTES: usize = 32 * 1024 * 1024;

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

#[derive(Debug, Clone, PartialEq)]
struct Transaction {
    edits: Vec<Edit>,
    kind: EditKind,
    selection_before: Selection,
    selection_after: Selection,
}

impl Transaction {
    /// The bytes of text it holds.
    fn size(&self) -> usize {
        self.edits
            .iter()
            .map(|edit| edit.deleted.len() + edit.inserted.len())
            .sum()
    }

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

#[derive(Debug, Clone)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Whether the next edit may join the last transaction.
    group_open: bool,
    /// Whether an explicit transaction is open, which takes every edit whatever its kind.
    in_transaction: bool,
    /// The size of every undo step but the last, which edits can still grow. Kept up to date by
    /// [`Self::push_undo`] and [`Self::pop_undo`].
    settled_bytes: usize,
    /// [`UNDO_HISTORY_BUDGET_BYTES`]; smaller in tests.
    budget_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            group_open: false,
            in_transaction: false,
            settled_bytes: 0,
            budget_bytes: UNDO_HISTORY_BUDGET_BYTES,
        }
    }
}

impl History {
    pub fn record(&mut self, edit: Edit, kind: EditKind, before: Selection, after: Selection) {
        self.redo.clear();
        if self.in_transaction
            && let Some(last) = self.undo.last_mut()
        {
            // Checked against the budget when the transaction ends, not once per edit in it.
            last.push(edit);
            return;
        }
        self.record_step(edit, kind, before, after);
        self.forget_oldest();
    }

    fn record_step(&mut self, edit: Edit, kind: EditKind, before: Selection, after: Selection) {
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
                    self.pop_undo();
                    self.group_open = false;
                    return;
                }
            }
            _ => self.push_undo(Transaction {
                edits: vec![edit],
                kind,
                selection_before: before,
                selection_after: after,
            }),
        }
        self.group_open = mergeable;
    }

    fn push_undo(&mut self, transaction: Transaction) {
        if let Some(last) = self.undo.last() {
            self.settled_bytes += last.size();
        }
        self.undo.push(transaction);
    }

    fn pop_undo(&mut self) -> Option<Transaction> {
        let transaction = self.undo.pop()?;
        if let Some(last) = self.undo.last() {
            self.settled_bytes -= last.size();
        }
        Some(transaction)
    }

    /// Drops the oldest undo steps while the history is over its budget, keeping the newest.
    fn forget_oldest(&mut self) {
        let newest = self.undo.last().map_or(0, Transaction::size);
        let mut forget = 0;
        while self.settled_bytes + newest > self.budget_bytes && forget + 1 < self.undo.len() {
            self.settled_bytes -= self.undo[forget].size();
            forget += 1;
        }
        self.undo.drain(..forget);
    }

    pub fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    /// Opens an explicit transaction: every edit until [`History::end_transaction`] joins one undo step.
    pub fn begin_transaction(&mut self, selection_before: Selection) {
        self.push_undo(Transaction {
            edits: Vec::new(),
            kind: EditKind::Other,
            selection_before,
            selection_after: selection_before,
        });
        self.in_transaction = true;
    }

    pub fn end_transaction(&mut self, selection_after: Selection) {
        self.in_transaction = false;
        self.group_open = false;
        if let Some(last) = self.pop_undo()
            && !last.edits.is_empty()
        {
            self.push_undo(Transaction {
                selection_after,
                ..last
            });
            self.forget_oldest();
        }
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
        let transaction = self.pop_undo()?;
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
        self.push_undo(transaction);
        self.group_open = false;
        Some(selection)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

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

    /// Undo restores the selection from before the first edit of a step, so an edit made from some other
    /// selection (it moved without the group being broken) must not be folded into the step.
    #[test]
    fn typing_from_another_selection_starts_a_new_step() {
        let at = |offset| Selection::cursor(ByteOffset(offset));
        let mut history = History::default();
        history.record(edit(0, "", "a"), EditKind::Typing, at(0), at(1));
        history.record(edit(1, "", "b"), EditKind::Typing, at(1), at(2));
        assert_eq!(
            history.undo.len(),
            1,
            "continuing from the cursor extends the step"
        );

        history.record(edit(2, "", "c"), EditKind::Typing, at(0), at(3));
        assert_eq!(history.undo.len(), 2);
    }

    #[test]
    fn edits_that_do_not_touch_stay_separate() {
        assert_folds(
            "abcdef",
            &[edit(0, "a", ""), edit(3, "", "X"), edit(1, "c", "")],
            3,
        );
    }

    #[derive(Debug, Clone)]
    enum Step {
        /// Replaces `len` bytes at `at` (both scaled into the text) with `text`.
        Edit(usize, usize, String, EditKind),
        Transaction(Vec<(usize, usize, String)>),
        Undo,
        Redo,
    }

    fn step() -> impl Strategy<Value = Step> {
        let kind = prop::sample::select(vec![
            EditKind::Typing,
            EditKind::DeleteBackward,
            EditKind::DeleteForward,
            EditKind::Composition,
            EditKind::Other,
        ]);
        let edit = (any::<usize>(), 0..8usize, "[ab\n]{0,8}");
        prop_oneof![
            4 => (edit.clone(), kind).prop_map(|((at, len, text), kind)| Step::Edit(at, len, text, kind)),
            1 => prop::collection::vec(edit, 0..4).prop_map(Step::Transaction),
            2 => Just(Step::Undo),
            1 => Just(Step::Redo),
        ]
    }

    /// Applies an edit to `buffer` and records it in both histories, with distinct selections so that
    /// restoring the wrong one shows.
    fn record(
        buffers: &mut [Buffer; 2],
        histories: &mut [History; 2],
        (at, len, text): &(usize, usize, String),
        kind: EditKind,
    ) {
        let end = buffers[0].len();
        let start = at % (end + 1);
        let range = start..(start + len).min(end);
        let edit = Edit {
            start,
            deleted: buffers[0]
                .text_for_range(ByteOffset(range.start)..ByteOffset(range.end))
                .into_owned(),
            inserted: text.clone(),
        };
        let before = Selection::cursor(ByteOffset(range.end));
        let after = Selection::cursor(ByteOffset(start + text.len()));
        for (buffer, history) in buffers.iter_mut().zip(histories.iter_mut()) {
            buffer.replace(range.clone(), text);
            history.record(edit.clone(), kind, before, after);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// A history with a small budget behaves exactly like an unbounded one, except that it holds only the
        /// newest steps: always the one just recorded, and within the budget unless that one alone is larger.
        #[test]
        fn a_history_over_budget_keeps_the_newest_steps_of_an_unbounded_one(
            budget in 0..40usize,
            steps in prop::collection::vec(step(), 0..40),
        ) {
            let mut buffers = [Buffer::from_text("ab\nba"), Buffer::from_text("ab\nba")];
            // The default budget is far above anything these steps record.
            let mut histories = [History { budget_bytes: budget, ..History::default() }, History::default()];
            for step in &steps {
                let (steps_before, newest_before) = (histories[1].undo.len(), histories[1].undo.last().cloned());
                match step {
                    Step::Edit(at, len, text, kind) => {
                        record(&mut buffers, &mut histories, &(*at, *len, text.clone()), *kind);
                    }
                    Step::Transaction(edits) => {
                        let cursor = Selection::cursor(ByteOffset(edits.len()));
                        histories.iter_mut().for_each(|history| history.begin_transaction(cursor));
                        for edit in edits {
                            record(&mut buffers, &mut histories, edit, EditKind::Other);
                        }
                        histories.iter_mut().for_each(|history| history.end_transaction(cursor));
                    }
                    // Only as far as the capped history can go, so both stay on the same text.
                    Step::Undo => {
                        if let Some(selection) = histories[0].undo(&mut buffers[0]) {
                            prop_assert_eq!(histories[1].undo(&mut buffers[1]), Some(selection));
                        }
                    }
                    Step::Redo => {
                        let selection = histories[0].redo(&mut buffers[0]);
                        prop_assert_eq!(histories[1].redo(&mut buffers[1]), selection);
                    }
                }
                prop_assert_eq!(buffers[0].normalized_text(), buffers[1].normalized_text());

                let [capped, unbounded] = &histories;
                prop_assert!(unbounded.undo.ends_with(&capped.undo));
                prop_assert_eq!(&capped.redo, &unbounded.redo);
                let added_or_grown =
                    unbounded.undo.len() >= steps_before && unbounded.undo.last() != newest_before.as_ref();
                if added_or_grown {
                    prop_assert_eq!(capped.undo.last(), unbounded.undo.last());
                }
                let sizes: Vec<usize> = capped.undo.iter().map(Transaction::size).collect();
                prop_assert!(sizes.iter().sum::<usize>() <= budget || sizes.len() == 1, "{sizes:?}");
                prop_assert_eq!(capped.settled_bytes, sizes.iter().rev().skip(1).sum::<usize>());
            }

            // What is left undoes and redoes cleanly.
            let edited = buffers[0].normalized_text();
            let mut undone = 0;
            while histories[0].undo(&mut buffers[0]).is_some() {
                undone += 1;
            }
            for _ in 0..undone {
                histories[0].redo(&mut buffers[0]);
            }
            prop_assert_eq!(buffers[0].normalized_text(), edited);
        }
    }
}
