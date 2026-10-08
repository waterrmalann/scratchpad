# 0126. Keyboard conventions for Markdown text
Date: 2026-10-09
Status: Accepted

## Context
The final review walked through writing notes by keyboard. Keys that treat Markdown as plain text
surprised: Home put the caret before a list bullet, where typing breaks the item.

## Decision
- Home on a line that starts with block markers (list bullet or number, task box, quote `>`,
  heading `#`s, with any indentation before them) goes to the start of the text after them; from
  there, Home goes to the line start, and from the line start back to the text (VS Code's smart
  Home). Lines without such markers, and wrapped rows after the first, keep plain Home. Shift+Home
  selects the same way.

## Consequences
- Indented plain text and code keep Notepad's Home, to the line start.
