# 0150. An in-window menu bar whose keys come from the keymap
Date: 2026-10-09
Status: Accepted; amended by 0170, 0175, 0180, 0185

## Context
Notepad users look for File, Edit and View menus, with each command's key beside it. GPUI 0.2.2
stores application menus (`cx.set_menus`) but its Windows platform never shows them, so a native
menu bar is not available.

## Decision
- `MenuBar` draws a 30 px row under the title bar with File, Edit and View, in Windows 11
  Notepad's style and theme tokens only. A menu is a small list built when it is shown: label,
  action, enabled, tick, or a submenu (only View > Zoom). No general menu framework. (Amended by
  ADR 0175: the note's context menu has a Formatting submenu too, and a shared `menu` module,
  `crates/scratchpad/src/menu.rs`, draws both menus' box, rows and separators and holds their
  arrow, Enter, Space and Escape actions; each menu keeps its own items and state.)
- Every command is an existing action. Choosing one puts focus back where it was before the menu
  opened, then dispatches the action there (`window.dispatch_action`), exactly as its key would.
  If that element is gone (Go to line closes when the menu takes focus; the sidebar shown over a
  narrow window closes) focus goes to the note.
- Edit's editing commands (undo, redo, cut, copy, paste, delete, select all) always act on the
  note and leave the caret there, wherever focus was, as Notepad's Edit menu edits the document:
  Undo from the find bar undoes its Replace all, Paste from the note list pastes into the note.
  (The one-line fields have no history; their own Ctrl+X/C/V still work.) They are greyed when
  they cannot apply: the note is read-only (ADR 0066), there is nothing to undo or redo, or
  nothing is selected (cut, copy, delete, as in Notepad). Paste does not look at the clipboard.
  Greyed items do nothing and are skipped by the arrow keys.
- The keys shown are read from the keymap: the bindings that would run the action with the note
  focused (`bindings_for_action_in`), written as Windows does (`Ctrl+Shift+S`, `Ctrl+Plus`,
  `Del`). Of several, the one with the fewest modifiers, then a character over a named key
  (Ctrl+Y over Ctrl+Shift+Z, Ctrl+C over Ctrl+Insert). Rebinding a key changes the label.
- Pressing a title opens its menu; moving to another title switches to it; a click anywhere else
  only closes it, as the sidebar overlay does (ADR 0135). The open menu holds focus, so Up/Down,
  Left/Right (neighbouring menu, into and out of Zoom), Enter/Space and Escape work; Escape and
  closing put focus back. Alt+F, Alt+E, Alt+V and F10 open a menu with its first item highlighted.
- View ticks show live state: the status bar, word wrap, source mode (Ctrl+/) and the sidebar (Ctrl+\).
  (Amended by ADR 0170 and 0180: also Spell check and Autocorrect, greyed without a checker and
  Autocorrect while Spell check is off, and Auto-replace symbols.)

## Consequences
- The bar takes 30 px of height in every window size; it cannot be hidden.
- Alt alone does not focus the bar, and access keys are not underlined.
- A menu longer than the window (Edit in a minimum-size window) is pushed up rather than scrolled.
