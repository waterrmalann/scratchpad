# 0031. Soft wrapping and navigation by visual rows
Date: 2026-10-08
Status: Accepted; Tab on list items amended by ADR 0126, the text column by ADR 0130

## Context
Word wrap is on by default (PLAN §26) and Scratchpad is for prose, not code (PLAN §27). GPUI's own wrapper
may start a row with a period or comma, and its `WrappedLine` paints every glyph of a line. Cursor keys
have to decide between logical lines and the rows the user sees.

## Decision
- Text sits in a centred column at most 680 px wide (about 85 characters of 15 px Segoe UI, a comfortable
  measure for prose) with at least 32 px on each side.
- The view shapes each line with `WindowTextSystem::layout_line` and wraps it itself: rows break after
  whitespace, after a hyphen inside a word, and between CJK characters, never before closing or after
  opening punctuation; words longer than a row break at grapheme boundaries. Trailing spaces hang past the
  column instead of opening a new row. The view paints glyphs itself, which also lets it skip rows outside
  the viewport and give tabs a width.
- Tab characters are shaped as spaces (fonts give them no width) and widened to stops every four space
  widths. The Tab key inserts four spaces: they render identically everywhere and Markdown treats them as
  indentation.
- A column at a soft wrap belongs to the row it starts. So that every column has exactly one row without
  tracking cursor affinity, a click past the end of a wrapped row, End, and Up/Down onto such a row land
  before its last glyph, normally the space the line wrapped at.
- Up/Down move by visual rows and keep the pixel x as the engine's `Goal::Horizontal`. Home/End go to the
  start/end of the visual row, as in Notepad with word wrap; Ctrl+Home/End go to the document ends.
  PageUp/PageDown scroll by the viewport height less one line and move the cursor the same distance, so it
  keeps its place on screen. Moving past the first or last row goes to the document start or end, as the
  engine's logical motions do.

## Consequences
- Wrapping is simpler than full Unicode line breaking (UAX #14): no hyphenation and no special rules for
  Thai or similar scripts. Bidirectional text is laid out but not navigated visually.
- Shift+End on a wrapped row selects up to, not including, the space at the wrap.
