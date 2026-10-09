# 0141. Go to line
Date: 2026-10-09
Status: Accepted

## Context
Notepad's Ctrl+G asks for a line number in a modal dialog and refuses numbers past the end ("The
line number is beyond the total number of lines"). Scratchpad has no dialogs besides the settings
panel, and soft wrapping (ADR 0031) makes "line" ambiguous.

## Decision
- Ctrl+G shows a small box over the top centre of the editor, in the find bar's style: "Go to line
  [12] of 340  [Go to] [Cancel]". It reuses `TextInput` (ADR 0052) and accepts only digits;
  anything else typed or pasted is dropped.
- Lines are logical lines (the text between line breaks), counted from 1, as in Notepad with word
  wrap off and as the "of 340" says. Wrapped rows are not counted.
- The field starts with the cursor's line, selected. Enter or Go to puts the cursor at the start
  of the line and the line in the middle of the view unless it is already well inside it, as a
  find jump does (ADR 0100). Escape or Cancel leaves the cursor where it was. Either way the caret
  is back in the note. Focus moving elsewhere (a click in the note, Ctrl+F) closes the box.
- A number past the end goes to the last line and 0 to the first, instead of Notepad's error:
  the bound is shown next to the field, and "go to 99999" is a quick way to the end. An empty
  field does nothing.

## Consequences
- The box is not modal: it never blocks the window, and there is no error state to explain.
- Columns (VS Code's `12:5`) are not supported.
