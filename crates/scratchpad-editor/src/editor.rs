use std::ops::Range;

use crate::buffer::{Buffer, normalize_line_endings};
use crate::coords::{Bias, ByteOffset, to_usize_range};
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
