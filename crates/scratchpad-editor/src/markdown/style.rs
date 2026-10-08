//! Per-line styles for rendering and the live preview rules that decide which syntax markers are shown
//! (PLAN §18, §20, §22; ADR 0042).

use std::collections::BTreeSet;
use std::ops::Range;

use super::{Decoration, DecorationKind, MarkdownState};
use crate::buffer::Buffer;
use crate::coords::{Bias, ByteOffset};
use crate::selection::Selection;

/// The combined text style of a span. A renderer maps it to font size, weight, slant, decoration and colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SpanStyle {
    /// Heading level 1–6, or 0 outside headings.
    pub heading: u8,
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    /// Inline code.
    pub code: bool,
    pub link: bool,
    pub quote: bool,
    pub code_block: bool,
}

/// Which syntax a marker span belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerKind {
    /// `#`s with their spaces, a closing `#` sequence or a setext underline.
    Heading,
    /// `*`, `_`, `**`, `__`, `~` or `~~` around emphasis, strong or strikethrough text.
    Emphasis,
    /// Backticks around inline code.
    Code,
    /// `[`, `](destination)`, `<` or `>` of a link.
    Link,
    /// `>` and the space after it.
    Quote,
    ListBullet,
    ListNumber,
    TaskBox {
        checked: bool,
    },
    ThematicBreak,
    /// A whole fence line, info string included.
    CodeFence,
}

/// A run of a line with one style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StyledSpan {
    /// Byte columns within the line, as in [`Point::column`](crate::Point).
    pub columns: Range<usize>,
    pub style: SpanStyle,
    /// Set if the span is (part of) a syntax marker.
    pub marker: Option<MarkerKind>,
    /// Live preview hides this marker: it takes no space and is not drawn. List bullets, numbers and task boxes
    /// are never hidden; a renderer may draw them differently instead (e.g. `•` for `-`).
    pub hidden: bool,
}

/// How to draw one line.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct StyledLine {
    /// Block styles covering the whole line, even where it has no text: heading level for the line height, quote
    /// and code block for backgrounds and bars.
    pub style: SpanStyle,
    /// Consecutive spans covering the line from column 0 to its end; empty for an empty line.
    pub spans: Vec<StyledSpan>,
}

impl StyledLine {
    /// The line as displayed: its text without hidden spans. `line` is the line's text.
    pub fn display_text(&self, line: &str) -> String {
        self.spans
            .iter()
            .filter(|span| !span.hidden)
            .filter_map(|span| line.get(span.columns.clone()))
            .collect()
    }

    /// The byte column in [`StyledLine::display_text`] of byte `column` of the line. A column inside a hidden
    /// span maps to where the span would be.
    pub fn display_column(&self, column: usize) -> usize {
        let hidden_before: usize = self
            .spans
            .iter()
            .filter(|span| span.hidden)
            .map(|span| {
                column
                    .min(span.columns.end)
                    .saturating_sub(span.columns.start)
            })
            .sum();
        column - hidden_before
    }

    /// The byte column of the line shown at byte `display_column` of [`StyledLine::display_text`], e.g. for a
    /// click. Where hidden spans sit, [`Bias::Left`] picks the column before them and [`Bias::Right`] the one
    /// after them. Columns past the end map to the end of the line.
    pub fn buffer_column(&self, display_column: usize, bias: Bias) -> usize {
        let mut first = None;
        let mut last = None;
        let mut shown = 0;
        for span in &self.spans {
            let (start, end) = (span.columns.start, span.columns.end);
            if span.hidden {
                if shown == display_column {
                    first.get_or_insert(start);
                    last = Some(end);
                }
            } else {
                if (shown..=shown + (end - start)).contains(&display_column) {
                    let column = start + display_column - shown;
                    first.get_or_insert(column);
                    last = Some(column);
                }
                shown += end - start;
            }
        }
        let line_end = self.spans.last().map_or(0, |span| span.columns.end);
        match bias {
            Bias::Left => first.unwrap_or(line_end),
            Bias::Right => last.unwrap_or(line_end),
        }
    }
}

impl MarkdownState {
    /// Styles for `lines` (clamped to the document), parsing only the regions they touch. With a `selection`
    /// (live preview), markers away from it are hidden; without one (source mode) nothing is.
    pub fn styled_lines(
        &mut self,
        buffer: &Buffer,
        lines: Range<usize>,
        selection: Option<Selection>,
    ) -> Vec<StyledLine> {
        let lines = lines.start..lines.end.min(buffer.line_count());
        if lines.is_empty() {
            return Vec::new();
        }
        let range = buffer.line_start(lines.start)..buffer.line_end(lines.end - 1);
        let decorations = self.decorations(buffer, range);
        let selection = selection.map(|s| s.range());
        lines
            .map(|line| style_line(buffer, &decorations, line, selection.clone()))
            .collect()
    }
}

fn style_line(
    buffer: &Buffer,
    decorations: &[Decoration],
    line: usize,
    selection: Option<Range<ByteOffset>>,
) -> StyledLine {
    let line_start = buffer.line_start(line);
    let line_end = buffer.line_end(line);
    let on_line: Vec<&Decoration> = decorations
        .iter()
        .filter(|d| d.range.start <= line_end && line_start <= d.range.end)
        .collect();

    let mut style = SpanStyle::default();
    for d in &on_line {
        let block = matches!(
            d.kind,
            DecorationKind::Heading { .. }
                | DecorationKind::BlockQuote
                | DecorationKind::CodeBlock { .. }
        );
        // Blocks inside list items and quotes start after their markers, mid-line.
        if block {
            apply(&d.kind, &mut style);
        }
    }

    // Every range edge on the line splits it; between two edges the style is constant. A sweep over the
    // edges keeps the ranges and markers covering the current piece, so a long line with many spans costs
    // O(n log n) rather than checking every decoration for every piece.
    let mut edges: Vec<(ByteOffset, Edge)> = Vec::new();
    for (index, d) in on_line.iter().enumerate() {
        let covers = std::iter::once((None, &d.range))
            .chain(d.markers.iter().enumerate().map(|(m, r)| (Some(m), r)));
        for (marker, range) in covers {
            let (start, end) = (range.start.max(line_start), range.end.min(line_end));
            // Empty and off-line ranges cover no piece.
            if start < end {
                edges.push((start, Edge::Start(index, marker)));
                edges.push((end, Edge::End(index, marker)));
            }
        }
    }
    edges.sort_by_key(|(offset, _)| *offset);

    let mut ranges = BTreeSet::new();
    let mut markers = BTreeSet::new();
    let mut edges = edges.into_iter().peekable();
    let mut spans: Vec<StyledSpan> = Vec::new();
    let mut at = line_start;
    while at < line_end {
        while let Some((_, edge)) = edges.next_if(|(offset, _)| *offset == at) {
            match edge {
                Edge::Start(index, None) => ranges.insert(index),
                Edge::End(index, None) => ranges.remove(&index),
                Edge::Start(index, Some(m)) => markers.insert((index, m)),
                Edge::End(index, Some(m)) => markers.remove(&(index, m)),
            };
        }
        let piece = at..edges.peek().map_or(line_end, |(offset, _)| *offset);
        at = piece.end;

        let mut span_style = SpanStyle::default();
        for &index in &ranges {
            apply(&on_line[index].kind, &mut span_style);
        }
        // The innermost marker: decorations are ordered with enclosing ones first.
        let marker = markers
            .last()
            .map(|&(index, m)| (on_line[index], &on_line[index].markers[m]));
        let hidden = marker.is_some_and(|(d, m)| {
            hideable(&d.kind)
                && selection
                    .as_ref()
                    .is_some_and(|selection| !reveals(buffer, selection, d, m))
        });
        let span = StyledSpan {
            columns: piece.start.0 - line_start.0..piece.end.0 - line_start.0,
            style: span_style,
            marker: marker.map(|(d, _)| marker_kind(&d.kind)),
            hidden,
        };
        match spans.last_mut() {
            Some(last)
                if last.style == span.style
                    && last.marker == span.marker
                    && last.hidden == span.hidden =>
            {
                last.columns.end = span.columns.end;
            }
            _ => spans.push(span),
        }
    }
    StyledLine { style, spans }
}

/// Where a decoration's range (marker `None`) or one of its markers starts or ends on a line; indices into the
/// line's decorations and the decoration's markers.
#[derive(Clone, Copy)]
enum Edge {
    Start(usize, Option<usize>),
    End(usize, Option<usize>),
}

fn apply(kind: &DecorationKind, style: &mut SpanStyle) {
    match kind {
        DecorationKind::Heading { level } => style.heading = *level,
        DecorationKind::Strong => style.bold = true,
        DecorationKind::Emphasis => style.italic = true,
        DecorationKind::Strikethrough => style.strikethrough = true,
        DecorationKind::InlineCode => style.code = true,
        DecorationKind::Link { .. } => style.link = true,
        DecorationKind::BlockQuote => style.quote = true,
        DecorationKind::CodeBlock { .. } => style.code_block = true,
        DecorationKind::ListItem { .. }
        | DecorationKind::Task { .. }
        | DecorationKind::ThematicBreak => {}
    }
}

fn marker_kind(kind: &DecorationKind) -> MarkerKind {
    match kind {
        DecorationKind::Heading { .. } => MarkerKind::Heading,
        DecorationKind::Strong | DecorationKind::Emphasis | DecorationKind::Strikethrough => {
            MarkerKind::Emphasis
        }
        DecorationKind::InlineCode => MarkerKind::Code,
        DecorationKind::Link { .. } => MarkerKind::Link,
        DecorationKind::BlockQuote => MarkerKind::Quote,
        DecorationKind::ListItem { ordered: false } => MarkerKind::ListBullet,
        DecorationKind::ListItem { ordered: true } => MarkerKind::ListNumber,
        DecorationKind::Task { checked } => MarkerKind::TaskBox { checked: *checked },
        DecorationKind::ThematicBreak => MarkerKind::ThematicBreak,
        DecorationKind::CodeBlock { .. } => MarkerKind::CodeFence,
    }
}

/// List bullets, numbers and task boxes stay in place: hiding them would shift the item text.
fn hideable(kind: &DecorationKind) -> bool {
    !matches!(
        kind,
        DecorationKind::ListItem { .. } | DecorationKind::Task { .. }
    )
}

/// Whether live preview shows `marker` of `decoration` for `selection`: the selection touches (overlaps or is
/// adjacent to) the marker's scope. Touching rather than overlapping means the cursor is never next to a hidden
/// marker, so cursor motion never steps over invisible text.
fn reveals(
    buffer: &Buffer,
    selection: &Range<ByteOffset>,
    decoration: &Decoration,
    marker: &Range<ByteOffset>,
) -> bool {
    let lines_of = |range: &Range<ByteOffset>| {
        buffer.line_start(buffer.line_of(range.start))..buffer.line_end(buffer.line_of(range.end))
    };
    let scope = match decoration.kind {
        DecorationKind::Strong
        | DecorationKind::Emphasis
        | DecorationKind::Strikethrough
        | DecorationKind::InlineCode
        | DecorationKind::Link { .. } => decoration.range.clone(),
        DecorationKind::BlockQuote
        | DecorationKind::ListItem { .. }
        | DecorationKind::Task { .. } => lines_of(marker),
        DecorationKind::Heading { .. }
        | DecorationKind::ThematicBreak
        | DecorationKind::CodeBlock { .. } => lines_of(&decoration.range),
    };
    selection.start <= scope.end && scope.start <= selection.end
}
