# 0126. Keyboard conventions for Markdown text
Date: 2026-10-09
Status: Accepted

## Context
The final review walked through writing notes by keyboard. Keys that treat Markdown as plain text
surprised: Home put the caret before a list bullet, where typing breaks the item, Tab in a list
item inserted spaces in its text, and Ctrl+Left stopped at `**` that live preview does not show.

## Decision
- Home on a line that starts with block markers (list bullet or number, task box, quote `>`,
  heading `#`s, with any indentation before them) goes to the start of the text after them; from
  there, Home goes to the line start, and from the line start back to the text (VS Code's smart
  Home). Lines without such markers, and wrapped rows after the first, keep plain Home. Shift+Home
  selects the same way.
- Tab on list items nests them one level deeper, Shift+Tab moves them one level out (as in
  Obsidian or Typora), when every non-blank line the selection touches is a list item outside
  code. Tab indents an item by the width of its own marker (`- ` 2, `1. ` 3), which is what
  CommonMark needs to put it under the item above; Shift+Tab goes back to the parent item's
  indentation. Lines indented further below an item (its children; after a blank line, those
  indented to its text) move with it. Elsewhere Tab still inserts four spaces (amends ADR 0031)
  and Shift+Tab does nothing. One undo step.
- Ctrl+Left/Right (and with Shift) move on over runs of hidden markers instead of stopping at
  them, as if the markers were not there. Markers the cursor has revealed are stops like any
  punctuation. Ctrl+Backspace/Delete are unchanged: deleting is done where the markers show.
- Ctrl+B, Ctrl+I, Ctrl+Shift+X and Ctrl+E with the cursor at the end of a span's text step over
  its closing marker instead of removing the span (amends ADR 0043), so "toggle, type, toggle,
  type" works as in a word processor.

## Consequences
- Indented plain text and code keep Notepad's Home, to the line start.
- Tab with a selection that is not all list items still replaces it with spaces, as in Notepad.
- An item nested under nothing (the list's first) is indented all the same; Markdown keeps it in
  the top-level list.
- To remove formatting with the cursor at the end of a span, press the toggle twice (step out,
  then remove the span just before the cursor) or select the text.
