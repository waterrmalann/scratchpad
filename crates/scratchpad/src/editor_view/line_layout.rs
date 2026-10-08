//! One logical line shaped by GPUI's text system, soft-wrapped, and painted row by row.

use std::ops::Deref;
use std::sync::Arc;

use gpui::{Font, Hsla, Pixels, Point, TextRun, Window, WindowTextSystem, point};

use super::geometry::{Glyph, LineGeometry, expand_tabs};

/// Text settings shared by every line: the body font from the theme and the text column width.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BaseStyle {
    pub font: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub color: Hsla,
    pub wrap_width: Pixels,
}

/// How one logical line is drawn.
pub(crate) struct LineStyle {
    /// Decides the line's ascent and descent, so the caret height and baseline.
    pub font: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    /// Cover the line's text exactly. Only `font` and `color` are honoured so far.
    pub runs: Vec<TextRun>,
}

/// The single place that decides fonts, sizes and colours per line. Markdown rendering will return
/// several runs here (and a larger size and line height for headings); nothing else in the element
/// assumes plain text or uniform line heights.
///
/// Hidden syntax markers fit the same design: [`LineLayout::shape`] would shape the line without
/// them and map each glyph's index back to its column in the buffer line when building
/// [`Glyph`]s. Wrapping, hit testing, carets and selections only ever see buffer columns, so a
/// hidden range simply has no glyphs.
pub(crate) fn line_style(text: &str, base: &BaseStyle) -> LineStyle {
    LineStyle {
        font: base.font.clone(),
        font_size: base.font_size,
        line_height: base.line_height,
        runs: vec![TextRun {
            len: text.len(),
            font: base.font.clone(),
            color: base.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
    }
}

/// A shaped and wrapped line. Derefs to its [`LineGeometry`] for hit testing and positions.
pub(crate) struct LineLayout {
    shaped: Arc<gpui::LineLayout>,
    geometry: LineGeometry,
    /// `(end column, colour)` of each run.
    colors: Vec<(usize, Hsla)>,
    font_size: Pixels,
    line_height: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

impl Deref for LineLayout {
    type Target = LineGeometry;

    fn deref(&self) -> &LineGeometry {
        &self.geometry
    }
}

impl LineLayout {
    pub fn shape(text: &str, base: &BaseStyle, text_system: &WindowTextSystem) -> Self {
        let style = line_style(text, base);
        // Tabs are shaped as spaces (same byte length, so columns are unchanged) and then widened to
        // tab stops; fonts give tab characters no width at all.
        let shaped_text = text.replace('\t', " ");
        let shaped = text_system.layout_line(&shaped_text, style.font_size, &style.runs, None);
        let mut glyphs: Vec<Glyph> = shaped
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| Glyph {
                column: glyph.index,
                x: glyph.position.x,
            })
            .collect();
        let width = expand_tabs(text, &mut glyphs, shaped.width);
        let geometry = LineGeometry::new(text, glyphs, width, base.wrap_width);
        let mut end = 0;
        let colors = style
            .runs
            .iter()
            .map(|run| {
                end += run.len;
                (end, run.color)
            })
            .collect();
        // Metrics come from the font rather than the shaped glyphs so that empty lines get the same
        // caret and baseline as lines with text. Font metrics report the descent as negative.
        let font_id = text_system.resolve_font(&style.font);
        Self {
            ascent: text_system.ascent(font_id, style.font_size),
            descent: -text_system.descent(font_id, style.font_size),
            shaped,
            geometry,
            colors,
            font_size: style.font_size,
            line_height: style.line_height,
        }
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    pub fn height(&self) -> Pixels {
        self.line_height * self.row_count() as f32
    }

    /// y of `row`'s top below the line's top. Everything vertical inside a line goes through this
    /// and [`LineLayout::row_at`], so per-line spacing (e.g. above Markdown headings) only has to
    /// change these and [`LineLayout::height`].
    pub fn row_top(&self, row: usize) -> Pixels {
        self.line_height * row as f32
    }

    /// The row at `y` below the line's top; above or below the line, its first or last row.
    pub fn row_at(&self, y: Pixels) -> usize {
        ((y / self.line_height).floor().max(0.) as usize).min(self.row_count() - 1)
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

    /// Paints the rows of the line that intersect the content mask, the first row's top-left at
    /// `origin`. Only the glyphs of those rows are visited, so a frame (even a caret blink) costs
    /// the same on a megabyte-long line as on a short one.
    pub fn paint(&self, origin: Point<Pixels>, window: &mut Window) {
        let clip = window.content_mask().bounds;
        let first_row = self.row_at(clip.top() - origin.y);
        let last_row = self.row_at(clip.bottom() - origin.y);
        let visible = self.row_glyphs(first_row).start..self.row_glyphs(last_row).end;
        let baseline = self.baseline();
        let mut row = first_row;
        // Geometry numbers glyphs across all shaped runs; `run_start` is this run's first.
        let mut run_start = 0;
        for run in &self.shaped.runs {
            let run_glyphs = run_start..run_start + run.glyphs.len();
            run_start = run_glyphs.end;
            for glyph_ix in visible.start.max(run_glyphs.start)..visible.end.min(run_glyphs.end) {
                let glyph = &run.glyphs[glyph_ix - run_glyphs.start];
                while row < last_row && glyph_ix >= self.row_glyphs(row + 1).start {
                    row += 1;
                }
                let origin = point(
                    origin.x + self.glyph_x(row, glyph_ix),
                    origin.y + self.row_top(row) + baseline,
                );
                // A glyph that fails to rasterise is skipped rather than failing the frame.
                let _ = if glyph.is_emoji {
                    window.paint_emoji(origin, run.font_id, glyph.id, self.font_size)
                } else {
                    let color = self.color_at(glyph.index);
                    window.paint_glyph(origin, run.font_id, glyph.id, self.font_size, color)
                };
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
