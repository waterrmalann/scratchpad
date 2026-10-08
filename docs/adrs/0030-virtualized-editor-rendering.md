# 0030. Virtualized editor rendering with an anchor scroll position
Date: 2026-10-08
Status: Accepted

## Context
A 100k-line or 10 MB note must type and scroll as smoothly as a short one (PLAN §22–23, §37, rule 5). With
soft wrapping, a line's height is only known after shaping it, and later Markdown headings will make line
heights differ, so a pixel scroll offset over the whole document would require shaping everything.

## Decision
- `EditorView` owns the engine `Editor` (source of truth) plus view state only. A custom GPUI element
  (`EditorElement`) shapes and paints the visible logical lines, and shapes 4 more beyond each edge so lines
  scrolling in are ready.
- The scroll position is an anchor, as in Zed: a logical line plus the pixel offset of the viewport top
  within it (negative only on line 0, for the 32 px top padding). Scrolling, clamping, revealing the
  cursor and hit testing walk line heights from the anchor and stop after about a viewport, so they never
  need the document height. Edits above the anchor renumber it so the text on screen stays put.
- Clamping: a note that fits does not scroll; otherwise the last line can rise to the middle of the
  viewport, so the end of a note is written at eye level rather than at the bottom edge.
- Keyboard moves and edits scroll the cursor into view with two rows of margin; jumps (Ctrl+End) only shape
  lines near the destination.
- Layouts are cached per logical line in the view (`LayoutCache`), not just in GPUI's two-frame cache, so
  scrolling back, cursor movement and hit testing reuse them. `Buffer::changes_since` drops the lines an
  edit touched and renumbers later ones; a change of font, colour or wrap width drops everything. Beyond
  512 entries the cache keeps only lines within 256 of the anchor.
- One function, `line_style`, decides font, size, line height and coloured runs per line; everything else
  works with per-line heights, ready for Markdown decorations.
- The caret blinks every 530 ms by repainting from cached layouts; nothing is re-shaped. GPUI has no partial
  invalidation, so a blink repaints the window's visible glyphs.

## Consequences
- A single huge line (megabytes without a line break) is still shaped as a whole; only its visible rows
  are painted.
