# 0066. Notes that are not valid UTF-8 open read-only until the user agrees
Date: 2026-10-08
Status: Accepted

## Context
`NoteStore::read` decodes invalid UTF-8 lossily and flags it (ADR 0013): saving the decoded text
would replace the original bytes (e.g. a Windows-1252 file from another tool) with U+FFFD for
good. Autosave makes that a silent side effect of the first keystroke.

## Decision
- Such a note opens read-only, with a notice bar: "This note contains characters that are not
  valid UTF-8 (shown as �). Editing it replaces them when it is saved." and an Edit Anyway button.
- Until the user clicks it nothing is saved, so the file keeps its bytes; afterwards the note
  behaves like any other. A reload after an outside change checks the flag again.

## Consequences
- Viewing a legacy-encoded note is always safe; editing it is an explicit choice.
- No encoding detection or conversion: Markdown notes are expected to be UTF-8 (PLAN §31).
