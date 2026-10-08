//! Soft wrapping and the geometry of one shaped logical line, independent of GPUI's glyph types.
//!
//! Every position here is a **byte column** within the line (the engine's `Point::column`); x values are
//! pixels from the left edge of the row they are on.

use std::ops::Range;

use gpui::{Pixels, px};
use unicode_segmentation::UnicodeSegmentation;

/// Where a shaped glyph starts: the byte column of the text it renders and its x from the line start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub column: usize,
    pub x: Pixels,
}

/// How far rows sit from the left edge of the text column: the first row, and the rows a line wraps
/// into. Negative for text that hangs into the margin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Indents {
    pub first: Pixels,
    pub rest: Pixels,
}

/// One visual row of a wrapped line.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    /// Column of the first byte on the row.
    start: usize,
    /// x of the row's first glyph in the unwrapped line.
    start_x: Pixels,
    /// Index into `glyphs` of the row's first glyph.
    first_glyph: usize,
}

/// A logical line broken into rows no wider than the wrap width.
///
/// A column at a soft wrap belongs to the row it starts, so the end of a non-final row is not a
/// cursor position of that row: clicking past it or pressing End lands before its last glyph
/// (usually the space the line wrapped at). This keeps every column on exactly one row without
/// tracking cursor affinity (ADR 0031).
#[derive(Debug)]
pub(crate) struct LineGeometry {
    glyphs: Vec<Glyph>,
    rows: Vec<Row>,
    /// Width of the whole line unwrapped.
    width: Pixels,
    wrap_width: Pixels,
    indents: Indents,
    len: usize,
}

impl LineGeometry {
    /// Wraps `text` (one line, no line break) whose glyphs are `glyphs`, in column order, into rows
    /// that start at their indent and end at `wrap_width`. Rows break after whitespace, around CJK
    /// ideographs, and after a hyphen; a word longer than a row is broken at a grapheme boundary.
    /// Trailing spaces may hang past the wrap width instead of starting a row of their own.
    pub fn new(
        text: &str,
        glyphs: Vec<Glyph>,
        width: Pixels,
        wrap_width: Pixels,
        indents: Indents,
    ) -> Self {
        let rows = wrap(text, &glyphs, width, wrap_width, indents);
        Self {
            glyphs,
            rows,
            width,
            wrap_width,
            indents,
            len: text.len(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The row a cursor at `column` is drawn on.
    pub fn row_of(&self, column: usize) -> usize {
        self.rows
            .partition_point(|row| row.start <= column)
            .saturating_sub(1)
    }

    pub fn row_start(&self, row: usize) -> usize {
        self.rows[row.min(self.rows.len() - 1)].start
    }

    /// Where End moves on `row`: the line end on the last row, otherwise before the row's last glyph.
    pub fn row_end(&self, row: usize) -> usize {
        let row = row.min(self.rows.len() - 1);
        match self.rows.get(row + 1) {
            Some(next) => self.glyphs[next.first_glyph - 1].column,
            None => self.len,
        }
    }

    /// x of a cursor at `column`, capped at the wrap width so hanging spaces never put the cursor
    /// outside the text column.
    pub fn x_for(&self, column: usize) -> Pixels {
        let row = self.row_of(column);
        let x = unwrapped_x(&self.glyphs, column, self.width) - self.rows[row].start_x;
        self.indent(row) + x.min(self.room(row))
    }

    /// The column closest to `x` on `row`. Never returns a non-final row's end (see the type docs).
    pub fn column_at(&self, row: usize, x: Pixels) -> usize {
        let row = row.min(self.rows.len() - 1);
        let glyphs = self.row_glyphs(row);
        let x = x - self.indent(row) + self.rows[row].start_x;
        for i in glyphs.clone() {
            let left = self.glyphs[i].x;
            let right = self.glyphs.get(i + 1).map_or(self.width, |next| next.x);
            if x < (left + right) / 2. {
                return self.glyphs[i].column;
            }
        }
        if row + 1 < self.rows.len() {
            self.glyphs[glyphs.end - 1].column
        } else {
            self.len
        }
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
        let first = self.row_of(columns.start);
        let last = self.row_of(columns.end);
        (first..=last)
            .filter_map(|row| {
                let is_last_row = row + 1 == self.rows.len();
                let start = if row == first {
                    self.x_for(columns.start)
                } else {
                    self.indent(row)
                };
                let mut end = if row == last {
                    self.x_for(columns.end)
                } else {
                    self.indent(row) + self.row_width(row)
                };
                if newline && is_last_row {
                    end += newline_width;
                }
                (end > start).then_some((row, start, end))
            })
            .collect()
    }

    /// Glyph indices on `row`.
    pub fn row_glyphs(&self, row: usize) -> Range<usize> {
        let start = self.rows[row].first_glyph;
        let end = self
            .rows
            .get(row + 1)
            .map_or(self.glyphs.len(), |next| next.first_glyph);
        start..end
    }

    /// x of `glyph` on `row`.
    pub fn glyph_x(&self, row: usize, glyph: usize) -> Pixels {
        self.indent(row) + self.glyphs[glyph].x - self.rows[row].start_x
    }

    /// The column of the glyph drawn at `x` on `row`, if any: what the pointer is over, as opposed
    /// to the nearest cursor position.
    pub fn glyph_at(&self, row: usize, x: Pixels) -> Option<usize> {
        let row = row.min(self.rows.len() - 1);
        let x = x - self.indent(row) + self.rows[row].start_x;
        self.row_glyphs(row)
            .find(|&i| {
                let right = self.glyphs.get(i + 1).map_or(self.width, |next| next.x);
                self.glyphs[i].x <= x && x < right
            })
            .map(|i| self.glyphs[i].column)
    }

    fn indent(&self, row: usize) -> Pixels {
        if row == 0 {
            self.indents.first
        } else {
            self.indents.rest
        }
    }

    /// Width available to `row` between its indent and the wrap width.
    fn room(&self, row: usize) -> Pixels {
        self.wrap_width - self.indent(row)
    }

    fn row_width(&self, row: usize) -> Pixels {
        let end_x = self
            .rows
            .get(row + 1)
            .map_or(self.width, |next| next.start_x);
        (end_x - self.rows[row].start_x).min(self.room(row))
    }
}

/// x in the unwrapped line of the first glyph at or after `column`, or the line's `width` past the
/// last glyph.
pub(crate) fn unwrapped_x(glyphs: &[Glyph], column: usize, width: Pixels) -> Pixels {
    let i = glyphs.partition_point(|glyph| glyph.column < column);
    glyphs.get(i).map_or(width, |glyph| glyph.x)
}

/// Makes the glyphs of `columns` take `span_width` by moving every later glyph, e.g. to make room
/// for a shape drawn in their place. Returns the new line width.
pub(crate) fn set_span_width(
    glyphs: &mut [Glyph],
    columns: Range<usize>,
    span_width: Pixels,
    width: Pixels,
) -> Pixels {
    let start = unwrapped_x(glyphs, columns.start, width);
    let after = glyphs.partition_point(|glyph| glyph.column < columns.end);
    let shift = span_width - (unwrapped_x(glyphs, columns.end, width) - start);
    for glyph in &mut glyphs[after..] {
        glyph.x += shift;
    }
    width + shift
}

/// Tab stops are this many space widths apart, matching the four spaces the Tab key inserts.
const TAB_STOP_SPACES: f32 = 4.;

/// Widens the glyphs of tab characters, shaped as spaces (fonts give tabs no width), so that each
/// reaches the next tab stop. Returns the new line width.
pub(crate) fn expand_tabs(text: &str, glyphs: &mut [Glyph], width: Pixels) -> Pixels {
    let mut shift = px(0.);
    for i in 0..glyphs.len() {
        let next_x = glyphs.get(i + 1).map_or(width, |next| next.x);
        let advance = next_x - glyphs[i].x;
        glyphs[i].x += shift;
        if char_at(text, glyphs[i].column) == '\t' && advance > px(0.) {
            let stop = advance * TAB_STOP_SPACES;
            let tab_end = stop * ((glyphs[i].x / stop).floor() + 1.);
            shift += tab_end - glyphs[i].x - advance;
        }
    }
    width + shift
}

fn wrap(
    text: &str,
    glyphs: &[Glyph],
    width: Pixels,
    wrap_width: Pixels,
    indents: Indents,
) -> Vec<Row> {
    // Collected once per line: asking `GraphemeCursor` about each glyph separately rescans runs of
    // regional indicators, which made a line of flags quadratic.
    let graphemes: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    // Shapers may report glyphs inside a cluster; a column that does not start a grapheme is never
    // a break.
    let starts_grapheme = |i: usize| {
        let column = glyphs[i].column;
        column != glyphs[i - 1].column && graphemes.binary_search(&column).is_ok()
    };
    let mut rows = vec![Row {
        start: 0,
        start_x: px(0.),
        first_glyph: 0,
    }];
    // The last glyphs in the current row before which it may break by the wrapping rules, and at
    // all (for a word wider than the row). Tracking them as we go keeps wrapping linear even when a
    // row has no break opportunity at all.
    let mut candidate = None;
    let mut forced = None;
    for i in 0..glyphs.len() {
        let row = *rows.last().expect("rows start non-empty");
        if i > row.first_glyph && starts_grapheme(i) {
            forced = Some(i);
            if can_break_before(text, glyphs[i].column) {
                candidate = Some(i);
            }
        }
        let right = glyphs.get(i + 1).map_or(width, |next| next.x);
        let indent = if rows.len() == 1 {
            indents.first
        } else {
            indents.rest
        };
        let overflows = right - row.start_x > wrap_width - indent
            && i > row.first_glyph
            && !char_at(text, glyphs[i].column).is_whitespace();
        if !overflows {
            continue;
        }
        let Some(at) = candidate.take().or(forced) else {
            continue;
        };
        forced = forced.filter(|&forced| forced > at);
        rows.push(Row {
            start: glyphs[at].column,
            start_x: glyphs[at].x,
            first_glyph: at,
        });
    }
    rows
}

/// Whether a row may start at `column`, a grapheme boundary that is not the start of the line.
fn can_break_before(text: &str, column: usize) -> bool {
    let prev = text[..column].chars().next_back().unwrap_or(' ');
    let next = char_at(text, column);
    if next.is_whitespace() {
        return false;
    }
    prev.is_whitespace()
        || (prev == '-' && next.is_alphanumeric())
        || ((is_cjk(prev) || is_cjk(next)) && !closes(next) && !opens(prev))
}

fn char_at(text: &str, column: usize) -> char {
    text.get(column..)
        .and_then(|rest| rest.chars().next())
        .unwrap_or(' ')
}

/// Scripts written without spaces between words, where a row may break between any two characters.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{1100}'..='\u{11FF}'
        | '\u{2E80}'..='\u{303F}'
        | '\u{3040}'..='\u{33FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{AC00}'..='\u{D7AF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}'
        | '\u{20000}'..='\u{3FFFF}')
}

/// Punctuation that must not start a row.
fn closes(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
            | '、'
            | '。'
            | '，'
            | '．'
            | '！'
            | '？'
            | '：'
            | '；'
            | '）'
            | '」'
            | '』'
            | '》'
            | '〉'
            | '】'
            | 'ー'
    )
}

/// Punctuation that must not end a row.
fn opens(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '（' | '「' | '『' | '《' | '〈' | '【')
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADVANCE: f32 = 10.;

    /// Every char 10 px wide, like a monospace font.
    fn layout(text: &str, wrap_width: f32) -> LineGeometry {
        indented(text, wrap_width, Indents::default())
    }

    fn glyphs(text: &str) -> Vec<Glyph> {
        text.char_indices()
            .enumerate()
            .map(|(i, (column, _))| Glyph {
                column,
                x: px(i as f32 * ADVANCE),
            })
            .collect()
    }

    fn indented(text: &str, wrap_width: f32, indents: Indents) -> LineGeometry {
        let glyphs = glyphs(text);
        let width = px(text.chars().count() as f32 * ADVANCE);
        LineGeometry::new(text, glyphs, width, px(wrap_width), indents)
    }

    fn rows(text: &str, wrap_width: f32) -> Vec<&str> {
        rows_of(&layout(text, wrap_width), text)
    }

    /// The text of each row of `line`, laid out from `text`.
    fn rows_of<'a>(line: &LineGeometry, text: &'a str) -> Vec<&'a str> {
        (0..line.row_count())
            .map(|row| {
                let end = line.rows.get(row + 1).map_or(text.len(), |next| next.start);
                &text[line.row_start(row)..end]
            })
            .collect()
    }

    #[test]
    fn wraps_after_spaces_and_lets_trailing_spaces_hang() {
        assert_eq!(rows("hello world foo", 80.), ["hello ", "world ", "foo"]);
        // "hello world" fits in 110 px; the space after it hangs instead of starting a row.
        assert_eq!(rows("hello world foo", 110.), ["hello world ", "foo"]);
        assert_eq!(rows("short", 80.), ["short"]);
    }

    #[test]
    fn never_starts_a_row_with_closing_punctuation() {
        // The period overflows; the whole word moves down with it.
        assert_eq!(rows("ab cdef.", 70.), ["ab ", "cdef."]);
    }

    #[test]
    fn breaks_long_words_at_grapheme_boundaries() {
        assert_eq!(rows("abcdefghij", 40.), ["abcd", "efgh", "ij"]);
        // "e" + combining acute accent is one grapheme of two chars; it is never split.
        assert_eq!(rows("abce\u{301}fg", 40.), ["abc", "e\u{301}fg"]);
    }

    #[test]
    fn wraps_megabyte_lines_without_splitting_graphemes() {
        // Each flag is two regional indicators (8 bytes); finding the boundaries between them used
        // to take quadratic time, so this test would not finish.
        let flags = "\u{1F1FA}\u{1F1F8}".repeat(131_072);
        let line = layout(&flags, 400.);
        assert_eq!(
            line.row_count(),
            131_072_usize.div_ceil(20),
            "20 flags of 20 px per row"
        );
        assert!((0..line.row_count()).all(|row| line.row_start(row).is_multiple_of(8)));

        // One grapheme of half a million combining marks cannot be broken at all.
        let zalgo = format!("a{}", "\u{301}".repeat(500_000));
        assert_eq!(layout(&zalgo, 400.).row_count(), 1);
    }

    #[test]
    fn breaks_between_cjk_characters_but_not_before_their_punctuation() {
        assert_eq!(rows("日本語の文章", 30.), ["日本語", "の文章"]);
        assert_eq!(rows("日本語。です", 30.), ["日本", "語。で", "す"]);
    }

    #[test]
    fn breaks_after_hyphens_inside_words() {
        assert_eq!(rows("well-known", 60.), ["well-", "known"]);
    }

    #[test]
    fn a_wrap_column_belongs_to_the_next_row() {
        let line = layout("hello world", 80.);
        assert_eq!(line.row_of(5), 0);
        assert_eq!(line.row_of(6), 1, "the column after the wrapped space");
        assert_eq!(line.x_for(6), px(0.));
        assert_eq!(line.x_for(11), px(50.));
        assert_eq!(line.row_end(0), 5, "End stops before the space");
        assert_eq!(line.row_end(1), 11);
    }

    #[test]
    fn hit_testing_picks_the_nearest_column_in_the_row() {
        let line = layout("hello world", 80.);
        assert_eq!(line.column_at(0, px(-5.)), 0);
        assert_eq!(line.column_at(0, px(14.)), 1);
        assert_eq!(line.column_at(0, px(16.)), 2);
        assert_eq!(
            line.column_at(0, px(500.)),
            5,
            "past the end of a wrapped row"
        );
        assert_eq!(
            line.column_at(1, px(500.)),
            11,
            "past the end of the last row"
        );
        assert_eq!(line.column_at(1, px(21.)), 8);
    }

    #[test]
    fn spans_cover_partial_and_wrapped_rows() {
        let line = layout("hello world", 80.);
        assert_eq!(line.spans(1..3, false, px(4.)), [(0, px(10.), px(30.))]);
        assert_eq!(
            line.spans(3..8, false, px(4.)),
            [(0, px(30.), px(60.)), (1, px(0.), px(20.))]
        );
        assert_eq!(
            line.spans(8..11, true, px(4.)),
            [(1, px(20.), px(54.))],
            "a selected line break adds a sliver"
        );
        let empty = layout("", 80.);
        assert_eq!(empty.spans(0..0, true, px(4.)), [(0, px(0.), px(4.))]);
        assert!(empty.spans(0..0, false, px(4.)).is_empty());
    }

    #[test]
    fn tabs_advance_to_the_next_stop_of_four_spaces() {
        let text = "a\tb\t\tc";
        let line = layout(text, 1000.);
        let mut glyphs = line.glyphs.clone();
        let width = expand_tabs(text, &mut glyphs, line.width);
        let xs: Vec<f32> = glyphs.iter().map(|glyph| glyph.x.into()).collect();
        // a at 0, tab to 40, b at 40, tab to 80, tab to 120, c at 120.
        assert_eq!(xs, [0., 10., 40., 50., 80., 120.]);
        assert_eq!(width, px(130.));
    }

    #[test]
    fn hanging_spaces_never_reach_past_the_wrap_width() {
        let line = layout("abc      ", 50.);
        assert_eq!(line.row_count(), 1);
        assert_eq!(line.x_for(9), px(50.));
    }

    #[test]
    fn indents_shift_rows_and_narrow_the_rows_after_the_first() {
        // A list item: "- " hangs before the first row's text; wrapped rows line up with it.
        let indents = Indents {
            first: px(0.),
            rest: px(20.),
        };
        let line = indented("- aaa bbb ccc", 80., indents);
        assert_eq!(rows_of(&line, "- aaa bbb ccc"), ["- aaa ", "bbb ", "ccc"]);
        assert_eq!(line.x_for(6), px(20.), "wrapped rows start at the indent");
        assert_eq!(line.x_for(8), px(40.));
        assert_eq!(line.column_at(1, px(21.)), 6);
        assert_eq!(line.column_at(1, px(0.)), 6);
        assert_eq!(
            line.spans(4..8, false, px(0.)),
            [(0, px(40.), px(60.)), (1, px(20.), px(40.))]
        );
        assert_eq!(line.glyph_x(1, 7), px(30.));

        // A heading marker hanging in the margin: the first row has its width in extra room.
        let hanging = Indents {
            first: px(-20.),
            rest: px(0.),
        };
        let line = indented("# abcdefgh", 80., hanging);
        assert_eq!(
            line.row_count(),
            1,
            "10 chars fit in 80 px plus the 20 px margin"
        );
        assert_eq!(line.x_for(2), px(0.));
        assert_eq!(line.x_for(0), px(-20.));
    }

    #[test]
    fn span_widths_move_the_glyphs_after_the_span() {
        let mut glyphs = glyphs("- ab");
        let width = set_span_width(&mut glyphs, 0..2, px(35.), px(40.));
        let xs: Vec<f32> = glyphs.iter().map(|glyph| glyph.x.into()).collect();
        assert_eq!(xs, [0., 10., 35., 45.]);
        assert_eq!(width, px(55.));
        let width = set_span_width(&mut glyphs, 2..4, px(5.), width);
        assert_eq!(width, px(40.), "the last span ends the line");
    }

    #[test]
    fn the_glyph_under_a_point_is_the_one_drawn_there() {
        let line = layout("hello world", 80.);
        assert_eq!(line.glyph_at(0, px(14.)), Some(1));
        assert_eq!(
            line.glyph_at(0, px(16.)),
            Some(1),
            "not the nearest boundary"
        );
        assert_eq!(line.glyph_at(1, px(5.)), Some(6));
        assert_eq!(line.glyph_at(1, px(55.)), None, "past the end");
        assert_eq!(line.glyph_at(0, px(-1.)), None);
    }
}
