# 0126. Keyboard conventions for Markdown text
Date: 2026-10-09
Status: Accepted

## Context
The final review walked through writing notes by keyboard. Keys that treat Markdown as plain text
surprised: Home put the caret before a list bullet, where typing breaks the item, and Ctrl+Left
stopped at `**` that live preview does not show.

## Decision
- Home on a line that starts with block markers (list bullet or number, task box, quote `>`,
  heading `#`s, with any indentation before them) goes to the start of the text after them; from
  there, Home goes to the line start, and from the line start back to the text (VS Code's smart
  Home). Lines without such markers, and wrapped rows after the first, keep plain Home. Shift+Home
  selects the same way.
- Ctrl+Left/Right (and with Shift) move on over runs of hidden markers instead of stopping at
  them, as if the markers were not there. Markers the cursor has revealed are stops like any
  punctuation. Ctrl+Backspace/Delete are unchanged: deleting is done where the markers show.
- Ctrl+B, Ctrl+I, Ctrl+Shift+X and Ctrl+E with the cursor at the end of a span's text step over
  its closing marker instead of removing the span (amends ADR 0043), so "toggle, type, toggle,
  type" works as in a word processor.

## Consequences
- Indented plain text and code keep Notepad's Home, to the line start.
- To remove formatting with the cursor at the end of a span, press the toggle twice (step out,
  then remove the span just before the cursor) or select the text.
