# 0042. Styled lines and marker reveal for live preview
Date: 2026-10-08
Status: Accepted

## Context
The GPUI view shapes one line at a time from runs of uniform style (PLAN §22/§23) and, in live preview, hides
syntax markers except around the cursor (PLAN §18/§20, Obsidian-like). The rules must be testable without GPUI,
and the cursor must never get stuck on invisible text.

## Decision
- `MarkdownState::styled_lines(buffer, lines, selection)` returns one `StyledLine` per line: a line style
  (heading level, quote, code block of any block on the line, also on empty lines and where the block starts
  after list or quote markers: for line height, bars and backgrounds) and consecutive
  `StyledSpan`s covering the line in byte columns, each with a combined `SpanStyle`, an optional `MarkerKind` and
  a `hidden` flag. The view maps a span straight to a `TextRun`. Only regions touching the lines are parsed.
- Reveal rule: inline markers (emphasis, strong, strikethrough, code, link) show while the selection touches the
  span, edges included; heading, thematic break and code fence markers while it touches the block's lines;
  quote markers, bullets and task boxes while it touches the marker's line. A selection reveals everything it
  touches. `selection = None` is source mode: nothing hidden.
- Bullets, numbers and task boxes are never hidden (hiding them would shift item text); the view may draw them
  as `•` or a checkbox in place.
- Cursor mapping lives in `StyledLine`: `display_text`, `display_column` and `buffer_column(display, Bias)` map
  between buffer columns and the displayed text where hidden spans take no space. No Markdown-aware motion is
  added to the engine: because touching a span reveals it, the cursor is never inside or next to a hidden
  marker, so grapheme motions stay correct. Clicks map with `buffer_column`, `Bias::Right` after leading markers.

## Consequences
- Moving the cursor changes which spans are hidden on the lines it leaves and enters (and whole code blocks or
  setext headings), so the view re-queries visible lines after selection changes and reshapes lines whose
  `StyledLine` changed (`StyledLine: Eq + Hash`).
- Revealing shifts text horizontally on the cursor's line, as in Obsidian; vertical motion with a pixel goal uses
  the layout the target line will have, which the view owns.
