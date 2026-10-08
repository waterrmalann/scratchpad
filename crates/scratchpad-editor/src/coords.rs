//! Explicit coordinate types (PLAN §13).
//!
//! The canonical position in a document is a [`ByteOffset`]. Every other coordinate system converts to and from
//! it through [`Buffer`](crate::Buffer), so byte, UTF-16 and line/column positions can never be mixed up.

use std::ops::Range;

/// A UTF-8 byte offset into the buffer's (LF-normalized) text.
///
/// Offsets handed out by the engine always lie on a `char` boundary, and cursor positions additionally lie on a
/// grapheme cluster boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(pub usize);

/// A line/column position. Both are zero-based.
///
/// `column` is measured in **UTF-8 bytes from the start of the line** (not chars, graphemes or visual columns),
/// which is what per-line text shaping APIs index by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub line: usize,
    pub column: usize,
}

/// An offset in UTF-16 code units, as used by platform text input (IME) APIs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utf16Offset(pub usize);

/// Which way to move an offset that falls inside a character or grapheme cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Left,
    Right,
}

impl Point {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

pub(crate) fn to_usize_range(range: &Range<ByteOffset>) -> Range<usize> {
    range.start.0..range.end.0
}
