# 0051. Sidebar note list: uniform virtual rows and iCloud-style interactions
Date: 2026-10-08
Status: Accepted

## Context
PLAN §8 and §42-43 describe a notes list rather than a file tree: date groups, newest first,
click to open, double-click to rename, a small context menu. It must stay fast with
thousands of notes and be fully usable from the keyboard.

## Decision
- The list is a `uniform_list` (only visible rows are rendered) of rows rebuilt when the notes
  model changes or the date does, not per frame: group headers, the draft, notes (or search
  hits). Every row is 46 px; a header puts its label at the bottom, so its empty top
  half is the gap between sections. `list` with variable heights would need its state reset
  (and scroll lost) whenever the rows change.
- Rows show the title and a muted second line: time (today/yesterday), weekday (last week) or
  date. No content preview, since that would read every file at startup.
- Selected row: `selection` token while the list has focus, a neutral tint otherwise.
- Mouse: click opens; double-click (or context-menu Rename, or F2) edits the title in place
  with the text input; Enter or focus loss commits, Escape cancels, an empty title cancels.
  Right-click opens Rename / Delete / Show in Folder (`deferred` + `anchored`, outlined row).
  Show in Folder runs `explorer /select,"path"` via `raw_arg` (Explorer's own parsing).
- Delete needs no confirmation: notes go to the recycle bin (ADR 0011).
- Keyboard (list focused): Up/Down open the previous/next note and scroll minimally; Enter
  emits `SidebarEvent::FocusEditor`; F2 renames; Delete deletes.
- Width: a 5 px drag strip on the right edge, clamped to 180-480 px; `width()`/`set_width()`
  for persistence.
- Ctrl+N, Ctrl+P and Ctrl+Shift+F are handled on the window root so they work from anywhere.

## Consequences
- No scrollbar yet: GPUI 0.2.2 has none built in; wheel and keyboard scrolling work.
- Times use 24-hour `HH:MM` and English month/day names regardless of the Windows locale.
- Tests locate rows with `debug_selector`s (`note:<title>`, `hit:<title>`, `group:<label>`).
  GPUI never clears recorded debug bounds, so they prove an element was drawn, not that it is
  gone; absence is asserted on the model. Test windows must be activated for focus-out events.
