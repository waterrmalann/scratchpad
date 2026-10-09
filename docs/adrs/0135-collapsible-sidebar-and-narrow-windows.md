# 0135. A collapsible sidebar that floats over the note in narrow windows
Date: 2026-10-09
Status: Accepted

## Context
The sidebar (ADR 0051) always took 180-480 px. Writers want the note alone on screen, and in a
narrow window (snapped to half a laptop screen) a docked sidebar squeezes the note to a sliver.
The note has priority over the list.

## Decision
- `ToggleSidebar` (Ctrl+\, and a navigation button in the sidebar header and, while the sidebar
  is hidden, at the note's top left) collapses or expands the docked sidebar. The choice is
  remembered in `Config::sidebar_collapsed`; the width is kept, collapsed or not.
- Below `AUTO_COLLAPSE_WIDTH` (720 px, where a default 260 px sidebar leaves 460 px of note) the
  sidebar is never docked, whatever the setting. Asking for it there (Ctrl+\, the button, Ctrl+P
  or Ctrl+Shift+F) shows it as an overlay: the same sidebar, floating over the left edge of the
  note with a shadow, while the note keeps its full width. Search shortcuts also show it this way
  when the user collapsed it in a wide window: a quick look should not undo their choice.
- The overlay is a flyout. It goes away when focus leaves it (Escape in the search field, Ctrl+N,
  the settings), on Escape in the list, on a click outside it (which does nothing else), and when
  a note is chosen by clicking it or with Enter in the search. Browsing with Up/Down keeps it.
  Another window becoming active (the delete confirmation) does not dismiss it.
- Widening the window past the threshold docks the sidebar again if the user had it expanded.
  When the sidebar disappears while it has focus, the caret goes back to the note.
- `AppWindow::sidebar_mode()` (`Docked` / `Hidden` / `Overlay`) is the one place this is decided.

## Consequences
- In the overlay, the first click of a double-click opens the note and closes the overlay, so
  renaming there uses F2 or the context menu.
- The threshold is a fixed window width, not a function of the sidebar width: a 480 px sidebar
  in a 720 px window leaves only 240 px, but the user chose that width.
