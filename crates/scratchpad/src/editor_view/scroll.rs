//! The scroll position as an anchor line plus a pixel offset (ADR 0030).
//!
//! Nothing here needs the height of the whole document: every operation walks line heights from the
//! anchor and stops after about a viewport's worth, so scrolling a 100k-line document only shapes the
//! lines it passes.

use std::ops::Range;

use gpui::{Pixels, px};
use scratchpad_editor::TextChange;

/// Heights of laid-out logical lines (all of their wrapped rows).
pub(crate) trait LineHeights {
    fn line_count(&self) -> usize;
    fn height(&mut self, line: usize) -> Pixels;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub height: Pixels,
    /// Space above the first line when scrolled to the top.
    pub top_padding: Pixels,
}

/// The viewport's top edge is `offset` below the top of logical line `line`. The offset is negative
/// only on line 0, down to `-top_padding`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollAnchor {
    pub line: usize,
    pub offset: Pixels,
}

impl ScrollAnchor {
    pub fn top(viewport: &Viewport) -> Self {
        Self {
            line: 0,
            offset: -viewport.top_padding,
        }
    }

    pub fn scrolled_by(self, delta: Pixels, lines: &mut impl LineHeights, vp: &Viewport) -> Self {
        Self {
            offset: self.offset + delta,
            ..self
        }
        .clamped(lines, vp)
    }

    /// Keeps the position within the document: no higher than the top padding, and no lower than
    /// the last line's bottom at the middle of the viewport (so the end of a note can be written in
    /// the middle of the screen). A document that fits in the viewport does not scroll at all.
    pub fn clamped(self, lines: &mut impl LineHeights, vp: &Viewport) -> Self {
        let anchor = self.normalized(lines, vp);
        // Content between the viewport's top and the document end, measured up to a viewport.
        let mut below = -anchor.offset;
        let mut line = anchor.line;
        while line < lines.line_count() && below < vp.height {
            below += lines.height(line);
            line += 1;
        }
        if below >= vp.height {
            return anchor;
        }
        let mut total = below + anchor.offset + vp.top_padding;
        let mut line = anchor.line;
        while line > 0 && total <= vp.height {
            line -= 1;
            total += lines.height(line);
        }
        if total <= vp.height {
            return Self::top(vp);
        }
        let min_below = vp.height / 2.;
        if below >= min_below {
            return anchor;
        }
        Self {
            offset: anchor.offset - (min_below - below),
            ..anchor
        }
        .normalized(lines, vp)
    }

    /// Scrolls the least amount that shows `rows` (a y range within `line`) at least `margin` away
    /// from the viewport edges.
    pub fn revealing(
        self,
        line: usize,
        rows: Range<Pixels>,
        margin: Pixels,
        lines: &mut impl LineHeights,
        vp: &Viewport,
    ) -> Self {
        let anchor = self.normalized(lines, vp);
        let height = rows.end - rows.start;
        let margin = margin.min(((vp.height - height) / 2.).max(px(0.)));
        let top = anchor
            .line_top(line, lines, vp.height)
            .map(|top| top + rows.start);
        let target = match top {
            Some(top) if top >= margin && top + height <= vp.height - margin => return anchor,
            // Below the viewport, or too far below to measure: put it at the bottom.
            Some(top) if top >= margin => rows.end + margin - vp.height,
            None if line > anchor.line => rows.end + margin - vp.height,
            // Above the viewport.
            _ => rows.start - margin,
        };
        Self {
            line,
            offset: target,
        }
        .normalized(lines, vp)
        .clamped(lines, vp)
    }

    /// The y of `line`'s top relative to the viewport's top, if it is the anchor line or starts at
    /// most `limit` below the viewport's top.
    pub fn line_top(
        self,
        line: usize,
        lines: &mut impl LineHeights,
        limit: Pixels,
    ) -> Option<Pixels> {
        if line < self.line {
            return None;
        }
        let mut top = -self.offset;
        for above in self.line..line {
            top += lines.height(above);
            if top > limit {
                return None;
            }
        }
        Some(top)
    }

    /// The line at `y` (relative to the viewport's top) and `y` relative to that line's top. Points
    /// above the first or below the last line map to those lines.
    pub fn line_at(self, y: Pixels, lines: &mut impl LineHeights) -> (usize, Pixels) {
        let mut line = self.line;
        let mut top = -self.offset;
        while y < top && line > 0 {
            line -= 1;
            top -= lines.height(line);
        }
        while line + 1 < lines.line_count() {
            let height = lines.height(line);
            if y < top + height {
                break;
            }
            top += height;
            line += 1;
        }
        (line, y - top)
    }

    /// Keeps the same text at the top of the viewport when lines are inserted or removed above it.
    pub fn apply_change(&mut self, change: &TextChange) {
        let first = change.start_point.line;
        let old_last = change.old_end_point.line;
        let new_last = change.new_end_point.line;
        if self.line > old_last {
            self.line = self.line - old_last + new_last;
        } else if self.line > new_last {
            // The anchor line was deleted: continue from the last line that replaced it.
            self.line = new_last.max(first);
            self.offset = px(0.);
        }
    }

    /// Moves the anchor to the line the offset points into.
    fn normalized(self, lines: &mut impl LineHeights, vp: &Viewport) -> Self {
        let last = lines.line_count().saturating_sub(1);
        let mut anchor = Self {
            line: self.line.min(last),
            ..self
        };
        while anchor.offset < px(0.) && anchor.line > 0 {
            anchor.line -= 1;
            anchor.offset += lines.height(anchor.line);
        }
        if anchor.line == 0 {
            anchor.offset = anchor.offset.max(-vp.top_padding);
        }
        while anchor.line < last {
            let height = lines.height(anchor.line);
            if anchor.offset < height {
                break;
            }
            anchor.offset -= height;
            anchor.line += 1;
        }
        anchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scratchpad_editor::{ByteOffset, Point};

    /// Lines of the given heights; counts how many heights were asked for.
    struct Heights(Vec<f32>, usize);

    impl LineHeights for Heights {
        fn line_count(&self) -> usize {
            self.0.len()
        }

        fn height(&mut self, line: usize) -> Pixels {
            self.1 += 1;
            px(self.0[line])
        }
    }

    fn uniform(count: usize) -> Heights {
        Heights(vec![20.; count], 0)
    }

    const VP: Viewport = Viewport {
        height: px(100.),
        top_padding: px(10.),
    };

    fn at(line: usize, offset: f32) -> ScrollAnchor {
        ScrollAnchor {
            line,
            offset: px(offset),
        }
    }

    #[test]
    fn scrolling_moves_the_anchor_across_lines_of_different_heights() {
        let mut lines = Heights(vec![20., 60., 20., 20., 20., 20., 20., 20., 20., 20.], 0);
        let top = ScrollAnchor::top(&VP);
        assert_eq!(top.scrolled_by(px(15.), &mut lines, &VP), at(0, 5.));
        assert_eq!(top.scrolled_by(px(40.), &mut lines, &VP), at(1, 10.));
        assert_eq!(at(1, 10.).scrolled_by(px(65.), &mut lines, &VP), at(2, 15.));
        assert_eq!(at(1, 10.).scrolled_by(px(75.), &mut lines, &VP), at(3, 5.));
        assert_eq!(at(2, 5.).scrolled_by(px(-30.), &mut lines, &VP), at(1, 35.));
        assert_eq!(at(2, 5.).scrolled_by(px(-500.), &mut lines, &VP), top);
    }

    #[test]
    fn the_last_line_can_scroll_up_to_the_middle_of_the_viewport() {
        let mut lines = uniform(10); // 200 px of text
        // The last line's bottom at y = 50: 150 px of the document above the viewport.
        let max = ScrollAnchor::top(&VP).scrolled_by(px(10_000.), &mut lines, &VP);
        assert_eq!(max, at(7, 10.));
        assert_eq!(max.scrolled_by(px(1.), &mut lines, &VP), max);
        assert_eq!(max.scrolled_by(px(-1.), &mut lines, &VP), at(7, 9.));
    }

    #[test]
    fn a_document_that_fits_does_not_scroll() {
        let mut lines = uniform(4); // 10 px padding + 80 px of text in a 100 px viewport
        let top = ScrollAnchor::top(&VP);
        assert_eq!(top.scrolled_by(px(30.), &mut lines, &VP), top);
        let mut lines = uniform(5);
        assert_ne!(top.scrolled_by(px(30.), &mut lines, &VP), top);
    }

    #[test]
    fn scrolling_far_into_a_long_document_only_measures_lines_near_the_viewport() {
        let mut lines = uniform(100_000);
        let anchor = at(50_000, 0.).scrolled_by(px(35.), &mut lines, &VP);
        assert_eq!(anchor, at(50_001, 15.));
        assert!(lines.1 < 20, "measured {} lines", lines.1);
    }

    #[test]
    fn revealing_scrolls_only_when_the_rows_are_outside_the_margins() {
        let mut lines = uniform(100);
        let anchor = at(10, 0.);
        let margin = px(20.);
        let reveal = |line, rows: Range<f32>, lines: &mut Heights| {
            anchor.revealing(line, px(rows.start)..px(rows.end), margin, lines, &VP)
        };
        assert_eq!(reveal(12, 0.0..20.0, &mut lines), anchor, "already visible");
        // Line 14 is at y 80..100, inside the bottom margin: its bottom moves to y = 80.
        assert_eq!(reveal(14, 0.0..20.0, &mut lines), at(11, 0.));
        // Line 10 is at the very top: its top moves down to y = 20.
        assert_eq!(reveal(10, 0.0..20.0, &mut lines), at(9, 0.));
        // Far away in either direction, without measuring the lines in between.
        lines.1 = 0;
        assert_eq!(reveal(90, 0.0..20.0, &mut lines), at(87, 0.));
        assert_eq!(reveal(2, 10.0..30.0, &mut lines), at(1, 10.));
        assert!(lines.1 < 40, "measured {} lines", lines.1);
    }

    #[test]
    fn hit_testing_finds_the_line_under_a_y() {
        let mut lines = Heights(vec![20., 60., 20.], 0);
        let anchor = at(1, 10.);
        assert_eq!(anchor.line_at(px(0.), &mut lines), (1, px(10.)));
        assert_eq!(anchor.line_at(px(55.), &mut lines), (2, px(5.)));
        assert_eq!(anchor.line_at(px(-15.), &mut lines), (0, px(15.)));
        assert_eq!(anchor.line_at(px(500.), &mut lines), (2, px(450.)));
    }

    fn change(start: usize, old_end: usize, new_end: usize) -> TextChange {
        let point = |line| Point::new(line, 0);
        TextChange {
            start: ByteOffset(0),
            old_end: ByteOffset(0),
            new_end: ByteOffset(0),
            start_point: point(start),
            old_end_point: point(old_end),
            new_end_point: point(new_end),
        }
    }

    #[test]
    fn edits_above_the_anchor_keep_the_same_text_on_screen() {
        let mut anchor = at(10, 5.);
        anchor.apply_change(&change(2, 2, 5));
        assert_eq!(anchor, at(13, 5.), "three lines inserted above");
        anchor.apply_change(&change(1, 4, 1));
        assert_eq!(anchor, at(10, 5.), "three lines deleted above");
        anchor.apply_change(&change(10, 10, 10));
        assert_eq!(anchor, at(10, 5.), "an edit inside the anchor line");
        anchor.apply_change(&change(8, 12, 8));
        assert_eq!(anchor, at(8, 0.), "the anchor line was deleted");
    }
}
