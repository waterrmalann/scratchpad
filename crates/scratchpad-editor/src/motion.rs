//! Cursor motions over logical lines, grapheme clusters and words.

use std::ops::Range;

use crate::buffer::Buffer;
use crate::coords::ByteOffset;
use crate::selection::{Goal, Selection};

/// A keyboard cursor motion. Vertical motions move by logical (unwrapped) lines; the view implements movement by
/// wrapped visual lines itself with [`Editor::move_to_with_goal`](crate::Editor::move_to_with_goal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// One grapheme cluster left. Without extend, a selection collapses to its start instead.
    Left,
    /// One grapheme cluster right. Without extend, a selection collapses to its end instead.
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    DocumentStart,
    DocumentEnd,
    Up,
    Down,
    /// Up by the given number of lines.
    PageUp(usize),
    /// Down by the given number of lines.
    PageDown(usize),
}

/// Applies `motion` to `selection`, returning the new selection and goal.
pub(crate) fn apply(
    buffer: &Buffer,
    selection: Selection,
    goal: Goal,
    motion: Motion,
    extend: bool,
) -> (Selection, Goal) {
    let head = selection.head;
    let (new_head, new_goal) = match motion {
        Motion::Left if !extend && !selection.is_empty() => (selection.start(), Goal::None),
        Motion::Right if !extend && !selection.is_empty() => (selection.end(), Goal::None),
        Motion::Left => (buffer.prev_grapheme_boundary(head), Goal::None),
        Motion::Right => (buffer.next_grapheme_boundary(head), Goal::None),
        Motion::WordLeft => (word_left(buffer, head), Goal::None),
        Motion::WordRight => (word_right(buffer, head), Goal::None),
        Motion::LineStart => (buffer.line_start(buffer.line_of(head)), Goal::None),
        Motion::LineEnd => (buffer.line_end(buffer.line_of(head)), Goal::None),
        Motion::DocumentStart => (ByteOffset(0), Goal::None),
        Motion::DocumentEnd => (buffer.end(), Goal::None),
        Motion::Up => vertical(buffer, head, goal, -1),
        Motion::Down => vertical(buffer, head, goal, 1),
        Motion::PageUp(lines) => vertical(buffer, head, goal, -saturating_isize(lines)),
        Motion::PageDown(lines) => vertical(buffer, head, goal, saturating_isize(lines)),
    };
    let anchor = if extend { selection.anchor } else { new_head };
    (Selection::new(anchor, new_head), new_goal)
}

fn saturating_isize(n: usize) -> isize {
    isize::try_from(n).unwrap_or(isize::MAX)
}

/// Moves `line_delta` logical lines, keeping the grapheme column of the goal. Moving past the first or last line
/// goes to the start or end of the document.
fn vertical(
    buffer: &Buffer,
    head: ByteOffset,
    goal: Goal,
    line_delta: isize,
) -> (ByteOffset, Goal) {
    let column = match goal {
        Goal::Column(column) => column,
        Goal::None | Goal::Horizontal(_) => grapheme_column(buffer, head),
    };
    let line = buffer.line_of(head);
    let target = match line.checked_add_signed(line_delta) {
        None => ByteOffset(0),
        Some(target) if target >= buffer.line_count() => buffer.end(),
        Some(target) => offset_at_grapheme_column(buffer, target, column),
    };
    (target, Goal::Column(column))
}

fn grapheme_column(buffer: &Buffer, offset: ByteOffset) -> usize {
    let mut graphemes = buffer.graphemes(buffer.line_start(buffer.line_of(offset)));
    let mut column = 0;
    while graphemes.offset() < offset.0 && graphemes.next().is_some() {
        column += 1;
    }
    column
}

fn offset_at_grapheme_column(buffer: &Buffer, line: usize, column: usize) -> ByteOffset {
    let end = buffer.line_end(line);
    let mut graphemes = buffer.graphemes(buffer.line_start(line));
    for _ in 0..column {
        if graphemes.offset() >= end.0 || graphemes.next().is_none() {
            break;
        }
    }
    ByteOffset(graphemes.offset())
}

/// Word motions treat runs of word characters, runs of punctuation and line breaks as stops, and skip
/// whitespace. A grapheme cluster is classified by its first char, so combining marks stay with their base and
/// emoji count as punctuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    LineBreak,
    Whitespace,
    Word,
    Punctuation,
}

fn classify(c: char) -> CharClass {
    if c == '\n' {
        CharClass::LineBreak
    } else if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// The grapheme clusters after `offset`, nearest first: each one's class and the offset where it ends.
fn classes_after(
    buffer: &Buffer,
    offset: ByteOffset,
) -> impl Iterator<Item = (CharClass, ByteOffset)> {
    let mut graphemes = buffer.graphemes(offset);
    std::iter::from_fn(move || {
        let class = classify(graphemes.char_after()?);
        Some((class, ByteOffset(graphemes.next()?)))
    })
}

/// The grapheme clusters before `offset`, nearest first: each one's class and the offset where it starts.
fn classes_before(
    buffer: &Buffer,
    offset: ByteOffset,
) -> impl Iterator<Item = (CharClass, ByteOffset)> {
    let mut graphemes = buffer.graphemes(offset);
    std::iter::from_fn(move || {
        let start = graphemes.prev()?;
        Some((classify(graphemes.char_after()?), ByteOffset(start)))
    })
}

/// Ctrl+Right: skip whitespace, then the following run of one class. A line break is a stop of its own, so the
/// cursor halts at the end of a line before moving onto the next one.
pub(crate) fn word_right(buffer: &Buffer, offset: ByteOffset) -> ByteOffset {
    word_stop(offset, classes_after(buffer, offset))
}

/// Ctrl+Left: the mirror image of [`word_right`].
pub(crate) fn word_left(buffer: &Buffer, offset: ByteOffset) -> ByteOffset {
    word_stop(offset, classes_before(buffer, offset))
}

fn word_stop(
    offset: ByteOffset,
    graphemes: impl Iterator<Item = (CharClass, ByteOffset)>,
) -> ByteOffset {
    let mut graphemes = graphemes.peekable();
    let mut pos = offset;
    let mut skipped_whitespace = false;
    while let Some((_, next)) = graphemes.next_if(|&(class, _)| class == CharClass::Whitespace) {
        pos = next;
        skipped_whitespace = true;
    }
    match graphemes.next() {
        None => pos,
        Some((CharClass::LineBreak, _)) if skipped_whitespace => pos,
        Some((CharClass::LineBreak, next)) => next,
        Some((class, next)) => run_end(next, class, graphemes),
    }
}

/// How far the run of `class` that `graphemes` continue from `pos` extends.
fn run_end(
    pos: ByteOffset,
    class: CharClass,
    graphemes: impl Iterator<Item = (CharClass, ByteOffset)>,
) -> ByteOffset {
    graphemes
        .take_while(|&(c, _)| c == class)
        .last()
        .map_or(pos, |(_, end)| end)
}

/// The run of same-class graphemes around `offset` (a word, a punctuation run or whitespace), preferring the
/// grapheme after `offset`. Empty if `offset` is surrounded by line breaks or the document edges.
pub(crate) fn word_range_at(buffer: &Buffer, offset: ByteOffset) -> Range<ByteOffset> {
    let not_line_break = |&(class, _): &(CharClass, ByteOffset)| class != CharClass::LineBreak;
    let class = classes_after(buffer, offset)
        .next()
        .filter(not_line_break)
        .or_else(|| classes_before(buffer, offset).next().filter(not_line_break));
    let Some((class, _)) = class else {
        return offset..offset;
    };
    run_end(offset, class, classes_before(buffer, offset))
        ..run_end(offset, class, classes_after(buffer, offset))
}

/// The whole line containing `offset`, including its line break.
pub(crate) fn line_range_at(buffer: &Buffer, offset: ByteOffset) -> Range<ByteOffset> {
    let line = buffer.line_of(offset);
    let end = if line + 1 < buffer.line_count() {
        buffer.line_start(line + 1)
    } else {
        buffer.end()
    };
    buffer.line_start(line)..end
}
