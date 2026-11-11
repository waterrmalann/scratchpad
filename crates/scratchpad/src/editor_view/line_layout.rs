//! One logical line styled from its Markdown, shaped by GPUI's text system, soft-wrapped and painted
//! row by row (PLAN §18–20, §27; ADR 0070, 0071).
//!
//! The line is shaped and wrapped as it is displayed: without the syntax markers live preview hides,
//! and with list bullets and task boxes drawn as shapes. Columns in the API are byte columns of the
//! buffer line; the line's [`StyledLine`] maps them to and from columns of the displayed text, which
//! is what [`LineGeometry`] works in.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    Bounds, Font, FontStyle, FontWeight, Hsla, PathBuilder, Pixels, Point, SharedString, TextRun,
    Window, WindowTextSystem, fill, font, point, px, quad, size, transparent_black,
};
use scratchpad_core::PageIndex;
use scratchpad_editor::markdown::{
    CalloutKind, MarkdownState, MarkerKind, SpanStyle, StyledLine, StyledSpan, TableRowKind,
};
use scratchpad_editor::{Bias, Buffer, Selection};

use super::geometry::{Glyph, Indents, LineGeometry, expand_tabs, set_span_width, unwrapped_x};
use super::table::{self, GridKey, TableCache, TableLayout};
use crate::theme::{Theme, typography};

/// Heading sizes as multiples of the body size: the plan's 1.8 / 1.5 / 1.25 (PLAN §27), then
/// levels 4–6 set apart by weight more than by size.
const HEADING_SCALES: [f32; 6] = [
    typography::H1_SCALE,
    typography::H2_SCALE,
    typography::H3_SCALE,
    1.1,
    1.0,
    1.0,
];
/// Line height of headings as a multiple of their size: large type needs less leading.
const HEADING_LINE_HEIGHT: f32 = 1.3;
/// Space above a heading, in multiples of the body size, by level.
const HEADING_SPACE_ABOVE: [f32; 6] = [1.0, 0.8, 0.6, 0.4, 0.4, 0.4];
/// Monospace fonts look larger than the body font at the same size.
const CODE_SCALE: f32 = 0.88;
/// Room between a code block's text and the left edge of its background, at the normal text size.
const CODE_BLOCK_PADDING: Pixels = px(14.);
/// Room left and right of inline code on its background, at the normal text size.
const INLINE_CODE_PADDING: Pixels = px(3.);
const INLINE_CODE_RADIUS: Pixels = px(3.);
/// Quote text sits this far right of the quote's bar, at the normal text size.
const QUOTE_INDENT: Pixels = px(18.);
const QUOTE_BAR_WIDTH: Pixels = px(3.);
/// Superscript and subscript text size, and how far their baseline moves up or down, in multiples
/// of the line's text size.
const SCRIPT_SCALE: f32 = 0.75;
const SUPERSCRIPT_RISE: f32 = 0.33;
const SUBSCRIPT_DROP: f32 = 0.15;
/// Length of the dashes (and of the gaps between them) under a wikilink to a missing page, in
/// multiples of the font size.
const MISSING_PAGE_DASH: f32 = 0.2;
/// Room after a callout's label, in multiples of the font size.
const CALLOUT_LABEL_GAP: f32 = 0.5;
/// Width of a bullet, and of a task box, with the space after it, in multiples of the font size.
const BULLET_WIDTH: f32 = 1.15;
const TASK_BOX_WIDTH: f32 = 1.6;
/// Side of a task box, in multiples of the font size.
const TASK_BOX_SIZE: f32 = 0.92;

/// Text settings shared by every line: fonts and colours from the theme and the text column width.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BaseStyle {
    pub font: Font,
    pub mono_family: SharedString,
    /// The body text size, zoom included.
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub theme: Theme,
    /// Width of the text column.
    pub wrap_width: Pixels,
    /// Whether lines wrap at `wrap_width`; otherwise each line is one row.
    pub soft_wrap: bool,
    /// How far a heading's `#`s may hang into the margin left of the text column.
    pub hang_room: Pixels,
}

impl BaseStyle {
    /// A distance given for the normal text size, scaled with the zoom.
    fn scaled(&self, pixels: Pixels) -> Pixels {
        (pixels * (self.font_size / typography::BODY_FONT_SIZE)).round()
    }

    /// Whether text shaped with `other` is as wide as with this style: the same fonts and size. Only the
    /// wrap width, the colours and the margin may differ.
    pub(super) fn same_text_widths(&self, other: &BaseStyle) -> bool {
        self.font == other.font
            && self.mono_family == other.mono_family
            && self.font_size == other.font_size
    }

    /// How far quote text sits right of the text column's left edge.
    pub(super) fn quote_indent(&self) -> Pixels {
        self.scaled(QUOTE_INDENT)
    }
}

/// Everything besides its text that a line's layout depends on. A cached layout is reused while its
/// key stays the same, so moving the cursor only re-shapes lines whose markers it reveals or hides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct LineKey {
    pub styled: StyledLine,
    /// Draws list bullets and task boxes as shapes, and a callout's tag as its label, rather than
    /// as the characters typed.
    pub widgets: bool,
    /// Lays a table line out as part of an aligned grid (ADR 0186); set by [`KeyContext::key`].
    pub grid: Option<GridKey>,
}

/// What a line's [`LineKey`] is built from besides the layout cache: the document, how it is shown
/// and the text system tables are measured with.
pub(crate) struct KeyContext<'a> {
    pub buffer: &'a Buffer,
    /// `None` in plain text.
    pub markdown: Option<&'a mut MarkdownState>,
    /// The pages wikilinks resolve to, if known.
    pub pages: Option<&'a PageIndex>,
    /// The selection markers are revealed around; `None` in source mode and plain text.
    pub selection: Option<Selection>,
    pub text_system: &'a WindowTextSystem,
}

impl KeyContext<'_> {
    /// The same context with the markers revealed around `selection` instead, e.g. with the cursor
    /// moved somewhere else.
    pub fn with_selection(&mut self, selection: Option<Selection>) -> KeyContext<'_> {
        KeyContext {
            buffer: self.buffer,
            markdown: self.markdown.as_deref_mut(),
            pages: self.pages,
            selection,
            text_system: self.text_system,
        }
    }

    /// How `line` looks: every reveal rule is applied here. In live preview (a selection) markers
    /// away from the selection are hidden, bullets and task boxes are drawn as shapes unless the
    /// selection reaches inside them, and a table the selection is off is laid out as a grid with
    /// the widths kept in `tables`, or shown as typed when it cannot be one; in source mode
    /// everything shows as typed. Wikilinks whose page is not in `pages` are drawn as missing; with
    /// no `pages`, none is.
    pub fn key(&mut self, line: usize, tables: &mut TableCache, base: &BaseStyle) -> LineKey {
        let (buffer, pages, selection) = (self.buffer, self.pages, self.selection);
        let Some(markdown) = self.markdown.as_deref_mut() else {
            return LineKey::plain(buffer, line);
        };
        let page_exists = |page: &str| pages.is_none_or(|pages| pages.has_page(page));
        let styled = markdown
            .styled_lines_with_pages(buffer, line..line + 1, selection, page_exists)
            .pop()
            .unwrap_or_default();
        let line_start = buffer.line_start(line).0;
        let line_end = buffer.line_end(line).0;
        let widgets = selection.is_some_and(|selection| {
            !widgets(&styled).iter().any(|widget| match widget.kind {
                // A callout's label shows while the selection is off its line, as the quote's
                // `>` does: the tag is typed text the cursor may want to edit.
                WidgetKind::Callout { .. } => {
                    selection.start().0 <= line_end && selection.end().0 >= line_start
                }
                _ => {
                    selection.start().0 < line_start + widget.columns.end
                        && selection.end().0 > line_start + widget.columns.start
                }
            })
        });
        let mut key = LineKey {
            styled,
            widgets,
            grid: None,
        };
        if let Some(row) = key.styled.style.table {
            match tables.layout(line, row, base, self) {
                Some(TableLayout::Grid(grid)) => key.grid = Some(grid),
                // Too wide or too long for a grid: the table's markers show.
                Some(TableLayout::AsTyped) => {
                    for span in &mut key.styled.spans {
                        if matches!(
                            span.marker,
                            Some(MarkerKind::TablePipe | MarkerKind::TableDelimiter)
                        ) {
                            span.hidden = false;
                        }
                    }
                }
                None => {}
            }
        }
        key
    }
}

impl LineKey {
    /// How `line` looks in a plain text file: one unstyled span, with no markers.
    pub fn plain(buffer: &Buffer, line: usize) -> Self {
        let len = buffer.line_end(line).0 - buffer.line_start(line).0;
        let spans = (len > 0)
            .then(|| StyledSpan {
                columns: 0..len,
                style: SpanStyle::default(),
                marker: None,
                hidden: false,
            })
            .into_iter()
            .collect();
        Self {
            styled: StyledLine {
                style: SpanStyle::default(),
                spans,
            },
            widgets: false,
            grid: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum WidgetKind {
    Bullet,
    /// `column` is the buffer column of the box's `[`.
    TaskBox {
        checked: bool,
        column: usize,
    },
    /// A callout's `[!TAG]`, drawn as the kind's label.
    Callout {
        kind: CalloutKind,
    },
}

/// A list marker drawn as a shape, or a callout tag drawn as its label, in live preview.
struct Widget {
    kind: WidgetKind,
    /// Buffer columns at first, display columns once the line is laid out.
    columns: Range<usize>,
    /// A callout's label, shaped with the line.
    label: Option<Arc<gpui::LineLayout>>,
}

/// The line's bullets, task boxes and callout tags. A bullet followed by a task box is drawn as
/// just the box.
fn widgets(styled: &StyledLine) -> Vec<Widget> {
    let mut widgets: Vec<Widget> = Vec::new();
    for span in &styled.spans {
        let kind = match span.marker {
            Some(MarkerKind::ListBullet) => WidgetKind::Bullet,
            Some(MarkerKind::TaskBox { checked }) => WidgetKind::TaskBox {
                checked,
                column: span.columns.start,
            },
            Some(MarkerKind::Callout { kind }) => WidgetKind::Callout { kind },
            _ => continue,
        };
        match widgets.last_mut() {
            Some(last)
                if last.kind == WidgetKind::Bullet && last.columns.end == span.columns.start =>
            {
                last.kind = kind;
                last.columns.end = span.columns.end;
            }
            _ => widgets.push(Widget {
                kind,
                columns: span.columns.clone(),
                label: None,
            }),
        }
    }
    widgets
}

/// A displayed run of text in one style.
struct Run {
    /// Display columns.
    columns: Range<usize>,
    style: SpanStyle,
    marker: bool,
    font: Font,
    size: Pixels,
    /// How far the baseline is moved down (up if negative): superscript and subscript.
    shift: Pixels,
    color: Hsla,
}

/// Something drawn besides glyphs, relative to the line's top-left corner.
enum Shape {
    /// Code backgrounds, lines through and under text, rules, quote bars and bullets.
    Rect {
        bounds: Bounds<Pixels>,
        color: Hsla,
        radius: Pixels,
    },
    TaskBox {
        bounds: Bounds<Pixels>,
        checked: bool,
        color: Hsla,
        /// Colour of the check mark.
        mark: Hsla,
        /// Buffer column of the box's `[`.
        column: usize,
    },
    /// Text that is not in the buffer: a callout's label. `bounds` is its row's box.
    Label {
        bounds: Bounds<Pixels>,
        baseline: Pixels,
        shaped: Arc<gpui::LineLayout>,
        font_size: Pixels,
        color: Hsla,
    },
}

impl Shape {
    fn rect(bounds: Bounds<Pixels>, color: Hsla) -> Self {
        Self::Rect {
            bounds,
            color,
            radius: px(0.),
        }
    }

    fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Shape::Rect { bounds, .. }
            | Shape::TaskBox { bounds, .. }
            | Shape::Label { bounds, .. } => *bounds,
        }
    }

    fn paint(&self, origin: Point<Pixels>, window: &mut Window) {
        match *self {
            Shape::Rect {
                bounds,
                color,
                radius,
            } => window.paint_quad(fill(bounds + origin, color).corner_radii(radius)),
            Shape::TaskBox {
                bounds,
                checked,
                color,
                mark,
                ..
            } => {
                let bounds = bounds + origin;
                let side = bounds.size.width;
                let background = if checked { color } else { transparent_black() };
                let border = px(1.5);
                window.paint_quad(quad(
                    bounds,
                    side * 0.22,
                    background,
                    border,
                    color,
                    Default::default(),
                ));
                if checked {
                    let at = |x: f32, y: f32| bounds.origin + point(side * x, side * y);
                    let mut check = PathBuilder::stroke(px(1.75));
                    check.move_to(at(0.26, 0.53));
                    check.line_to(at(0.43, 0.70));
                    check.line_to(at(0.75, 0.33));
                    if let Ok(path) = check.build() {
                        window.paint_path(path, mark);
                    }
                }
            }
            Shape::Label {
                bounds,
                baseline,
                ref shaped,
                font_size,
                color,
            } => {
                let origin = bounds.origin + origin + point(px(0.), baseline);
                for run in &shaped.runs {
                    for glyph in &run.glyphs {
                        let at = origin + point(glyph.position.x, px(0.));
                        // A glyph that fails to rasterise is skipped rather than failing the frame.
                        let _ = window.paint_glyph(at, run.font_id, glyph.id, font_size, color);
                    }
                }
            }
        }
    }
}

/// Text shaped in one font; a line has several when it mixes bold, italic or code with plain text.
struct Segment {
    shaped: Arc<gpui::LineLayout>,
    /// Display column of the segment's first byte; shaped glyph indices are relative to it.
    column: usize,
    /// Index of the segment's first glyph in the line's geometry.
    first_glyph: usize,
    font_size: Pixels,
    /// How far its baseline is moved down (up if negative).
    shift: Pixels,
}

/// Font and spacing of a whole line, from its block style.
struct LineFont {
    font: Font,
    size: Pixels,
    line_height: Pixels,
    space_above: Pixels,
}

impl LineFont {
    fn new(block: SpanStyle, base: &BaseStyle) -> Self {
        if (1..=6).contains(&block.heading) {
            let level = block.heading as usize - 1;
            let size = base.font_size * HEADING_SCALES[level];
            return Self {
                font: Font {
                    weight: FontWeight::SEMIBOLD,
                    ..base.font.clone()
                },
                size,
                line_height: (size * HEADING_LINE_HEIGHT).max(base.line_height),
                space_above: (base.font_size * HEADING_SPACE_ABOVE[level]).round(),
            };
        }
        let (font, size) = if block.code_block {
            (font(base.mono_family.clone()), base.font_size * CODE_SCALE)
        } else {
            (base.font.clone(), base.font_size)
        };
        Self {
            font,
            size,
            line_height: base.line_height,
            space_above: px(0.),
        }
    }
}

/// A shaped and wrapped line.
pub(crate) struct LineLayout {
    segments: Vec<Segment>,
    geometry: LineGeometry,
    styled: StyledLine,
    /// Whether markers are hidden, so that buffer and display columns differ.
    hides: bool,
    /// Length of the buffer line.
    len: usize,
    /// Buffer column where the text starts, after block markers.
    text_start: usize,
    /// Bullets and task boxes in display columns: the cursor never stops inside them.
    widgets: Vec<Range<usize>>,
    /// `(end display column, colour)` of each run.
    colors: Vec<(usize, Hsla)>,
    /// Shapes painted below the selection and the text, and above the text.
    below: Vec<Shape>,
    above: Vec<Shape>,
    code_block: bool,
    font_size: Pixels,
    line_height: Pixels,
    space_above: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

impl LineLayout {
    pub fn shape(
        text: &str,
        key: &LineKey,
        base: &BaseStyle,
        text_system: &WindowTextSystem,
    ) -> Self {
        let block = key.styled.style;
        let mut line_font = LineFont::new(block, base);
        // A grid's delimiter row is drawn as a rule in a short row.
        if key.grid.as_ref().map(|grid| grid.row) == Some(TableRowKind::Delimiter) {
            line_font.line_height = (base.font_size * table::DELIMITER_ROW_HEIGHT).round();
        }
        let mut widgets = if key.widgets {
            widgets(&key.styled)
        } else {
            Vec::new()
        };
        let runs = runs(&key.styled, &widgets, &line_font, base);
        let mut display = String::with_capacity(text.len());
        for span in key.styled.spans.iter().filter(|span| !span.hidden) {
            display.push_str(&text[span.columns.clone()]);
        }
        let (segments, mut glyphs, width) = shape_runs(&display, &runs, text_system);
        // Tabs are shaped as spaces (same byte length, so columns are unchanged) and then widened to
        // tab stops; fonts give tab characters no width at all.
        let mut width = expand_tabs(&display, &mut glyphs, width);

        let display_of = |column: usize| key.styled.display_column(column);
        for widget in &mut widgets {
            widget.columns = display_of(widget.columns.start)..display_of(widget.columns.end);
            let scale = match widget.kind {
                WidgetKind::Bullet => BULLET_WIDTH,
                WidgetKind::TaskBox { .. } => TASK_BOX_WIDTH,
                WidgetKind::Callout { kind } => {
                    let label = callout_label(kind, &line_font, text_system);
                    let scale = label.width / line_font.size + CALLOUT_LABEL_GAP;
                    widget.label = Some(label);
                    scale
                }
            };
            let span_width = (line_font.size * scale).round();
            width = set_span_width(&mut glyphs, widget.columns.clone(), span_width, width);
        }
        let mut shown = key.styled.spans.iter().filter(|span| !span.hidden);
        let mut item_markers = shown.clone().filter(|span| {
            matches!(
                span.marker,
                Some(MarkerKind::ListBullet | MarkerKind::ListNumber | MarkerKind::TaskBox { .. })
            )
        });
        // Spaces are narrow in a proportional font: a nested item's indentation is widened so that
        // two spaces, the indentation CommonMark needs under `- `, line it up with its parent's text.
        if let Some(marker) = item_markers.clone().next()
            && text[..marker.columns.start]
                .bytes()
                .all(|byte| byte == b' ')
        {
            let indent = line_font.size * BULLET_WIDTH / 2. * marker.columns.start as f32;
            let columns = 0..display_of(marker.columns.start);
            width = set_span_width(&mut glyphs, columns, indent.round(), width);
        }

        if let Some(grid) = &key.grid {
            // A table in a list item: indentation lines its later rows up with the first, which has the
            // bullet, as above.
            let spaces = text.bytes().take_while(|&byte| byte == b' ').count();
            if item_markers.clone().next().is_none()
                && spaces > 0
                && display_of(spaces) == table::row_start(&key.styled)
            {
                let indent = line_font.size * BULLET_WIDTH / 2. * spaces as f32;
                width = set_span_width(&mut glyphs, 0..display_of(spaces), indent.round(), width);
            }
            width = table::position(&key.styled, &display, &mut glyphs, width, grid);
        }

        let mut indents = Indents::default();
        // A grid row is never wrapped (ADR 0186).
        let mut wrap_width = if base.soft_wrap && key.grid.is_none() {
            base.wrap_width
        } else {
            Pixels::MAX
        };
        if block.code_block {
            let padding = base.scaled(CODE_BLOCK_PADDING);
            indents.first = padding;
            wrap_width -= padding;
        } else if block.quote {
            indents.first = base.scaled(QUOTE_INDENT);
        }
        indents.rest = indents.first;
        // Wrapped rows of a list item line up with its text rather than with its bullet.
        if let Some(marker) = item_markers.next_back() {
            indents.rest += unwrapped_x(&glyphs, display_of(marker.columns.end), width);
        }
        // A heading's `#`s hang in the margin, so revealing them does not move the heading's text;
        // in a margin too narrow for them, the text moves by what does not fit.
        if let Some(first) = shown.next()
            && first.marker == Some(MarkerKind::Heading)
            && first.columns.start == 0
        {
            let markers = unwrapped_x(&glyphs, display_of(first.columns.end), width);
            indents.first -= markers.min(base.hang_room);
        }

        let font_id = text_system.resolve_font(&line_font.font);
        let mut layout = Self {
            segments,
            geometry: LineGeometry::new(&display, glyphs, width, wrap_width, indents),
            styled: key.styled.clone(),
            hides: key.styled.spans.iter().any(|span| span.hidden),
            len: text.len(),
            text_start: text_start(text, &key.styled),
            widgets: widgets.iter().map(|w| w.columns.clone()).collect(),
            colors: runs
                .iter()
                .map(|run| (run.columns.end, run.color))
                .collect(),
            below: Vec::new(),
            above: Vec::new(),
            code_block: block.code_block,
            font_size: line_font.size,
            line_height: line_font.line_height,
            space_above: line_font.space_above,
            // Metrics come from the font rather than the shaped glyphs so that empty lines get the
            // same caret and baseline as lines with text. Font metrics report the descent as negative.
            ascent: text_system.ascent(font_id, line_font.size),
            descent: -text_system.descent(font_id, line_font.size),
        };
        layout.add_shapes(&display, &runs, &widgets, key, base);
        layout
    }

    /// Like [`LineGeometry::spans`] for `columns` of `display`, but rows that wrap at a space end
    /// before it, so underlines and backgrounds stop with the text.
    fn text_spans(&self, display: &str, columns: Range<usize>) -> Vec<(usize, Pixels, Pixels)> {
        let mut spans = self.geometry.spans(columns, false, px(0.));
        if let Some((_, rows)) = spans.split_last_mut() {
            for (row, _, x1) in rows {
                let end = self.geometry.row_end(*row);
                if display[end..].starts_with(char::is_whitespace) {
                    *x1 = self.geometry.x_for(end);
                }
            }
        }
        spans
    }

    /// Code backgrounds, lines through and under text, bullets, task boxes, rules and quote bars.
    fn add_shapes(
        &mut self,
        display: &str,
        runs: &[Run],
        widgets: &[Widget],
        key: &LineKey,
        base: &BaseStyle,
    ) {
        let theme = &base.theme;
        let font_size = self.font_size;
        let baseline = self.baseline();

        let code_size = font_size * CODE_SCALE;
        for columns in merged(runs, |run| run.style.code && !run.style.code_block) {
            for (row, x0, x1) in self.text_spans(display, columns) {
                let top = self.row_top(row) + baseline - code_size;
                let padding = base.scaled(INLINE_CODE_PADDING);
                let bounds = Bounds::new(
                    point(x0 - padding, top),
                    size(x1 - x0 + padding * 2., code_size * 1.38),
                );
                self.below.push(Shape::Rect {
                    bounds,
                    color: theme.surface,
                    radius: INLINE_CODE_RADIUS,
                });
            }
        }
        let lines = [
            // Strikethrough at about the middle of lower-case letters, links underlined.
            (
                merged(runs, |run| run.style.strikethrough && !run.marker),
                baseline - (font_size * 0.3).round(),
                theme.muted,
            ),
            (
                merged(runs, |run| {
                    run.style.link && !run.marker && !run.style.missing_page
                }),
                baseline + (font_size * 0.15).round(),
                theme.accent,
            ),
        ];
        for (ranges, y, color) in lines {
            for columns in ranges {
                for (row, x0, x1) in self.text_spans(display, columns) {
                    let origin = point(x0, self.row_top(row) + y);
                    self.above.push(Shape::rect(
                        Bounds::new(origin, size(x1 - x0, px(1.))),
                        color,
                    ));
                }
            }
        }
        // A wikilink to a page that does not exist yet is underlined with dashes (ADR 0188).
        let dash = (font_size * MISSING_PAGE_DASH).round().max(px(1.));
        let missing = merged(runs, |run| run.style.missing_page && !run.marker);
        for columns in missing {
            for (row, x0, x1) in self.text_spans(display, columns) {
                let y = self.row_top(row) + baseline + (font_size * 0.15).round();
                let mut x = x0;
                while x < x1 {
                    let width = dash.min(x1 - x);
                    self.above.push(Shape::rect(
                        Bounds::new(point(x, y), size(width, px(1.))),
                        theme.muted,
                    ));
                    x += dash * 2.;
                }
            }
        }

        // Bullets and boxes sit at the height of the middle of lower-case letters.
        let middle = baseline - font_size * 0.3;
        for widget in widgets {
            let x = self.geometry.x_for(widget.columns.start);
            let top = self.row_top(self.geometry.row_of(widget.columns.start));
            self.above.push(match widget.kind {
                WidgetKind::Bullet => {
                    let diameter = (font_size * 0.3).round();
                    let origin = point(x + font_size * 0.3, top + middle - diameter / 2.);
                    Shape::Rect {
                        bounds: Bounds::new(origin, size(diameter, diameter)),
                        color: theme.muted,
                        radius: diameter / 2.,
                    }
                }
                WidgetKind::TaskBox { checked, column } => {
                    let side = (font_size * TASK_BOX_SIZE).round();
                    let origin = point(x + px(1.), (top + middle - side / 2.).round());
                    Shape::TaskBox {
                        bounds: Bounds::new(origin, size(side, side)),
                        checked,
                        color: if checked { theme.accent } else { theme.muted },
                        mark: theme.background,
                        column,
                    }
                }
                WidgetKind::Callout { kind } => {
                    let Some(shaped) = widget.label.clone() else {
                        continue;
                    };
                    Shape::Label {
                        bounds: Bounds::new(point(x, top), size(shaped.width, self.line_height)),
                        baseline,
                        shaped,
                        font_size,
                        color: theme.callouts.get(kind),
                    }
                }
            });
        }

        let hidden_rule = key
            .styled
            .spans
            .iter()
            .any(|span| span.hidden && span.marker == Some(MarkerKind::ThematicBreak));
        if hidden_rule {
            let origin = point(px(0.), (self.line_height / 2.).round());
            let bounds = Bounds::new(origin, size(base.wrap_width, px(1.)));
            self.above.push(Shape::rect(bounds, theme.border));
        }
        if let Some(grid) = &key.grid {
            let left = self.geometry.x_for(table::row_start(&key.styled));
            for (bounds, color) in table::shapes(grid, left, self.line_height, theme) {
                self.below.push(Shape::rect(bounds, color));
            }
        }
        if key.styled.style.quote {
            let bounds = Bounds::new(point(px(0.), px(0.)), size(QUOTE_BAR_WIDTH, self.height()));
            let callout = key.styled.style.callout;
            self.below.push(Shape::Rect {
                bounds,
                color: callout.map_or(theme.border, |kind| theme.callouts.get(kind)),
                radius: QUOTE_BAR_WIDTH / 2.,
            });
        }
    }

    // --- Columns: the API speaks buffer columns, the geometry display columns. ---

    fn display(&self, column: usize) -> usize {
        if self.hides {
            self.styled.display_column(column)
        } else {
            column
        }
    }

    fn buffer(&self, display: usize, bias: Bias) -> usize {
        if self.hides {
            self.styled.buffer_column(display, bias)
        } else {
            display
        }
    }

    pub fn row_count(&self) -> usize {
        self.geometry.row_count()
    }

    /// The row a cursor at `column` is drawn on.
    pub fn row_of(&self, column: usize) -> usize {
        self.geometry.row_of(self.display(column))
    }

    /// Where Home moves on `row`: before any markers hidden there.
    pub fn row_start(&self, row: usize) -> usize {
        self.buffer(self.geometry.row_start(row), Bias::Left)
    }

    /// Where End moves on `row`: the line end on the last row, else before the row's last glyph.
    pub fn row_end(&self, row: usize) -> usize {
        if row + 1 >= self.row_count() {
            self.len
        } else {
            self.buffer(self.geometry.row_end(row), Bias::Left)
        }
    }

    /// Where the line's text starts after any list bullet, task box, quote or heading markers:
    /// where Home goes first.
    pub fn text_start(&self) -> usize {
        self.text_start
    }

    /// Whether the byte at `column` belongs to a marker that is hidden.
    pub fn hides(&self, column: usize) -> bool {
        self.hides
            && self
                .styled
                .spans
                .iter()
                .any(|span| span.hidden && span.columns.contains(&column))
    }

    /// x of a cursor at `column`.
    pub fn x_for(&self, column: usize) -> Pixels {
        self.geometry.x_for(self.display(column))
    }

    /// x of the end of the line: how far an unwrapped line reaches.
    pub fn end_x(&self) -> Pixels {
        self.x_for(self.len)
    }

    /// The cursor position closest to `x` on `row`. Next to hidden markers it is the position after
    /// them, so a click just before a word lands in the word; it is never inside a bullet or task box.
    pub fn column_at(&self, row: usize, x: Pixels) -> usize {
        let mut display = self.geometry.column_at(row, x);
        if let Some(widget) = self
            .widgets
            .iter()
            .find(|widget| widget.start < display && display < widget.end)
        {
            display = widget.end;
        }
        self.buffer(display, Bias::Right)
    }

    /// The column of the character drawn at `x` on `row`, if any.
    pub fn column_under(&self, row: usize, x: Pixels) -> Option<usize> {
        let display = self.geometry.glyph_at(row, x)?;
        Some(self.buffer(display, Bias::Right))
    }

    /// The buffer column of the `[` of the task box drawn at `position` (relative to the line's
    /// top-left corner), if any.
    pub fn task_box_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.above.iter().find_map(|shape| match shape {
            // A little slack around the box makes it easier to hit.
            Shape::TaskBox { bounds, column, .. } if bounds.dilate(px(3.)).contains(&position) => {
                Some(*column)
            }
            _ => None,
        })
    }

    /// Horizontal spans `(row, x_start, x_end)` covered by `columns`, one per row it touches. With
    /// `newline`, the line break after the last column is selected too and shown as `newline_width`
    /// of extra space, so selected empty lines stay visible.
    pub fn spans(
        &self,
        columns: Range<usize>,
        newline: bool,
        newline_width: Pixels,
    ) -> Vec<(usize, Pixels, Pixels)> {
        let columns = self.display(columns.start)..self.display(columns.end);
        self.geometry.spans(columns, newline, newline_width)
    }

    /// Whether the line belongs to a code block, whose background spans the text column.
    pub fn code_block(&self) -> bool {
        self.code_block
    }

    // --- Vertical metrics ---

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    pub fn height(&self) -> Pixels {
        self.space_above + self.line_height * self.row_count() as f32
    }

    /// y of `row`'s top below the line's top. Everything vertical inside a line goes through this
    /// and [`LineLayout::row_at`].
    pub fn row_top(&self, row: usize) -> Pixels {
        self.space_above + self.line_height * row as f32
    }

    /// The row at `y` below the line's top; above or below the line, its first or last row.
    pub fn row_at(&self, y: Pixels) -> usize {
        let row = ((y - self.space_above) / self.line_height).floor().max(0.);
        (row as usize).min(self.row_count() - 1)
    }

    /// Distance from a row's top to the top of its text, and the text's height (for the caret).
    pub fn text_extent(&self) -> (Pixels, Pixels) {
        let height = self.ascent + self.descent;
        ((self.line_height - height) / 2., height)
    }

    /// Distance from a row's top to the baseline.
    pub fn baseline(&self) -> Pixels {
        self.text_extent().0 + self.ascent
    }

    // --- Painting ---

    /// Paints what goes below the selection and the text (code and quote backgrounds), the line's
    /// top-left at `origin`.
    pub fn paint_background(&self, origin: Point<Pixels>, window: &mut Window) {
        for shape in &self.below {
            shape.paint(origin, window);
        }
    }

    /// Paints the rows of the line that intersect the content mask, and the shapes drawn over the
    /// text. Only the glyphs within the mask are visited, so a frame (even a caret blink) costs the
    /// same on a megabyte-long line, wrapped or not, as on a short one.
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window) {
        let clip = window.content_mask().bounds;
        let first_row = self.row_at(clip.top() - origin.y);
        let last_row = self.row_at(clip.bottom() - origin.y);
        // Room for glyphs that reach left of their origin or are wider than usual.
        let slack = self.line_height * 2.;
        let clip_x = clip.left() - origin.x - slack..clip.right() - origin.x;
        let visible = self
            .geometry
            .row_glyphs_within(first_row, clip_x.clone())
            .start..self.geometry.row_glyphs_within(last_row, clip_x).end;
        let baseline = self.baseline();
        let mut row = first_row;
        for segment in &self.segments {
            // Geometry numbers glyphs across all segments and runs; `run_start` is this run's first.
            let mut run_start = segment.first_glyph;
            for run in &segment.shaped.runs {
                let run_glyphs = run_start..run_start + run.glyphs.len();
                run_start = run_glyphs.end;
                for glyph_ix in visible.start.max(run_glyphs.start)..visible.end.min(run_glyphs.end)
                {
                    let glyph = &run.glyphs[glyph_ix - run_glyphs.start];
                    let color = self.color_at(segment.column + glyph.index);
                    // Bullets, task boxes and callout tags are transparent: shapes are drawn in
                    // their place.
                    if color.a == 0. {
                        continue;
                    }
                    while row < last_row && glyph_ix >= self.geometry.row_glyphs(row + 1).start {
                        row += 1;
                    }
                    let origin = point(
                        origin.x + self.geometry.glyph_x(row, glyph_ix),
                        origin.y + self.row_top(row) + baseline + segment.shift,
                    );
                    // A glyph that fails to rasterise is skipped rather than failing the frame.
                    let _ = if glyph.is_emoji {
                        window.paint_emoji(origin, run.font_id, glyph.id, segment.font_size)
                    } else {
                        window.paint_glyph(origin, run.font_id, glyph.id, segment.font_size, color)
                    };
                }
            }
        }
        for shape in &self.above {
            let bounds = shape.bounds() + origin;
            if bounds.top() < clip.bottom() && bounds.bottom() > clip.top() {
                shape.paint(origin, window);
            }
        }
    }

    fn color_at(&self, column: usize) -> Hsla {
        let run = self.colors.partition_point(|(end, _)| *end <= column);
        self.colors
            .get(run)
            .map_or(Hsla::default(), |(_, color)| *color)
    }
}

/// The buffer column after the block markers (list bullets and numbers, task boxes, quotes and
/// heading `#`s) that `text` starts with, and the indentation before them; 0 without any.
fn text_start(text: &str, styled: &StyledLine) -> usize {
    let mut start = 0;
    for span in &styled.spans {
        let span_text = &text[span.columns.clone()];
        let block_marker = match span.marker {
            Some(MarkerKind::Heading) => span_text.starts_with('#'),
            Some(
                MarkerKind::Quote
                | MarkerKind::ListBullet
                | MarkerKind::ListNumber
                | MarkerKind::TaskBox { .. },
            ) => true,
            _ => false,
        };
        if block_marker {
            start = span.columns.end;
        } else if !span_text.trim().is_empty() {
            break;
        }
    }
    start
}

/// The displayed runs of a line: fonts, sizes and colours from the Markdown styles.
fn runs(
    styled: &StyledLine,
    widgets: &[Widget],
    line_font: &LineFont,
    base: &BaseStyle,
) -> Vec<Run> {
    let theme = &base.theme;
    let mut runs = Vec::with_capacity(styled.spans.len());
    let mut display_len = 0;
    let mut after_checked_box = false;
    for span in styled.spans.iter().filter(|span| !span.hidden) {
        let columns = display_len..display_len + span.columns.len();
        display_len = columns.end;
        let style = span.style;
        let mono = style.code || style.code_block;
        let mut font = if mono {
            font(base.mono_family.clone())
        } else {
            line_font.font.clone()
        };
        if style.bold || style.table == Some(TableRowKind::Header) {
            font.weight = FontWeight::BOLD;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        let mut size = if style.code && !style.code_block {
            line_font.size * CODE_SCALE
        } else {
            line_font.size
        };
        let shift = if style.superscript {
            -(line_font.size * SUPERSCRIPT_RISE).round()
        } else if style.subscript {
            (line_font.size * SUBSCRIPT_DROP).round()
        } else {
            px(0.)
        };
        if style.superscript || style.subscript {
            size *= SCRIPT_SCALE;
        }
        let in_widget = widgets
            .iter()
            .any(|widget| widget.columns.contains(&span.columns.start));
        // Markers, quotes and done tasks recede; the text of the note stays in front, also in
        // callouts, which are there to be read.
        let color = if in_widget {
            transparent_black()
        } else if span.marker.is_some()
            || after_checked_box
            || style.strikethrough
            || style.quote && style.callout.is_none()
            || style.heading == 6
            || style.missing_page
        {
            theme.muted
        } else {
            theme.foreground
        };
        if let Some(MarkerKind::TaskBox { checked }) = span.marker {
            after_checked_box = checked;
        }
        runs.push(Run {
            columns,
            style,
            marker: span.marker.is_some(),
            font,
            size,
            shift,
            color,
        });
    }
    runs
}

/// Shapes `display` (the displayed text) in segments of one font size. Returns the segments, the
/// glyphs of the whole line and its width.
fn shape_runs(
    display: &str,
    runs: &[Run],
    text_system: &WindowTextSystem,
) -> (Vec<Segment>, Vec<Glyph>, Pixels) {
    let shaped_text = display.replace('\t', " ");
    let mut segments = Vec::new();
    let mut glyphs = Vec::new();
    let mut width = px(0.);
    // One segment per font: GPUI 0.2.2's `layout_line` gives a run the previous run's font when
    // only the font changes between them, which would lose bold and italic. And one per baseline.
    for group in runs.chunk_by(|a, b| a.size == b.size && a.font == b.font && a.shift == b.shift) {
        let columns = group[0].columns.start..group[group.len() - 1].columns.end;
        let text_runs: Vec<TextRun> = group
            .iter()
            .map(|run| TextRun {
                len: run.columns.len(),
                font: run.font.clone(),
                color: run.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect();
        let size = group[0].size;
        let shaped = text_system.layout_line(&shaped_text[columns.clone()], size, &text_runs, None);
        let first_glyph = glyphs.len();
        glyphs.extend(
            shaped
                .runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| Glyph {
                    column: columns.start + glyph.index,
                    x: width + glyph.position.x,
                }),
        );
        width += shaped.width;
        segments.push(Segment {
            shaped,
            column: columns.start,
            first_glyph,
            font_size: size,
            shift: group[0].shift,
        });
    }
    (segments, glyphs, width)
}

/// A callout's label, e.g. `Note`, in the line's font made semibold.
fn callout_label(
    kind: CalloutKind,
    line_font: &LineFont,
    text_system: &WindowTextSystem,
) -> Arc<gpui::LineLayout> {
    let label = kind.label();
    let run = TextRun {
        len: label.len(),
        font: Font {
            weight: FontWeight::SEMIBOLD,
            ..line_font.font.clone()
        },
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text_system.layout_line(label, line_font.size, &[run], None)
}

/// Display ranges of consecutive runs for which `include` holds.
fn merged(runs: &[Run], include: impl Fn(&Run) -> bool) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for run in runs.iter().filter(|run| include(run)) {
        match ranges.last_mut() {
            Some(last) if last.end == run.columns.start => last.end = run.columns.end,
            _ => ranges.push(run.columns.clone()),
        }
    }
    ranges
}

/// Measures the cells of the table row `text` styled as `styled`, shaped as [`LineLayout::shape`] shapes it,
/// calling `cell` with each cell's column and the width of its text. Returns where the row starts: the width of
/// what comes before it on the line (ADR 0186).
pub(super) fn measure_cells(
    text: &str,
    styled: &StyledLine,
    base: &BaseStyle,
    text_system: &WindowTextSystem,
    mut cell: impl FnMut(usize, Pixels),
) -> Pixels {
    let runs = runs(styled, &[], &LineFont::new(styled.style, base), base);
    let mut display = String::with_capacity(text.len());
    for span in styled.spans.iter().filter(|span| !span.hidden) {
        display.push_str(&text[span.columns.clone()]);
    }
    let (_, mut glyphs, width) = shape_runs(&display, &runs, text_system);
    let width = expand_tabs(&display, &mut glyphs, width);
    for (column, columns) in table::cells(styled, &display) {
        let x = |column| unwrapped_x(&glyphs, column, width);
        cell(column, x(columns.end) - x(columns.start));
    }
    unwrapped_x(&glyphs, table::row_start(styled), width)
}
