# 0080. A small settings panel with two settings
Date: 2026-10-08
Status: Accepted

## Context
PLAN §35 binds Ctrl+, to Settings and §33 asks for System, Light and Dark themes. PLAN §67 wants
"almost nothing the user needs to configure". Window bounds, sidebar width and the last note are
already remembered without asking (ADR 0065); the config's `notes_dir` had no UI.

## Decision
- Exactly two settings: Theme (System / Light / Dark) and the notes folder. Fonts, sizes, autosave
  timing, line width and the like stay sensible defaults. A new setting needs an ADR showing why
  no default works for most users.
- A panel over the window (440 px, centred), not a separate window: it opens instantly, needs no
  window management and keeps the app a single window. The rest of the window is covered by the
  `background` token at 60 % opacity, which blocks clicks and fades the notes in both themes.
- Choices apply immediately; there is no OK or Cancel. The theme is saved like other settings
  (written 1 s later and on exit). The selected theme segment uses the `selection` token like the
  selected note; buttons look like the notice bar's (ADR 0063). Only theme tokens are used.
- A muted footer shows the version and the config file's path, so a user can find (and back up)
  their settings without another setting.
- Escape, the close button and a click outside close it and put the caret back in the note.
  Focus leaving it (Ctrl+N, Ctrl+P while it is open) also closes it, leaving focus where it went;
  another window becoming active (the folder dialog, Explorer) does not.
- Keyboard: Tab and Shift+Tab cycle through the panel's own controls (theme, Change…, Open
  Folder, Close) rather than GPUI's window-wide tab stops, which would also reach the views
  behind it. The theme control is one stop; the arrow keys pick a theme, wrapping around like a
  radio group. Enter and Space press the focused button through GPUI's built-in keyboard click.

## Consequences
- GPUI 0.2.2 has no focus-visible state, so a clicked control shows the same accent focus border
  as one reached with Tab.
- The test platform implements neither folder dialogs nor `open_with_system`; Open Folder is
  only checked by hand.
