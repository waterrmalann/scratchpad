# 0130. The text uses the full window width
Date: 2026-10-09
Status: Accepted; supersedes the centred column of ADR 0031 and the margin of ADR 0070

## Context
ADR 0031 set the text in a centred column at most 680 px wide. On a wide window that leaves
large empty margins, and Notepad users expect the text to start at the left edge and use the
whole window. Heading `#`s hang left of the text (ADR 0070), so the left margin cannot be tiny.

## Decision
- The text column starts `LEFT_PADDING_EMS` (2.8) × the font size from the editor's left edge
  and ends 24 px from its right edge, clear of the scrollbar. The vertical padding is unchanged.
- The left padding is the room heading markers hang in. `### ` at the H3 size in Segoe UI
  semibold measures about 2.56 em of body text (`#` and `##` less); 2.8 em holds it with a few
  pixels to spare, where 2.5 left the markers touching the sidebar. `####` and deeper still
  move their text by what does not fit, never off the edge.
- The hang room no longer depends on the window width, so resizing re-shapes only because the
  wrap width changes.

## Consequences
- Lines are as long as the window is wide; a reader who wants a narrower measure narrows the
  window, as in Notepad.
- At 100% the text starts 42 px from the edge.
