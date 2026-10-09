# 0092. File associations are deferred until the app opens file arguments
Date: 2026-10-08
Status: Accepted

## Context
PLAN §59 lists a `.md` association and an "Open with Scratchpad" context-menu entry. Registering
the app as an "Open with" candidate means Windows launches `scratchpad.exe "C:\path\note.md"`.
`scratchpad::run` ignores its command-line arguments and always opens the notes folder, so a
registered entry would start the app and silently not show the file the user chose.

## Decision
The installer registers no file associations yet. When the app can open an arbitrary Markdown
file passed on the command line (and, for a file outside the notes folder, decides how to show
and save it), add per-user entries to the MSI under `HKCU\Software\Classes`:
`Applications\scratchpad.exe` with an `open` verb, plus `.md` under `OpenWithProgids` so
Scratchpad is offered in "Open with" without taking over the default `.md` handler.

## Consequences
- Scratchpad is reachable from the Start Menu only for now.
- Taking over `.md` as the default is deliberately not planned: Windows protects that choice for
  the user, and Scratchpad should not argue with other editors.
- Amended by ADR 0145: `scratchpad.exe <file>` now opens the file (a note of the notes folder
  as that note, any other file in place), so the "Open with" entries above can be registered.
  The installer is unchanged for now.
