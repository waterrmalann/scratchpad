# 0004. Single selection, goal column and word motions
Date: 2026-10-08
Status: Accepted

## Context
V1 has exactly one cursor (PLAN §46). Vertical movement must remember the column it started from, but the
engine only knows logical lines while the GPUI view knows wrapped visual lines and pixel positions. Word motions
need predictable rules that also work for non-Latin text.

## Decision
- `Selection { anchor, head }` in byte offsets; a cursor is an empty selection. Endpoints always lie on
  grapheme cluster boundaries.
- The editor stores a `Goal` next to the selection: `Column(n)` (grapheme clusters from line start) for the
  engine's logical `Up`/`Down`/`PageUp`/`PageDown`, or `Horizontal(f32)`, an opaque view-defined position
  (e.g. pixels) for visual-line movement done by the view via `move_to_with_goal`. Every other selection change
  resets it. Moving above the first / below the last line goes to the document start / end and keeps the goal.
- Keyboard motions are a small `Motion` enum applied by `Editor::move_cursor(motion, extend)`. Plain
  Left/Right collapse a selection to its start/end; every motion extends with Shift.
- Word motions classify each grapheme by its first char: line break, whitespace, word (`is_alphanumeric` or
  `_`), or punctuation (everything else, including emoji). Ctrl+Right skips whitespace and then one run of a
  single class; a line break is a stop of its own, so the cursor halts at a line end before crossing it.
  Ctrl+Left mirrors this. Double-click selects the run under the pointer (word, punctuation or whitespace).

## Consequences
- Scripts without spaces (CJK, Thai) move by whole runs; dictionary-based segmentation is out of scope.
- The view needs no goal bookkeeping of its own: it reads `goal()` and passes it back.
