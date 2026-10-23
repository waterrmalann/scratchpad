# 0160. Pinned notes
Date: 2026-10-23
Status: Accepted

## Context
Notes are listed by date, so a note that is always needed sinks as others are edited.

## Decision
- The note's context menu has Pin / Unpin. Pinned notes form a "Pinned" section below the new note
  and above the date groups, in the order of the list (most recently modified first); a pinned
  note is not listed again under its date. The date headings start afresh below it.
- Up/Down in the list follow the order shown. Search results are unaffected.
- `Config::pinned_notes` holds the full paths of the pins of every notes folder, so switching
  folders and back keeps them; `Notes` keeps those of the current folder. Renaming a pinned note
  moves its pin, deleting it drops the pin. A pin whose file is gone for any other reason is
  ignored and stays in the config.

## Consequences
- Pins are by path: a note renamed outside the app loses its pin's target.
