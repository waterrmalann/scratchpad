# 0071. Live preview layout and column mapping
Date: 2026-10-08
Status: Accepted; Up/Down and End amended by ADR 0125

## Context
Live preview hides syntax markers away from the selection (ADR 0042). The caret, clicks, selections,
IME bounds and vertical motion all work in buffer byte columns, and revealing markers changes a line's
layout. Typing must stay well under 16 ms on 10 MB notes (PLAN rule 8).

## Decision
- A line is shaped and wrapped as displayed: hidden spans are left out of the shaped text, so
  wrapping sees the real text. `LineGeometry` works in display columns; `LineLayout`'s API takes and
  returns buffer columns, mapping with the engine's `StyledLine::display_column`/`buffer_column`.
- Hit testing maps with `Bias::Right`: a click at the edge of hidden markers lands after leading ones
  (in the word). Home maps with `Bias::Left`, End goes to the line end. Clicks and Up/Down keep the
  character under the pointer or goal x; the reveal that follows may shift the text, as in Obsidian.
- Bullets and task boxes are drawn as shapes of a fixed width; the cursor never stops inside one
  (positions inside snap to its end) unless the selection reaches inside it, which shows it as typed.
- The layout cache keys each line by `LineKey` (its `StyledLine` plus whether shapes are drawn). After
  any edit or selection change the cache's generation is bumped and each line recomputes its key
  (one `styled_lines` query for that line) when next needed; only lines whose key changed are
  re-shaped. Caret blinks recompute nothing. No query covers more than the lines being laid out.
- Source mode (Ctrl+/) styles lines without a selection: every marker shows, nothing is a shape.

## Consequences
- Moving the cursor re-shapes the line it leaves and the line it enters, if they have markers.
- Revealing markers can rewrap a row: End on a wrapped row whose last word has hidden markers can
  leave the caret at the start of the next row, so a second End or a Home acts on that row.
- Measured typing latency stays at ~4.5 ms p50 on 1 and 10 MB notes (docs/performance.md).
- The first frame after a jump into an unparsed part of a 10 MB note takes ~17 ms (parse + shape).
