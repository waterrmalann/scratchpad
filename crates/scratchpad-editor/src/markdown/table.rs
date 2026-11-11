//! GFM tables (ADR 0186): their decorations, built from pulldown-cmark's table events, and the query a renderer
//! uses to lay a table out as a grid.
//!
//! pulldown-cmark reports the table, its header and body rows and their cells, with cell ranges between the
//! pipes. It says nothing about the delimiter row (`|:--|--:|`), which is always the line after the header, or
//! about the pipes, which are the bytes around the cells; both are derived here. A row's markers are its pipes:
//! the leading one, those between cells and the trailing one, which also takes any cells past the header's
//! count, since GFM drops those. The delimiter row is one marker. Every table marker reveals while the selection
//! touches the table's lines: the grid's column widths come from every row, so the table is shown as typed or as
//! a grid as a whole.

// The delimiter row's marker list holds a single range, which is what this lint flags.
#![allow(clippy::single_range_in_vec_init)]

use std::ops::Range;

use pulldown_cmark::Alignment;

use super::{Decoration, DecorationKind, MarkdownState};
use crate::buffer::Buffer;
use crate::coords::ByteOffset;
use crate::selection::Selection;

/// How a table column's text is aligned, from the colons of its delimiter row cell (`:--`, `:-:`, `--:`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TableAlignment {
    /// No colons: left aligned, as GitHub shows it.
    #[default]
    None,
    Left,
    Center,
    Right,
}

/// Which row of a table a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableRowKind {
    Header,
    /// The `|---|:-:|` line under the header.
    Delimiter,
    Body,
}

/// A table as a renderer needs it to lay out one of its lines ([`MarkdownState::table`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// The table, from the start of its header row to the end of its last row.
    pub range: Range<ByteOffset>,
    /// The lines it covers.
    pub lines: Range<usize>,
    /// One per column, from the delimiter row; their number is the table's column count.
    pub alignments: Vec<TableAlignment>,
    /// Whether the selection touches the table's lines, so that its markers show: a renderer then shows the
    /// table as typed rather than as a grid. Always set without a selection (source mode).
    pub revealed: bool,
}

impl MarkdownState {
    /// The table `line` belongs to, if any, parsing only the region it is in. With a `selection` (live preview),
    /// [`Table::revealed`] says whether it touches the table. Nothing is cloned but the table's alignments.
    pub fn table(
        &mut self,
        buffer: &Buffer,
        line: usize,
        selection: Option<Selection>,
    ) -> Option<Table> {
        self.sync(buffer);
        if line >= buffer.line_count() {
            return None;
        }
        let at = buffer.line_start(line).0;
        let end = buffer.line_end(line).0;
        let index = self.region_index(at);
        let region_start = self.starts[index];
        let (range, alignments) = self
            .region_decorations(buffer, index)
            .iter()
            .find_map(|d| match &d.kind {
                DecorationKind::Table { alignments }
                    if d.range.start.0 + region_start <= end
                        && at <= d.range.end.0 + region_start =>
                {
                    let range = ByteOffset(d.range.start.0 + region_start)
                        ..ByteOffset(d.range.end.0 + region_start);
                    Some((range, alignments.clone()))
                }
                _ => None,
            })?;
        let lines = buffer.line_of(range.start)..buffer.line_of(range.end) + 1;
        let revealed = selection.is_none_or(|selection| {
            let start = buffer.line_start(lines.start);
            let end = buffer.line_end(lines.end - 1);
            selection.start() <= end && start <= selection.end()
        });
        Some(Table {
            range,
            lines,
            alignments,
            revealed,
        })
    }
}

/// The table being built while its events come in; the parser passes the region's text and decorations to each
/// event.
#[derive(Debug, Default)]
pub(super) struct TableState {
    /// The index of the open row's decoration.
    row: Option<usize>,
    /// The column alignments of the open table.
    alignments: Vec<TableAlignment>,
    /// The ranges of the open row's cells so far.
    cells: Vec<Range<usize>>,
}

/// What a table `Start` event opens.
pub(super) enum Start<'a> {
    Table(&'a [Alignment]),
    Row { header: bool },
    Cell,
}

impl TableState {
    /// Adds the decoration that a `Start` event of a table, row or cell at `range` of `text` opens.
    pub fn start(
        &mut self,
        text: &str,
        decorations: &mut Vec<Decoration>,
        range: Range<usize>,
        event: Start,
    ) {
        match event {
            Start::Table(alignments) => {
                let range = trim(text, range);
                self.alignments = alignments.iter().map(|&a| alignment(a)).collect();
                let kind = DecorationKind::Table {
                    alignments: self.alignments.clone(),
                };
                push(decorations, kind, range.clone(), range, Vec::new());
            }
            Start::Row { header } => {
                let range = trim(text, range);
                let kind = if header {
                    DecorationKind::TableHeader
                } else {
                    DecorationKind::TableRow
                };
                // Content and markers are set when the row ends, once its cells are known.
                self.row = Some(push(decorations, kind, range.clone(), range, Vec::new()));
                self.cells.clear();
            }
            Start::Cell => {
                let Some(row) = self.row else {
                    return;
                };
                // A row with fewer cells than the header is padded with empty cells at its end, which are no
                // text.
                if range.is_empty() && range.start >= decorations[row].range.end.0 {
                    return;
                }
                let column = self.cells.len();
                let alignment = self.alignments.get(column).copied().unwrap_or_default();
                let start = skip(text, range.start, range.end, b" \t");
                let end = skip_back(text, range.end, start, b" \t");
                let kind = DecorationKind::TableCell { column, alignment };
                push(decorations, kind, range.clone(), start..end, Vec::new());
                self.cells.push(range);
            }
        }
    }

    /// Sets the content and markers (its pipes) of the row that ends, and after the header adds the delimiter
    /// row.
    pub fn end_row(&mut self, text: &str, decorations: &mut Vec<Decoration>) {
        let Some(index) = self.row.take() else {
            return;
        };
        let row = &mut decorations[index];
        let range = row.range.start.0..row.range.end.0;
        let byte = |i: usize| text.as_bytes().get(i).copied();
        let mut markers = Vec::new();
        let mut content = range.clone();
        if let (Some(first), Some(last)) = (self.cells.first(), self.cells.last()) {
            if first.start > range.start && byte(first.start - 1) == Some(b'|') {
                markers.push(first.start - 1..first.start);
                content.start = first.start;
            }
            for cell in &self.cells[..self.cells.len() - 1] {
                if byte(cell.end) == Some(b'|') {
                    markers.push(cell.end..cell.end + 1);
                }
            }
            // The trailing pipe, with any cells past the header's count (GFM drops them) and trailing spaces.
            if last.end < range.end && byte(last.end) == Some(b'|') {
                markers.push(last.end..range.end);
                content.end = last.end;
            }
        }
        row.content = offsets(content);
        row.markers = markers.into_iter().map(offsets).collect();
        if row.kind == DecorationKind::TableHeader {
            delimiter_row(text, decorations, range.end);
        }
    }
}

/// The delimiter row, on the line after the header row that ends at `header_end`: after any container prefix
/// (indentation and the `>` of quotes), to its last non-blank byte. Delimiter rows consist of `|`, `-`, `:` and
/// whitespace, so the prefix is the `>` and whitespace it starts with.
fn delimiter_row(text: &str, decorations: &mut Vec<Decoration>, header_end: usize) {
    if header_end >= text.len() {
        return;
    }
    let line_start = header_end + 1;
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    let start = skip(text, line_start, line_end, b" \t>");
    let end = skip_back(text, line_end, start, b" \t");
    if start < end {
        let kind = DecorationKind::TableDelimiter;
        push(decorations, kind, start..end, end..end, vec![start..end]);
    }
}

fn alignment(alignment: Alignment) -> TableAlignment {
    match alignment {
        Alignment::None => TableAlignment::None,
        Alignment::Left => TableAlignment::Left,
        Alignment::Center => TableAlignment::Center,
        Alignment::Right => TableAlignment::Right,
    }
}

/// The table among the decorations on a line: its lines are where the selection reveals its markers.
pub(super) fn enclosing<'a>(on_line: &[&'a Decoration]) -> Option<&'a Decoration> {
    on_line
        .iter()
        .find(|d| matches!(d.kind, DecorationKind::Table { .. }))
        .copied()
}

fn push(
    decorations: &mut Vec<Decoration>,
    kind: DecorationKind,
    range: Range<usize>,
    content: Range<usize>,
    markers: Vec<Range<usize>>,
) -> usize {
    decorations.push(Decoration {
        kind,
        range: offsets(range),
        content: offsets(content),
        markers: markers.into_iter().map(offsets).collect(),
    });
    decorations.len() - 1
}

fn offsets(range: Range<usize>) -> Range<ByteOffset> {
    ByteOffset(range.start)..ByteOffset(range.end)
}

/// `range` without the line breaks and blank lines pulldown-cmark ends some block ranges with.
fn trim(text: &str, range: Range<usize>) -> Range<usize> {
    let last_text = skip_back(text, range.end, range.start, b" \t\n");
    let line_end = text[last_text..range.end]
        .find('\n')
        .map_or(range.end, |i| last_text + i);
    range.start..line_end
}

/// The first position in `from..to` whose byte is not in `set`, or `to`.
fn skip(text: &str, from: usize, to: usize, set: &[u8]) -> usize {
    let bytes = &text.as_bytes()[from.min(to)..to];
    from + bytes.iter().take_while(|b| set.contains(b)).count()
}

/// Moves back from `from` over bytes in `set`, stopping at `limit`.
fn skip_back(text: &str, from: usize, limit: usize, set: &[u8]) -> usize {
    let bytes = &text.as_bytes()[limit.min(from)..from];
    from - bytes.iter().rev().take_while(|b| set.contains(b)).count()
}
