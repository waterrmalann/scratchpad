//! The text buffer: a rope holding LF-normalized text, plus the file's line ending and a change log.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::ops::Range;

use ropey::{Rope, RopeSlice};

use crate::coords::{Bias, ByteOffset, Point, Utf16Offset, to_usize_range};
use crate::graphemes::Graphemes;

/// How line breaks are written when the buffer is serialized (ADR 0003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

/// One replacement applied to the buffer, in the coordinates of the text just before it was applied.
///
/// `start..old_end` was replaced by `start..new_end`. Points are included so consumers keyed by line (layout
/// caches, incremental parsers) need not reconstruct old line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextChange {
    pub start: ByteOffset,
    pub old_end: ByteOffset,
    pub new_end: ByteOffset,
    pub start_point: Point,
    pub old_end_point: Point,
    pub new_end_point: Point,
}

/// How many recent changes [`Buffer::changes_since`] can report. Consumers further behind than this must
/// invalidate everything, which they have to support anyway (e.g. after loading a different note).
const CHANGE_LOG_CAPACITY: usize = 1024;

/// Document text. Internally every line break is `\n`; the file's original line ending is restored by
/// [`Buffer::to_text`].
///
/// The buffer is read-only from outside the crate: all edits go through [`Editor`](crate::Editor) so that history
/// and selection stay consistent.
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    rope: Rope,
    line_ending: LineEnding,
    version: u64,
    changes: VecDeque<TextChange>,
}

impl Buffer {
    /// Loads file text: detects the dominant line ending and normalizes all line breaks (`\r\n`, lone `\r`) to
    /// `\n`.
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(&normalize_line_endings(text)),
            line_ending: detect_line_ending(text),
            version: 0,
            changes: VecDeque::new(),
        }
    }

    /// Serializes the buffer for writing to disk, using [`Buffer::line_ending`] for every line break.
    pub fn to_text(&self) -> String {
        match self.line_ending {
            LineEnding::Lf => self.rope.to_string(),
            LineEnding::Crlf => {
                let mut out = String::with_capacity(self.rope.len_bytes() + self.rope.len_lines());
                for chunk in self.rope.chunks() {
                    for (i, piece) in chunk.split('\n').enumerate() {
                        if i > 0 {
                            out.push_str("\r\n");
                        }
                        out.push_str(piece);
                    }
                }
                out
            }
        }
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    /// The buffer contents with `\n` line breaks.
    pub fn normalized_text(&self) -> String {
        self.rope.to_string()
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn end(&self) -> ByteOffset {
        ByteOffset(self.len())
    }

    /// Number of lines. Always at least 1; text ending in `\n` has an empty last line.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Offset of the first byte of `line`, clamped to the last line.
    pub fn line_start(&self, line: usize) -> ByteOffset {
        ByteOffset(self.rope.line_to_byte(line.min(self.last_line())))
    }

    /// Offset just before the line break of `line` (or the end of the buffer for the last line), clamped to the
    /// last line.
    pub fn line_end(&self, line: usize) -> ByteOffset {
        let line = line.min(self.last_line());
        if line == self.last_line() {
            self.end()
        } else {
            ByteOffset(self.rope.line_to_byte(line + 1) - 1)
        }
    }

    /// The text of `line` without its line break.
    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        self.slice(self.line_start(line)..self.line_end(line))
            .into()
    }

    /// The line containing `offset` (clamped to the buffer).
    pub fn line_of(&self, offset: ByteOffset) -> usize {
        self.rope.byte_to_line(offset.0.min(self.len()))
    }

    pub fn offset_to_point(&self, offset: ByteOffset) -> Point {
        let offset = self.clip_to_char(offset.0);
        let line = self.rope.byte_to_line(offset);
        Point::new(line, offset - self.rope.line_to_byte(line))
    }

    /// Converts a point to an offset, clamping the line to the buffer and the column to the line's length, and
    /// rounding down to a char boundary.
    pub fn point_to_offset(&self, point: Point) -> ByteOffset {
        let start = self.line_start(point.line).0;
        let end = self.line_end(point.line).0;
        ByteOffset(self.clip_to_char((start + point.column).min(end)))
    }

    pub fn offset_to_utf16(&self, offset: ByteOffset) -> Utf16Offset {
        let char_idx = self.rope.byte_to_char(offset.0.min(self.len()));
        Utf16Offset(self.rope.char_to_utf16_cu(char_idx))
    }

    /// Converts a UTF-16 offset to a byte offset. Offsets inside a surrogate pair round down.
    pub fn utf16_to_offset(&self, offset: Utf16Offset) -> ByteOffset {
        let offset = offset.0.min(self.rope.len_utf16_cu());
        ByteOffset(self.rope.char_to_byte(self.rope.utf16_cu_to_char(offset)))
    }

    /// Like [`Buffer::text_for_range`], without copying.
    pub(crate) fn slice(&self, range: Range<ByteOffset>) -> RopeSlice<'_> {
        let range = self.clip_range_to_chars(to_usize_range(&range));
        self.rope.byte_slice(range)
    }

    /// The text in `range`, which is clamped to the buffer and rounded to char boundaries.
    pub fn text_for_range(&self, range: Range<ByteOffset>) -> Cow<'_, str> {
        self.slice(range).into()
    }

    /// Moves `offset` onto a grapheme cluster boundary (clamping it to the buffer first).
    pub fn clip_offset(&self, offset: ByteOffset, bias: Bias) -> ByteOffset {
        let mut graphemes = self.graphemes(offset);
        if graphemes.is_boundary() {
            return ByteOffset(graphemes.offset());
        }
        ByteOffset(match bias {
            Bias::Left => graphemes.prev().unwrap_or(0),
            Bias::Right => graphemes.next().unwrap_or(self.len()),
        })
    }

    /// The grapheme boundary before `offset`, or the start of the buffer.
    pub fn prev_grapheme_boundary(&self, offset: ByteOffset) -> ByteOffset {
        ByteOffset(self.graphemes(offset).prev().unwrap_or(0))
    }

    /// The grapheme boundary after `offset`, or the end of the buffer.
    pub fn next_grapheme_boundary(&self, offset: ByteOffset) -> ByteOffset {
        ByteOffset(self.graphemes(offset).next().unwrap_or(self.len()))
    }

    /// The lines from `line` (clamped) to the end, with their start offsets and without their line breaks; text
    /// ending in `\n` yields an empty last line. Lines are borrowed from the rope unless they span chunks, which
    /// makes this several times faster than ropey's line iterator for scanning a whole document.
    pub(crate) fn lines_from(&self, line: usize) -> Lines<'_> {
        let start = self.line_start(line).0;
        let (mut chunks, chunk_start, _, _) = self.rope.chunks_at_byte(start);
        let rest = chunks
            .next()
            .map_or("", |chunk| &chunk[start - chunk_start..]);
        Lines {
            chunks,
            rest,
            offset: start,
            done: false,
        }
    }

    /// A grapheme walker starting at `offset`, which is clamped to the buffer and rounded down to a char.
    pub(crate) fn graphemes(&self, offset: ByteOffset) -> Graphemes<'_> {
        Graphemes::at(&self.rope, self.clip_to_char(offset.0))
    }

    /// Incremented by every change to the text.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The changes made after `version`, oldest first, or `None` if they are no longer (or not yet) known and the
    /// caller must treat the whole buffer as changed.
    pub fn changes_since(&self, version: u64) -> Option<impl Iterator<Item = &TextChange>> {
        let missed = usize::try_from(self.version.checked_sub(version)?).ok()?;
        let first = self.changes.len().checked_sub(missed)?;
        Some(self.changes.range(first..))
    }

    /// Replaces `range` with `text` and returns the removed text. `text` must already be LF-normalized.
    pub(crate) fn replace(&mut self, range: Range<usize>, text: &str) -> String {
        debug_assert!(!text.contains('\r'), "inserted text must be normalized");
        let range = self.clip_range_to_chars(range);
        let start_point = self.offset_to_point(ByteOffset(range.start));
        let old_end_point = self.offset_to_point(ByteOffset(range.end));

        let removed = self.rope.byte_slice(range.clone()).to_string();
        let start_char = self.rope.byte_to_char(range.start);
        let end_char = self.rope.byte_to_char(range.end);
        self.rope.remove(start_char..end_char);
        self.rope.insert(start_char, text);

        let new_end = range.start + text.len();
        self.record_change(TextChange {
            start: ByteOffset(range.start),
            old_end: ByteOffset(range.end),
            new_end: ByteOffset(new_end),
            start_point,
            old_end_point,
            new_end_point: self.offset_to_point(ByteOffset(new_end)),
        });
        removed
    }

    fn record_change(&mut self, change: TextChange) {
        self.version += 1;
        if self.changes.len() == CHANGE_LOG_CAPACITY {
            self.changes.pop_front();
        }
        self.changes.push_back(change);
    }

    fn last_line(&self) -> usize {
        self.rope.len_lines() - 1
    }

    /// Clamps to the buffer and rounds down to a char boundary.
    fn clip_to_char(&self, offset: usize) -> usize {
        let offset = offset.min(self.len());
        self.rope.char_to_byte(self.rope.byte_to_char(offset))
    }

    /// Clamps to the buffer and rounds both ends down to char boundaries; an inverted range becomes empty.
    pub(crate) fn clip_range_to_chars(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clip_to_char(range.start);
        start..self.clip_to_char(range.end).max(start)
    }
}

/// See [`Buffer::lines_from`].
pub(crate) struct Lines<'a> {
    chunks: ropey::iter::Chunks<'a>,
    /// The part of the current chunk not yet returned.
    rest: &'a str,
    /// The buffer offset of `rest`.
    offset: usize,
    done: bool,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, Cow<'a, str>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let start = self.offset;
        // The beginning of a line that spans chunks.
        let mut spanning = String::new();
        loop {
            let (line, more) = match self.rest.find('\n') {
                Some(i) => (&self.rest[..i], &self.rest[i + 1..]),
                None => (self.rest, ""),
            };
            let found = line.len() < self.rest.len();
            self.offset += self.rest.len() - more.len();
            self.rest = more;
            if !found && let Some(chunk) = self.chunks.next() {
                spanning.push_str(line);
                self.rest = chunk;
                continue;
            }
            self.done = !found;
            let line = if spanning.is_empty() {
                Cow::Borrowed(line)
            } else {
                spanning.push_str(line);
                Cow::Owned(spanning)
            };
            return Some((start, line));
        }
    }
}

/// Converts `\r\n` and lone `\r` to `\n`, borrowing when there is nothing to convert.
pub(crate) fn normalize_line_endings(text: &str) -> Cow<'_, str> {
    if !text.contains('\r') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(cr) = rest.find('\r') {
        out.push_str(&rest[..cr]);
        out.push('\n');
        rest = &rest[cr + 1..];
        rest = rest.strip_prefix('\n').unwrap_or(rest);
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// CRLF wins only if it is strictly the most common line break; ties and break-less text default to LF.
fn detect_line_ending(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    let mut crlf = 0usize;
    let mut other = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => crlf += 1,
            b'\r' => other += 1,
            b'\n' if i > 0 && bytes[i - 1] == b'\r' => {}
            b'\n' => other += 1,
            _ => {}
        }
    }
    if crlf > other {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_records_change_with_points_in_pre_edit_coordinates() {
        let mut buffer = Buffer::from_text("ab\ncd\nef");
        let removed = buffer.replace(1..4, "X\nY\nZ");
        assert_eq!(removed, "b\nc");
        assert_eq!(buffer.normalized_text(), "aX\nY\nZd\nef");

        let changes: Vec<_> = buffer.changes_since(0).unwrap().copied().collect();
        assert_eq!(
            changes,
            [TextChange {
                start: ByteOffset(1),
                old_end: ByteOffset(4),
                new_end: ByteOffset(6),
                start_point: Point::new(0, 1),
                old_end_point: Point::new(1, 1),
                new_end_point: Point::new(2, 1),
            }]
        );
    }

    #[test]
    fn changes_since_reports_only_newer_changes_and_forgets_old_ones() {
        let mut buffer = Buffer::default();
        for i in 0..CHANGE_LOG_CAPACITY + 10 {
            buffer.replace(i..i, "x");
        }
        let current = buffer.version();
        assert_eq!(current, (CHANGE_LOG_CAPACITY + 10) as u64);

        let last_two: Vec<_> = buffer
            .changes_since(current - 2)
            .unwrap()
            .map(|c| c.start)
            .collect();
        assert_eq!(
            last_two,
            [
                ByteOffset(CHANGE_LOG_CAPACITY + 8),
                ByteOffset(CHANGE_LOG_CAPACITY + 9)
            ]
        );
        assert_eq!(buffer.changes_since(current).unwrap().count(), 0);
        assert_eq!(
            buffer.changes_since(10).unwrap().count(),
            CHANGE_LOG_CAPACITY
        );
        assert!(
            buffer.changes_since(9).is_none(),
            "evicted changes must not be reported as complete"
        );
        assert!(buffer.changes_since(current + 1).is_none());
    }

    #[test]
    fn lines_from_yields_every_line_across_chunk_boundaries() {
        // Lines of many lengths, some much longer than a rope chunk, and a trailing empty line.
        let text: String = (0..300)
            .map(|i| format!("{}é\n", "x".repeat(i * i % 3000)))
            .collect();
        let buffer = Buffer::from_text(&text);
        let expected: Vec<(usize, &str)> = text
            .split('\n')
            .scan(0, |start, line| {
                let item = (*start, line);
                *start += line.len() + 1;
                Some(item)
            })
            .collect();
        for first in [0, 1, 150, 300] {
            let lines: Vec<(usize, String)> = buffer
                .lines_from(first)
                .map(|(start, line)| (start, line.into_owned()))
                .collect();
            let expected: Vec<(usize, String)> = expected[first..]
                .iter()
                .map(|&(start, line)| (start, line.to_string()))
                .collect();
            assert_eq!(lines, expected, "from line {first}");
        }
        assert_eq!(
            Buffer::default().lines_from(0).collect::<Vec<_>>(),
            [(0, Cow::Borrowed(""))]
        );
    }

    #[test]
    fn line_ending_detection_prefers_lf_on_ties() {
        assert_eq!(detect_line_ending("a\r\nb\nc"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a\r\nb\r\nc\n"), LineEnding::Crlf);
        assert_eq!(detect_line_ending("a\r\r\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("no breaks"), LineEnding::Lf);
        assert_eq!(detect_line_ending("\r\n"), LineEnding::Crlf);
    }
}
