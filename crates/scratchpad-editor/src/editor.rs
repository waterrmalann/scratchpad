use std::ops::{Range, RangeInclusive};

use crate::buffer::{Buffer, normalize_line_endings};
use crate::coords::{Bias, ByteOffset, to_usize_range};
use crate::history::{Edit, EditKind, History};
use crate::motion::{self, Motion};
use crate::search::{self, CaseSensitivity};
use crate::selection::{Goal, Selection};

/// [`Editor::replace_all`] replaces matches fewer bytes apart than this as one edit.
const MERGE_GAP_BYTES: usize = 1024;

/// Editor state for one document: buffer, selection, goal column, undo history and IME composition. This is
/// the source of truth the UI renders from; every mutation goes through its methods.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    buffer: Buffer,
    selection: Selection,
    goal: Goal,
    history: History,
    /// Text of an in-progress IME composition.
    marked: Option<Range<ByteOffset>>,
}

impl Editor {
    /// Opens file text (see [`Buffer::from_text`]) with the cursor at the start.
    pub fn from_text(text: &str) -> Self {
        Self {
            buffer: Buffer::from_text(text),
            ..Self::default()
        }
    }

    /// Serializes the document for saving (see [`Buffer::to_text`]).
    pub fn to_text(&self) -> String {
        self.buffer.to_text()
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn goal(&self) -> Goal {
        self.goal
    }

    /// Sets the selection, snapping both ends to grapheme boundaries, and clears the goal.
    pub fn set_selection(&mut self, selection: Selection) {
        self.select(selection, Goal::None);
    }

    /// Moves the head to `offset` (a click), keeping the anchor when `extend` is set (a shift-click or
    /// drag).
    pub fn move_to(&mut self, offset: ByteOffset, extend: bool) {
        self.move_to_with_goal(offset, extend, Goal::None);
    }

    /// Like [`Editor::move_to`] but sets the goal afterwards. The view uses this for vertical movement
    /// over wrapped lines, storing its horizontal position as [`Goal::Horizontal`].
    pub fn move_to_with_goal(&mut self, offset: ByteOffset, extend: bool, goal: Goal) {
        let anchor = if extend {
            self.selection.anchor
        } else {
            offset
        };
        self.select(Selection::new(anchor, offset), goal);
    }

    /// Applies a keyboard motion; with `extend` (Shift) the anchor stays put.
    pub fn move_cursor(&mut self, motion: Motion, extend: bool) {
        let (selection, goal) =
            motion::apply(&self.buffer, self.selection, self.goal, motion, extend);
        self.select(selection, goal);
    }

    pub fn select_all(&mut self) {
        self.set_selection(Selection::new(ByteOffset(0), self.buffer.end()));
    }

    /// Selects the word, punctuation run or whitespace run at `offset` (double-click).
    pub fn select_word_at(&mut self, offset: ByteOffset) {
        let range = self.word_range_at(offset);
        self.set_selection(Selection::new(range.start, range.end));
    }

    /// Selects the line at `offset` including its line break (triple-click).
    pub fn select_line_at(&mut self, offset: ByteOffset) {
        let range = self.line_range_at(offset);
        self.set_selection(Selection::new(range.start, range.end));
    }

    /// The range [`Editor::select_word_at`] selects; dragging after a double-click extends by these.
    pub fn word_range_at(&self, offset: ByteOffset) -> Range<ByteOffset> {
        let offset = self.buffer.clip_offset(offset, Bias::Left);
        motion::word_range_at(&self.buffer, offset)
    }

    /// The range [`Editor::select_line_at`] selects; dragging after a triple-click extends by these.
    pub fn line_range_at(&self, offset: ByteOffset) -> Range<ByteOffset> {
        motion::line_range_at(&self.buffer, offset)
    }

    /// Every selection change that is not an edit goes through here. Moving the cursor ends the current
    /// undo group and any IME composition.
    fn select(&mut self, selection: Selection, goal: Goal) {
        self.selection = Selection::new(
            self.buffer.clip_offset(selection.anchor, Bias::Left),
            self.buffer.clip_offset(selection.head, Bias::Left),
        );
        self.goal = goal;
        self.marked = None;
        self.history.break_group();
    }

    /// Types `text`, replacing the selection — or, during IME composition, commits `text` in place of the
    /// composed text.
    pub fn insert_text(&mut self, text: &str) {
        if let Some(marked) = self.marked.clone() {
            self.edit(marked, text, EditKind::Composition, None);
            self.history.break_group();
            return;
        }
        let kind = if text.contains(['\n', '\r']) {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        self.edit(self.selection.range(), text, kind, None);
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    /// Deletes the selection, or the grapheme cluster before the cursor.
    pub fn backspace(&mut self) {
        self.delete_selection_or(EditKind::DeleteBackward, |buffer, head| {
            buffer.prev_grapheme_boundary(head)
        });
    }

    /// Deletes the selection, or the grapheme cluster after the cursor.
    pub fn delete_forward(&mut self) {
        self.delete_selection_or(EditKind::DeleteForward, |buffer, head| {
            buffer.next_grapheme_boundary(head)
        });
    }

    /// Ctrl+Backspace: deletes the selection, or back to where Ctrl+Left would move.
    pub fn delete_word_backward(&mut self) {
        self.delete_selection_or(EditKind::Other, motion::word_left);
    }

    /// Ctrl+Delete: deletes the selection, or up to where Ctrl+Right would move.
    pub fn delete_word_forward(&mut self) {
        self.delete_selection_or(EditKind::Other, motion::word_right);
    }

    fn delete_selection_or(
        &mut self,
        kind: EditKind,
        target: impl FnOnce(&Buffer, ByteOffset) -> ByteOffset,
    ) {
        let range = if self.selection.is_empty() {
            let head = self.selection.head;
            let other = target(&self.buffer, head);
            head.min(other)..head.max(other)
        } else {
            self.selection.range()
        };
        self.edit(range, "", kind, None);
    }

    /// Inserts a copy of the selected lines below them and moves the selection onto the copy.
    pub fn duplicate_lines(&mut self) {
        let lines = self.selected_lines();
        let end = self.buffer.line_end(*lines.end());
        let block = self
            .buffer
            .text_for_range(self.buffer.line_start(*lines.start())..end);
        let inserted = format!("\n{block}");
        let after = shifted(self.selection, inserted.len() as isize);
        self.edit(end..end, &inserted, EditKind::Other, Some(after));
    }

    /// Swaps the selected lines with the line above them.
    pub fn move_lines_up(&mut self) {
        let lines = self.selected_lines();
        let Some(above) = lines.start().checked_sub(1) else {
            return;
        };
        let start = self.buffer.line_start(above);
        let end = self.buffer.line_end(*lines.end());
        let above_text = self.buffer.line_text(above);
        let block = self
            .buffer
            .text_for_range(self.buffer.line_start(*lines.start())..end);
        let replacement = format!("{block}\n{above_text}");
        let after = shifted(self.selection, -(above_text.len() as isize + 1));
        self.edit(start..end, &replacement, EditKind::Other, Some(after));
    }

    /// Swaps the selected lines with the line below them.
    pub fn move_lines_down(&mut self) {
        let lines = self.selected_lines();
        let below = lines.end() + 1;
        if below >= self.buffer.line_count() {
            return;
        }
        let start = self.buffer.line_start(*lines.start());
        let end = self.buffer.line_end(below);
        let below_text = self.buffer.line_text(below);
        let block = self
            .buffer
            .text_for_range(start..self.buffer.line_end(*lines.end()));
        let replacement = format!("{below_text}\n{block}");
        let after = shifted(self.selection, below_text.len() as isize + 1);
        self.edit(start..end, &replacement, EditKind::Other, Some(after));
    }

    /// The lines touched by the selection. A selection ending at the very start of a line does not include
    /// that line, matching what users see highlighted.
    fn selected_lines(&self) -> RangeInclusive<usize> {
        let first = self.buffer.line_of(self.selection.start());
        let mut last = self.buffer.line_of(self.selection.end());
        if last > first && self.buffer.line_start(last) == self.selection.end() {
            last -= 1;
        }
        first..=last
    }

    /// The selected text for the clipboard (with `\n` line breaks), or `None` if nothing is selected.
    pub fn copy(&self) -> Option<String> {
        (!self.selection.is_empty()).then(|| {
            self.buffer
                .text_for_range(self.selection.range())
                .into_owned()
        })
    }

    /// Like [`Editor::copy`], and deletes the selection.
    pub fn cut(&mut self) -> Option<String> {
        let text = self.copy()?;
        self.edit(self.selection.range(), "", EditKind::Other, None);
        Some(text)
    }

    /// Inserts clipboard text, replacing the selection, as its own undo step. Line breaks are normalized.
    pub fn paste(&mut self, text: &str) {
        self.edit(self.selection.range(), text, EditKind::Other, None);
    }

    /// Replaces `range` with `text` as its own undo step (unless inside [`Editor::transact`]) and puts the
    /// cursor after the inserted text. Line breaks in `text` are normalized. The building block for
    /// programmatic edits such as Markdown formatting.
    pub fn replace_range(&mut self, range: Range<ByteOffset>, text: &str) {
        self.edit(range, text, EditKind::Other, None);
    }

    /// Replaces every match of `query` (as [`search::find_all`] finds them in the text as it is now) with
    /// `replacement`, taken literally, as one undo step. Returns how many were replaced.
    ///
    /// The cursor ends up where its text went: after the replacement if it was inside a match, else shifted
    /// by the replacements before it. Undo brings back the selection from before.
    pub fn replace_all(&mut self, query: &str, case: CaseSensitivity, replacement: &str) -> usize {
        let text = self.buffer.normalized_text();
        let matches = search::find_all(&text, query, case);
        if matches.is_empty() {
            return 0;
        }
        let replacement = normalize_line_endings(replacement);
        let cursor = offset_after_replacing(self.selection.head, &matches, replacement.len());
        // An edit costs about 10 µs, so 600,000 matches in a 10 MB note would take seconds one by one.
        // Matches close together are replaced as one edit of the text from the first to the last, which
        // bounds the edits by the length of the text and keeps the copied text between them short.
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        for found in &matches {
            let (start, end) = (found.start.0, found.end.0);
            match edits.last_mut() {
                Some((range, new)) if start - range.end < MERGE_GAP_BYTES => {
                    new.push_str(&text[range.end..start]);
                    new.push_str(&replacement);
                    range.end = end;
                }
                _ => edits.push((start..end, replacement.clone().into_owned())),
            }
        }
        self.transact(|editor| {
            // Back to front, so the ranges still to replace stay where the search found them.
            for (range, new) in edits.iter().rev() {
                let range = ByteOffset(range.start)..ByteOffset(range.end);
                editor.edit(range, new, EditKind::Other, None);
            }
            editor.set_selection(Selection::cursor(cursor));
        });
        matches.len()
    }

    /// The range of the in-progress IME composition, if any.
    pub fn marked_range(&self) -> Option<Range<ByteOffset>> {
        self.marked.clone()
    }

    /// Updates an IME composition: replaces `range` (default: the composed text, else the selection) with
    /// `text` and marks it as composed. `selected` is the selection within `text`, in bytes relative to its
    /// start; by default the cursor goes after it. All updates of one composition, and the final
    /// [`Editor::insert_text`] that commits it, undo as a single step.
    pub fn replace_and_mark(
        &mut self,
        range: Option<Range<ByteOffset>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let text = normalize_line_endings(text);
        let range = range
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.range());
        let start = self
            .buffer
            .clip_range_to_chars(to_usize_range(&range))
            .start;
        let selection_after = selected.map(|selected| {
            let in_text = |i: usize| ByteOffset(start + i.min(text.len()));
            Selection::new(in_text(selected.start), in_text(selected.end))
        });
        self.edit(range, &text, EditKind::Composition, selection_after);
        self.marked = (!text.is_empty()).then(|| ByteOffset(start)..ByteOffset(start + text.len()));
    }

    /// Ends the IME composition, keeping its text.
    pub fn unmark(&mut self) {
        self.marked = None;
        self.history.break_group();
    }

    /// Reverts the last undo step and restores the selection from before it. Returns false if there was
    /// nothing to undo.
    pub fn undo(&mut self) -> bool {
        let selection = self.history.undo(&mut self.buffer);
        self.restore(selection)
    }

    /// Re-applies the last undone step and restores the selection from after it. Returns false if there was
    /// nothing to redo.
    pub fn redo(&mut self) -> bool {
        let selection = self.history.redo(&mut self.buffer);
        self.restore(selection)
    }

    fn restore(&mut self, selection: Option<Selection>) -> bool {
        let Some(selection) = selection else {
            return false;
        };
        self.selection = selection;
        self.goal = Goal::None;
        self.marked = None;
        true
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Runs `f` as one undo step: undo reverts every edit `f` makes and restores the selection from before
    /// it ran; redo restores the selection `f` left. For commands that edit several places at once, such as
    /// Markdown formatting toggles. Nested calls join the outermost step. `f` must not undo or redo.
    pub fn transact<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if self.history.in_transaction() {
            return f(self);
        }
        self.history.begin_transaction(self.selection);
        let result = f(self);
        self.history.end_transaction(self.selection);
        result
    }

    /// Makes the next edit start a new undo step even if it would otherwise merge with the previous one.
    /// Call it on events such as focus changes or saves.
    pub fn break_undo_group(&mut self) {
        self.history.break_group();
    }

    /// The single path through which the buffer changes. `selection_after` defaults to a cursor after the
    /// inserted text; either way it is snapped to grapheme boundaries in the new text.
    fn edit(
        &mut self,
        range: Range<ByteOffset>,
        text: &str,
        kind: EditKind,
        selection_after: Option<Selection>,
    ) {
        let text = normalize_line_endings(text).into_owned();
        let range = self.buffer.clip_range_to_chars(to_usize_range(&range));
        self.marked = None;
        if range.is_empty() && text.is_empty() {
            return;
        }
        let deleted = self.buffer.replace(range.clone(), &text);
        let after = selection_after
            .unwrap_or_else(|| Selection::cursor(ByteOffset(range.start + text.len())));
        let before = self.selection;
        self.selection = Selection::new(
            self.buffer.clip_offset(after.anchor, Bias::Right),
            self.buffer.clip_offset(after.head, Bias::Right),
        );
        self.goal = Goal::None;
        let edit = Edit {
            start: range.start,
            deleted,
            inserted: text,
        };
        self.history.record(edit, kind, before, self.selection);
    }
}

/// Where `offset` is once each of the sorted `matches` is replaced by `replacement_len` bytes. An offset inside
/// a match goes after its replacement.
fn offset_after_replacing(
    offset: ByteOffset,
    matches: &[Range<ByteOffset>],
    replacement_len: usize,
) -> ByteOffset {
    let before = matches.partition_point(|m| m.end <= offset);
    let removed: usize = matches[..before].iter().map(|m| m.end.0 - m.start.0).sum();
    match matches.get(before) {
        Some(m) if m.start < offset => {
            ByteOffset(m.start.0 - removed + (before + 1) * replacement_len)
        }
        _ => ByteOffset(offset.0 - removed + before * replacement_len),
    }
}

fn shifted(selection: Selection, delta: isize) -> Selection {
    let shift = |offset: ByteOffset| ByteOffset(offset.0.saturating_add_signed(delta));
    Selection::new(shift(selection.anchor), shift(selection.head))
}
