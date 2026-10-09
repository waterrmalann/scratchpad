# 0132. Turning word wrap off
Date: 2026-10-09
Status: Accepted; amends ADR 0030 and 0031

## Context
Notepad's View > Word wrap turns wrapping off: each line is one row and the window scrolls
sideways. Scratchpad wraps (ADR 0031) and never knows the whole document's geometry (ADR 0030).

## Decision
- `view::ToggleWordWrap` (no key, as in Notepad) switches it; `Config::word_wrap` remembers it,
  on by default. As with zoom, the cursor's row keeps its place on screen.
- Unwrapped, a line is laid out with an unbounded wrap width: one row, so visual-row motions
  (Up/Down, Home/End) become motions by line without any special case.
- The view keeps a horizontal offset. Lines are drawn, hit-tested, highlighted and reported to
  the IME shifted by it. Shift+wheel (which Windows sends as a horizontal delta), a tilting wheel
  and a sideways touchpad swipe scroll it, from the line start up to where the end of the longest
  line *on screen* reaches the right edge: the widest line of the note is never known. Moving or
  typing scrolls it to keep the caret 3 em inside the column, and so does a click that puts the
  caret out of view (past the end of a line scrolled out to the left); a click in view never moves
  the text. Dragging a selection past the text's left or right edge scrolls it too (the text's,
  not the window's: a maximized window's edge is the screen's). Wrapping again resets it.
- There is no horizontal scrollbar: its extent would only describe the lines on screen, and a
  thumb that changes size as one scrolls vertically misleads more than it helps.
- Painting and hit-testing an unwrapped line find the glyphs in view by binary search on their x.

## Consequences
- Only lines on screen are shaped, as before. A megabyte-long line is shaped once, as when it
  wraps; a frame then visits only the glyphs in view.
- Scrolled right past the end of the lines that remain on screen, the view stays where it is
  until the next horizontal scroll brings it back.
- Code block backgrounds reach the right edge however far the text is scrolled; a hidden `---`
  rule is as wide as the column, from the line's start.
