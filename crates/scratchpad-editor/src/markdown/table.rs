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
use std::sync::Arc;

use pulldown_cmark::Alignment;

use super::blocks;
use super::parse::{offsets, push, skip, skip_back, trim};
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
    /// One per column, from the delimiter row; their number is the table's column count. Shared with the
    /// table's decoration, so a query copies no more than a pointer.
    pub alignments: Arc<[TableAlignment]>,
    /// Whether the selection touches the table's lines, so that its markers show: a renderer then shows the
    /// table as typed rather than as a grid. Always set without a selection (source mode).
    pub revealed: bool,
}

impl MarkdownState {
    /// The table `line` belongs to, if any, parsing only the region it is in. With a `selection` (live preview),
    /// [`Table::revealed`] says whether it touches the table. Only the decorations on the line are looked at, so
    /// the cost does not grow with the table's length, and nothing is cloned.
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
        let parsed = self.region_decorations(buffer, index);
        let mut touching = Vec::new();
        parsed.touching(at - region_start..end - region_start, &mut touching);
        let (range, alignments) = touching.iter().find_map(|&i| {
            let d = &parsed.decorations[i];
            match &d.kind {
                DecorationKind::Table { alignments } => {
                    let range = ByteOffset(d.range.start.0 + region_start)
                        ..ByteOffset(d.range.end.0 + region_start);
                    Some((range, Arc::clone(alignments)))
                }
                _ => None,
            }
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
                    alignments: self.alignments.as_slice().into(),
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

/// The delimiter row, on the line after the header row that ends at `header_end` (pulldown-cmark reports nothing
/// for it), with the range [`blocks::delimiter_row`] gives: after any container prefix, to its last non-blank byte.
fn delimiter_row(text: &str, decorations: &mut Vec<Decoration>, header_end: usize) {
    if header_end >= text.len() {
        return;
    }
    let line_start = header_end + 1;
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    if let Some(row) = blocks::delimiter_row(&text[line_start..line_end]) {
        let (start, end) = (line_start + row.start, line_start + row.end);
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
