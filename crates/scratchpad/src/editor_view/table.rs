//! GFM tables in live preview, drawn as aligned grids (ADR 0186).
//!
//! While the selection is off a table, its pipes and delimiter row are hidden (the engine's reveal rule) and
//! each row is laid out on one row of the screen with its cells moved to their column: the room between one
//! cell's text and the next is widened, so the buffer is never changed and the usual column mapping
//! ([`StyledLine::display_column`]) keeps clicks right. Column widths need every row of the table, so they are
//! measured once per table and kept in a [`TableCache`] next to the line layouts, dropped only when an edit
//! touches the table or the text style changes. While the selection is on a table it is shown as typed, like
//! any other line; a grid too wide for the text column with word wrap on is shown as typed too, wrapped.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Hsla, Pixels, WindowTextSystem, point, px, size};
use scratchpad_editor::markdown::{
    MarkdownOptions, MarkdownState, MarkerKind, StyledLine, Table, TableAlignment, TableRowKind,
};
use scratchpad_editor::{Buffer, Selection, TextChange};

use super::geometry::{Glyph, set_span_width, unwrapped_x};
use super::line_layout::{BaseStyle, LineKey, measure_cells};
use crate::theme::Theme;

/// Room left and right of a cell's text inside its column, in multiples of the font size.
const CELL_PADDING: f32 = 0.6;
/// Height of the delimiter row drawn as a rule, in multiples of the font size.
pub(super) const DELIMITER_ROW_HEIGHT: f32 = 0.5;
/// Tables whose column widths are kept; a screen rarely shows more than a few.
const KEEP_TABLES: usize = 16;
/// Lines styled at once while measuring a table, so that each query copies only the decorations of those
/// lines.
const MEASURE_CHUNK: usize = 64;

/// The columns of a table laid out as a grid, in pixels from the table's left edge.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct Columns {
    /// The left edge of each column, then the table's right edge.
    edges: Vec<Pixels>,
    /// The widest cell text of each column.
    widths: Vec<Pixels>,
    alignments: Vec<TableAlignment>,
    padding: Pixels,
    /// How far right of the text column's left edge the widest row's grid reaches.
    reach: Pixels,
}

impl Columns {
    fn new(widths: Vec<Pixels>, alignments: Vec<TableAlignment>, font_size: Pixels) -> Self {
        let padding = (font_size * CELL_PADDING).round();
        let mut edges = Vec::with_capacity(widths.len() + 1);
        let mut x = px(0.);
        edges.push(x);
        for width in &widths {
            x += (*width + padding * 2.).ceil();
            edges.push(x);
        }
        Self {
            edges,
            widths,
            alignments,
            padding,
            reach: px(0.),
        }
    }

    fn width(&self) -> Pixels {
        self.edges.last().copied().unwrap_or_default()
    }

    /// x from the table's left edge where text `width` wide starts in `column`.
    fn text_x(&self, column: usize, width: Pixels) -> Pixels {
        let room = (self.widths[column] - width).max(px(0.));
        let offset = match self.alignments[column] {
            TableAlignment::None | TableAlignment::Left => px(0.),
            TableAlignment::Center => (room / 2.).round(),
            TableAlignment::Right => room,
        };
        self.edges[column] + self.padding + offset
    }
}

/// How a table line is laid out as part of a grid: part of its [`LineKey`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GridKey {
    pub row: TableRowKind,
    pub columns: Arc<Columns>,
}

/// What the cache needs to look a table up or measure it.
pub(crate) struct Source<'a> {
    pub markdown: &'a mut MarkdownState,
    pub buffer: &'a Buffer,
    pub selection: Option<Selection>,
    pub text_system: &'a WindowTextSystem,
}

struct Entry {
    /// The table's lines when measured; a table found on other lines is measured again.
    lines: Range<usize>,
    options: MarkdownOptions,
    columns: Arc<Columns>,
    /// When it was last used, for dropping the least recently used table.
    used: u64,
}

/// Column widths of the tables laid out recently, kept by their lines like the line layouts.
#[derive(Default)]
pub(crate) struct TableCache {
    entries: Vec<Entry>,
    clock: u64,
    /// Tables measured so far, for tests that check what a change re-measures.
    measured: u64,
}

impl TableCache {
    /// Makes `key` (for `line`) lay its line out as part of a grid if the line is in a table the selection is
    /// off, measuring the table if its widths are not kept. A grid that would not fit the text column with
    /// word wrap on is shown as typed: `key` then shows the table's markers.
    pub fn fill(&mut self, key: &mut LineKey, line: usize, base: &BaseStyle, source: Source) {
        let Some(row) = key.styled.style.table else {
            return;
        };
        let Some(table) = source.markdown.table(source.buffer, line, source.selection) else {
            return;
        };
        if table.revealed {
            return;
        }
        let columns = self.columns(&table, base, source);
        if base.soft_wrap && columns.reach > base.wrap_width {
            for span in &mut key.styled.spans {
                if matches!(
                    span.marker,
                    Some(MarkerKind::TablePipe | MarkerKind::TableDelimiter)
                ) {
                    span.hidden = false;
                }
            }
            return;
        }
        key.grid = Some(GridKey { row, columns });
    }

    fn columns(&mut self, table: &Table, base: &BaseStyle, source: Source) -> Arc<Columns> {
        self.clock += 1;
        let options = source.markdown.options();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.lines == table.lines && entry.options == options)
        {
            entry.used = self.clock;
            return entry.columns.clone();
        }
        self.measured += 1;
        let columns = Arc::new(measure(table, base, source));
        if self.entries.len() >= KEEP_TABLES
            && let Some(oldest) = (0..self.entries.len()).min_by_key(|&i| self.entries[i].used)
        {
            self.entries.swap_remove(oldest);
        }
        self.entries.push(Entry {
            lines: table.lines.clone(),
            options,
            columns: columns.clone(),
            used: self.clock,
        });
        columns
    }

    /// Drops the widths of tables on or next to the lines `change` touched (a line added right after a table
    /// may be a row of it) and renumbers the lines of those after it.
    pub fn apply_change(&mut self, change: &TextChange) {
        let first = change.start_point.line;
        let old_last = change.old_end_point.line;
        let new_last = change.new_end_point.line;
        self.entries.retain_mut(|entry| {
            if entry.lines.end < first || entry.lines.start > old_last + 1 {
                if entry.lines.start > old_last {
                    entry.lines.start = entry.lines.start - old_last + new_last;
                    entry.lines.end = entry.lines.end - old_last + new_last;
                }
                true
            } else {
                false
            }
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn measured(&self) -> u64 {
        self.measured
    }
}

/// The column widths of `table`: the widest cell text of each column over all its rows, styled as they are
/// with the selection off the table (inline markers hidden, header bold).
fn measure(table: &Table, base: &BaseStyle, source: Source) -> Columns {
    let mut widths = vec![px(0.); table.alignments.len()];
    let mut lead = px(0.);
    let mut indent = px(0.);
    let mut start = table.lines.start;
    while start < table.lines.end {
        let end = (start + MEASURE_CHUNK).min(table.lines.end);
        let styled = source
            .markdown
            .styled_lines(source.buffer, start..end, source.selection);
        for (line, styled) in (start..end).zip(&styled) {
            if styled.style.table == Some(TableRowKind::Delimiter) {
                continue;
            }
            let text = source.buffer.line_text(line);
            let row_lead =
                measure_cells(&text, styled, base, source.text_system, |column, width| {
                    if let Some(widest) = widths.get_mut(column) {
                        *widest = (*widest).max(width);
                    }
                });
            lead = lead.max(row_lead);
            if styled.style.quote {
                indent = base.quote_indent();
            }
        }
        start = end;
    }
    let mut columns = Columns::new(widths, table.alignments.clone(), base.font_size);
    columns.reach = indent + lead + columns.width();
    columns
}

/// The cells of a styled table row: each one's column and the display columns of its text, without the spaces
/// around it. `display` is the line's displayed text.
pub(super) fn cells(styled: &StyledLine, display: &str) -> Vec<(usize, Range<usize>)> {
    let mut cells: Vec<(usize, Range<usize>)> = Vec::new();
    let mut at = 0;
    for span in styled.spans.iter().filter(|span| !span.hidden) {
        let columns = at..at + span.columns.len();
        at = columns.end;
        let Some(column) = span.style.table_column else {
            continue;
        };
        let column = usize::from(column);
        match cells.last_mut() {
            Some((last, range)) if *last == column && range.end == columns.start => {
                range.end = columns.end;
            }
            _ => cells.push((column, columns)),
        }
    }
    for (_, range) in &mut cells {
        let text = &display[range.clone()];
        let trimmed = text.trim_start();
        let start = range.start + text.len() - trimmed.len();
        let end = start + trimmed.trim_end().len();
        *range = start..end;
    }
    cells
}

/// The display column where the table row starts: after any quote markers or indentation before it.
pub(super) fn row_start(styled: &StyledLine) -> usize {
    styled
        .spans
        .iter()
        .find(|span| span.style.table.is_some())
        .map_or(0, |span| styled.display_column(span.columns.start))
}

/// Moves the cells of a row laid out as part of `grid` to their columns by widening the room before each
/// cell's text (`glyphs` of the unwrapped line `width` wide), and makes the row as wide as the grid. Returns
/// the new width.
pub(super) fn position(
    styled: &StyledLine,
    display: &str,
    glyphs: &mut [Glyph],
    mut width: Pixels,
    grid: &GridKey,
) -> Pixels {
    let columns = &grid.columns;
    let start = row_start(styled);
    let left = unwrapped_x(glyphs, start, width);
    let mut at = start;
    for (column, text) in cells(styled, display) {
        if column >= columns.widths.len() {
            break;
        }
        let x = |glyphs: &[Glyph], width, column| unwrapped_x(glyphs, column, width);
        let text_width = x(glyphs, width, text.end) - x(glyphs, width, text.start);
        let target = left + columns.text_x(column, text_width);
        let room = (target - x(glyphs, width, at)).max(px(0.));
        width = set_span_width(glyphs, at..text.start, room, width);
        at = text.end;
    }
    let room = (left + columns.width() - unwrapped_x(glyphs, at, width)).max(px(0.));
    set_span_width(glyphs, at..display.len(), room, width)
}

/// The rectangles drawn under a grid row whose table starts at `left` in a line `height` tall: the header's
/// background, the delimiter row's rule and the lines between columns.
pub(super) fn shapes(
    grid: &GridKey,
    left: Pixels,
    height: Pixels,
    theme: &Theme,
) -> Vec<(Bounds<Pixels>, Hsla)> {
    let columns = &grid.columns;
    let mut shapes = Vec::with_capacity(columns.edges.len());
    let width = columns.width();
    match grid.row {
        TableRowKind::Header => {
            shapes.push((
                Bounds::new(point(left, px(0.)), size(width, height)),
                theme.surface,
            ));
        }
        TableRowKind::Delimiter => {
            let y = (height / 2.).floor();
            shapes.push((
                Bounds::new(point(left, y), size(width, px(1.))),
                theme.border,
            ));
        }
        TableRowKind::Body => {}
    }
    let inner = &columns.edges[1..columns.edges.len().saturating_sub(1).max(1)];
    for &edge in inner {
        shapes.push((
            Bounds::new(point(left + edge, px(0.)), size(px(1.), height)),
            theme.border,
        ));
    }
    shapes
}
