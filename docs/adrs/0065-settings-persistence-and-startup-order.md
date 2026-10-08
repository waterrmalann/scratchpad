# 0065. Remembered settings and the startup order
Date: 2026-10-08
Status: Accepted

## Context
PLAN §32 asks the app to remember window size and position, sidebar width, the last note and the
theme; PLAN §39 orders startup as config, window, render, previous note, then the rest of the
notes. Monitors change between runs, and `Config` (ADR 0014) leaves validation to the app.

## Decision
- `run` reads only the config file before opening the window. The notes folder is
  `SCRATCHPAD_NOTES_DIR`, else `notes_dir` from the config, else `Documents\Scratchpad`.
- The window opens with the remembered bounds on the display that contains their centre, clamped
  to it, maximized if it was; with no such display it opens at the default size centred on the
  main display. The restore bounds are saved, so a maximized window un-maximizes to its old size.
- The notes model opens the folder in the background and then, before listing, opens the
  remembered note if it still exists in that folder. Otherwise the newest note opens when the
  list arrives, or a new note if the folder is empty, so the user can type right away.
- `settings` keeps the `Config` in a GPUI global. Changes (window moved or resized, sidebar width,
  opened note) are written 1 s after they stop, and synchronously when the window closes or the
  app quits; the theme is taken from the active mode when writing.
- `Storage` carries every path the app writes (notes, config, recovery) so tests use temp folders.

## Consequences
- No note is read and no folder listed before the first frame; the remembered note can appear
  before a large folder has been listed.
- On mixed-DPI setups the restored position is in the logical pixels of the display it was saved
  on, so a window may move slightly between monitors with different scaling.
