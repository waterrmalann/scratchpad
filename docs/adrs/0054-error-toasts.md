# 0054. File errors are shown as a transient toast
Date: 2026-10-08
Status: Accepted

## Context
PLAN §56: renames, deletes, saves and folder access can fail (locked or vanished files,
permissions, offline drives). The app must not crash and should say what failed briefly,
e.g. `Could not save "Meeting Notes". Permission denied.` Both the sidebar and the coming
editor integration need the same mechanism.

## Decision
- `toast` keeps one message in a GPUI global and the root view renders it at the bottom
  centre with inverted colours (`foreground` background, `background` text).
- `show_error(message)` replaces the current message and restarts a 6 s timer (the old
  timer task is dropped); clicking the toast dismisses it.
- `show_file_error(name, &scratchpad_core::Error)` formats `Could not <action> "<name>".
  <reason>`, using the store's action word and the OS message without `(os error N)`.

## Consequences
- Errors that need a decision (e.g. overwriting a lossily decoded note) need a dialog instead.
- Only the latest error is visible; earlier ones remain in the log.
