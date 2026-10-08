use std::ops::Range;

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

    /// Replaces `range` with `text` and puts the cursor after the inserted text. Line breaks in `text` are
    /// normalized.
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
