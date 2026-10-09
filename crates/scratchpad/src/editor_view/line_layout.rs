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
use scratchpad_editor::markdown::{MarkdownState, MarkerKind, SpanStyle, StyledLine};
use scratchpad_editor::{Bias, Buffer, Selection};

use super::geometry::{Glyph, Indents, LineGeometry, expand_tabs, set_span_width, unwrapped_x};
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
    pub wrap_width: Pixels,
    /// How far a heading's `#`s may hang into the margin left of the text column.
    pub hang_room: Pixels,
}

impl BaseStyle {
    /// A distance given for the normal text size, scaled with the zoom.
    fn scaled(&self, pixels: Pixels) -> Pixels {
        (pixels * (self.font_size / typography::BODY_FONT_SIZE)).round()
    }
}

/// Everything besides its text that a line's layout depends on. A cached layout is reused while its
/// key stays the same, so moving the cursor only re-shapes lines whose markers it reveals or hides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct LineKey {
    pub styled: StyledLine,
    /// Draws list bullets and task boxes as shapes rather than as the characters typed.
    pub widgets: bool,
}

impl LineKey {
    /// How `line` looks with `selection`. In live preview (`Some`) markers away from the selection
    /// are hidden, and bullets and task boxes are drawn as shapes unless the selection reaches inside
    /// them; in source mode (`None`) everything shows as typed.
    pub fn new(
        markdown: &mut MarkdownState,
        buffer: &Buffer,
        line: usize,
        selection: Option<Selection>,
    ) -> Self {
        let styled = markdown
            .styled_lines(buffer, line..line + 1, selection)
            .pop()
            .unwrap_or_default();
        let line_start = buffer.line_start(line).0;
        let widgets = selection.is_some_and(|selection| {
            !widgets(&styled).iter().any(|widget| {
                selection.start().0 < line_start + widget.columns.end
                    && selection.end().0 > line_start + widget.columns.start
            })
        });
        Self { styled, widgets }
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
}

/// A list marker drawn as a shape in live preview.
struct Widget {
    kind: WidgetKind,
    /// Buffer columns at first, display columns once the line is laid out.
    columns: Range<usize>,
}

/// The line's bullets and task boxes. A bullet followed by a task box is drawn as just the box.
fn widgets(styled: &StyledLine) -> Vec<Widget> {
    let mut widgets: Vec<Widget> = Vec::new();
    for span in &styled.spans {
        let kind = match span.marker {
            Some(MarkerKind::ListBullet) => WidgetKind::Bullet,
            Some(MarkerKind::TaskBox { checked }) => WidgetKind::TaskBox {
                checked,
                column: span.columns.start,
            },
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
            Shape::Rect { bounds, .. } | Shape::TaskBox { bounds, .. } => *bounds,
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
        let line_font = LineFont::new(block, base);
        let mut widgets = if key.widgets {
            widgets(&key.styled)
        } else {
            Vec::new()
        };
        let runs = runs(key, &widgets, &line_font, base);
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

        let mut indents = Indents::default();
        let mut wrap_width = base.wrap_width;
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
                merged(runs, |run| run.style.link && !run.marker),
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
        if key.styled.style.quote {
            let bounds = Bounds::new(point(px(0.), px(0.)), size(QUOTE_BAR_WIDTH, self.height()));
            self.below.push(Shape::Rect {
                bounds,
                color: theme.border,
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
    /// text. Only the glyphs of those rows are visited, so a frame (even a caret blink) costs the
    /// same on a megabyte-long line as on a short one.
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window) {
        let clip = window.content_mask().bounds;
        let first_row = self.row_at(clip.top() - origin.y);
        let last_row = self.row_at(clip.bottom() - origin.y);
        let visible =
            self.geometry.row_glyphs(first_row).start..self.geometry.row_glyphs(last_row).end;
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
                    // Bullets and task boxes are transparent: shapes are drawn in their place.
                    if color.a == 0. {
                        continue;
                    }
                    while row < last_row && glyph_ix >= self.geometry.row_glyphs(row + 1).start {
                        row += 1;
                    }
                    let origin = point(
                        origin.x + self.geometry.glyph_x(row, glyph_ix),
                        origin.y + self.row_top(row) + baseline,
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
fn runs(key: &LineKey, widgets: &[Widget], line_font: &LineFont, base: &BaseStyle) -> Vec<Run> {
    let theme = &base.theme;
    let mut runs = Vec::with_capacity(key.styled.spans.len());
    let mut display_len = 0;
    let mut after_checked_box = false;
    for span in key.styled.spans.iter().filter(|span| !span.hidden) {
        let columns = display_len..display_len + span.columns.len();
        display_len = columns.end;
        let style = span.style;
        let mono = style.code || style.code_block;
        let mut font = if mono {
            font(base.mono_family.clone())
        } else {
            line_font.font.clone()
        };
        if style.bold {
            font.weight = FontWeight::BOLD;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        let size = if style.code && !style.code_block {
            line_font.size * CODE_SCALE
        } else {
            line_font.size
        };
        let in_widget = widgets
            .iter()
            .any(|widget| widget.columns.contains(&span.columns.start));
        // Markers, quotes and done tasks recede; the text of the note stays in front.
        let color = if in_widget {
            transparent_black()
        } else if span.marker.is_some()
            || after_checked_box
            || style.strikethrough
            || style.quote
            || style.heading == 6
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
    // only the font changes between them, which would lose bold and italic.
    for group in runs.chunk_by(|a, b| a.size == b.size && a.font == b.font) {
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
        });
    }
    (segments, glyphs, width)
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
