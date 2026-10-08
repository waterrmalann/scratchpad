use std::ops::{Range, RangeInclusive};

use crate::buffer::{Buffer, normalize_line_endings};
use crate::coords::{Bias, ByteOffset, to_usize_range};
use crate::motion::{self, Motion};
use crate::selection::{Goal, Selection};

/// Editor state for one document: buffer, selection and goal column. This is the source of truth the UI renders
/// from; every mutation goes through its methods.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    buffer: Buffer,
    selection: Selection,
    goal: Goal,
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
        let offset = self.buffer.clip_offset(offset, Bias::Left);
        let range = motion::word_range_at(&self.buffer, offset);
        self.set_selection(Selection::new(range.start, range.end));
    }

    /// Selects the line at `offset` including its line break (triple-click).
    pub fn select_line_at(&mut self, offset: ByteOffset) {
        let range = motion::line_range_at(&self.buffer, offset);
        self.set_selection(Selection::new(range.start, range.end));
    }

    /// Every selection change that is not an edit goes through here.
    fn select(&mut self, selection: Selection, goal: Goal) {
        self.selection = Selection::new(
            self.buffer.clip_offset(selection.anchor, Bias::Left),
            self.buffer.clip_offset(selection.head, Bias::Left),
        );
        self.goal = goal;
    }

    /// Types `text`, replacing the selection.
    pub fn insert_text(&mut self, text: &str) {
        self.edit(self.selection.range(), text, None);
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    /// Deletes the selection, or the grapheme cluster before the cursor.
    pub fn backspace(&mut self) {
        self.delete_selection_or(|buffer, head| buffer.prev_grapheme_boundary(head));
    }

    /// Deletes the selection, or the grapheme cluster after the cursor.
    pub fn delete_forward(&mut self) {
        self.delete_selection_or(|buffer, head| buffer.next_grapheme_boundary(head));
    }

    /// Ctrl+Backspace: deletes the selection, or back to where Ctrl+Left would move.
    pub fn delete_word_backward(&mut self) {
        self.delete_selection_or(motion::word_left);
    }

    /// Ctrl+Delete: deletes the selection, or up to where Ctrl+Right would move.
    pub fn delete_word_forward(&mut self) {
        self.delete_selection_or(motion::word_right);
    }

    fn delete_selection_or(&mut self, target: impl FnOnce(&Buffer, ByteOffset) -> ByteOffset) {
        let range = if self.selection.is_empty() {
            let head = self.selection.head;
            let other = target(&self.buffer, head);
            head.min(other)..head.max(other)
        } else {
            self.selection.range()
        };
        self.edit(range, "", None);
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
        self.edit(end..end, &inserted, Some(after));
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
        self.edit(start..end, &replacement, Some(after));
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
        self.edit(start..end, &replacement, Some(after));
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
        self.edit(self.selection.range(), "", None);
        Some(text)
    }

    /// Inserts clipboard text, replacing the selection. Line breaks are normalized.
    pub fn paste(&mut self, text: &str) {
        self.edit(self.selection.range(), text, None);
    }

    /// Replaces `range` with `text` and puts the cursor after the inserted text. Line breaks in `text` are
    /// normalized. The building block for programmatic edits such as Markdown formatting.
    pub fn replace_range(&mut self, range: Range<ByteOffset>, text: &str) {
        self.edit(range, text, None);
    }

    /// The single path through which the buffer changes. `selection_after` defaults to a cursor after the
    /// inserted text; either way it is snapped to grapheme boundaries in the new text.
    fn edit(&mut self, range: Range<ByteOffset>, text: &str, selection_after: Option<Selection>) {
        let text = normalize_line_endings(text);
        let range = self.buffer.clip_range_to_chars(to_usize_range(&range));
        if range.is_empty() && text.is_empty() {
            return;
        }
        self.buffer.replace(range.clone(), &text);
        let after =
            selection_after.unwrap_or(Selection::cursor(ByteOffset(range.start + text.len())));
        self.selection = Selection::new(
            self.buffer.clip_offset(after.anchor, Bias::Right),
            self.buffer.clip_offset(after.head, Bias::Right),
        );
        self.goal = Goal::None;
    }
}

fn shifted(selection: Selection, delta: isize) -> Selection {
    let shift = |offset: ByteOffset| ByteOffset(offset.0.saturating_add_signed(delta));
    Selection::new(shift(selection.anchor), shift(selection.head))
}
