# 0175. The note's context menu, with the spelling corrections in it
Date: 2026-11-11
Status: Accepted; amends ADR 0150 and 0170

## Context
Right-clicking in the note did nothing unless the word was misspelled, when it opened a small menu
of corrections (ADR 0170). Notepad's right-click menu has the editing commands (Undo, Cut, Copy,
Paste, Delete, Select all), Shift+F10 and the Menu key open it at the caret, and on a misspelled
word the corrections come first in the same menu. The Edit menu (ADR 0150) already has those
commands, with rows, keys and arrow-key handling of its own, and the sidebar's note menu is a third
look.

## Decision
- **One menu.** A right-click anywhere in the text (not on a scrollbar) opens the note's context
  menu at the pointer; Shift+F10 or the Menu key open it below the cursor, with the first item that
  applies highlighted, as the menu bar does from the keyboard. On a misspelled word (at the
  pointer, or at the cursor from the keyboard) its corrections are the first section: the
  suggestions in semibold ("Looking for suggestions…" until the checker answers), then Add to
  dictionary and Ignore (a repeated word has Delete repeated word and Ignore, no Add; a read-only
  note drops the corrections and Delete repeated word). Then Undo; Cut, Copy, Paste, Delete;
  Select all; and a Formatting submenu (Bold, Italic, Strikethrough, Inline code, Link). The
  separate spelling menu is gone: its state is now a `SpellingSection` of the context menu, made
  by the spelling code, which still decides what it offers and applies the choice.
- **The cursor.** A right-click outside the selection moves the cursor there first, so Paste
  goes where the pointer was; inside the selection, the selection stays for Cut and Copy.
- **Same as the Edit menu.** Undo needs a writable note with history; Cut and Delete a writable
  note and a selection; Copy a selection; Paste a writable note. Formatting needs a writable
  Markdown note. The commands are the note's actions: choosing one gives the note its focus back
  and dispatches the action there, as its key would, and the keys shown are read from the keymap.
- **Shared rows.** A new `menu` module holds what the menu bar's menus and the context menu draw
  alike, the box, rows (label, key, tick, submenu arrow, greyed out when disabled), separators
  and muted notes, all from the theme's tokens, so both menus look the same in light and dark.
  It also holds the arrow, Enter, Space and Escape actions, bound in both menus' key contexts
  (`MenuBar`, `ContextMenu`). Each menu keeps its own items and state. The sidebar's menu is
  left as it is for now.
- **The keys are the menu's.** While the menu is open it has the focus: Up/Down move (wrapping,
  skipping disabled items), Right/Left go into and out of Formatting, Enter or Space choose,
  Escape leaves the submenu, then the menu. The menu is drawn beside the note's element tree,
  not inside its `Editor` key context, so the note's keys (Delete, Tab, Ctrl+V) do nothing while
  it is open. Window-wide keys still work: those that take the focus (Ctrl+N, Alt+E) close it
  that way, and zooming and Ctrl+\ (the sidebar), which move the text from under it, close it
  too.
- **The highlight is an item, not a row number.** The list changes while the menu is open: the
  suggestions arrive after it opens. The highlighted item is kept by what it is (its label, and
  whether it is a correction), so Enter on Add to dictionary still adds the word after
  suggestions appear above it, and the keyboard stays in Formatting.
- **Closing.** Escape, a command, the focus going elsewhere (another window, Alt+E, Ctrl+N), a
  change to the text (an edit, another note, a change from another program: the corrections and
  the history may no longer apply) or the note turning read-only close it. A click
  anywhere else only closes it: a backdrop over the window takes the click, as the menu bar's
  does (ADR 0150), so it neither moves the cursor nor opens another menu.

## Consequences
- The corrections are one Down and Enter away from the keyboard, which ADR 0170 left out.
- Nothing runs while the menu is closed; opening it costs one hit test and, on a misspelled
  word, one request to the checker.
- Redo is not in the menu, as in Notepad; it stays in the Edit menu.
- The submenu has its own anchor, so near the window's bottom it moves up as the menu does. It
  opens on the right of the menu, or on the left when it does not fit on the right and there is
  more room on the left, judged from both menus' sizes in the last frame. Neither scrolls,
  which their few items do not need.
- The sidebar's note menu still has its own rows and no arrow keys; moving it onto `menu` is
  left for later.
- End-to-end tests drive the real window: right-clicks and Shift+F10/Menu, what is enabled
  (including read-only and plain text notes), the arrow keys, the submenu, clicks elsewhere and
  the corrections, with the word list standing in for the system checker (ADR 0170).
