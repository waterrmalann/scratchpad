use std::ops::Range;

use crate::coords::ByteOffset;

/// The single selection of an editor. `anchor` stays put while extending; `head` is where the cursor is drawn.
/// A cursor is an empty selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: ByteOffset,
    pub head: ByteOffset,
}

impl Selection {
    pub fn new(anchor: ByteOffset, head: ByteOffset) -> Self {
        Self { anchor, head }
    }

    pub fn cursor(offset: ByteOffset) -> Self {
        Self::new(offset, offset)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> ByteOffset {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> ByteOffset {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<ByteOffset> {
        self.start()..self.end()
    }
}

/// The horizontal position vertical movement tries to return to, so that moving through a short line does not
/// lose the column. Any non-vertical selection change resets it to `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Goal {
    #[default]
    None,
    /// Grapheme clusters from the start of the line; used by the engine's logical-line movement.
    Column(usize),
    /// A position defined by the view (e.g. pixels from the left of a wrapped line). The engine only stores it.
    Horizontal(f32),
}
