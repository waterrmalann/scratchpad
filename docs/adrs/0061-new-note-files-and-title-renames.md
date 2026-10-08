# 0061. When new notes get a file and when files follow their title
Date: 2026-10-08
Status: Accepted

## Context
In V1 the title is the file name (PLAN §44, ADR 0013): the first meaningful line names the file.
Ctrl+N must not litter empty `Untitled N.md` files (PLAN §9, ADR 0050). Naming a file after a
title that is still being typed gives names like `Ide.md`, and renaming on every autosave while
the first line is edited churns the folder for sync clients, Git and other editors.

## Decision
- A new note stays in memory until its text is not blank and either its title line is finished
  (another line follows it) or the user leaves it: switching notes, the window losing focus,
  Ctrl+S, closing or quitting. Then `Notes::save_draft` creates the file named after the title
  (or `Untitled`) and the text is saved to it. Until then its text is kept in a recovery snapshot
  (ADR 0064), so a crash does not lose it.
- An existing note is renamed only when the user changes its title line, and only at those same
  flush points: the title when the note was opened (or last renamed) is remembered, and at a flush
  point a different, non-empty title renames the file. Edits below the title never rename, so a
  note whose name never matched its first line (e.g. `Meeting.md` starting with `# Agenda`) keeps
  its name until its title line is edited. A rename from the sidebar is kept until then too.
- Meanwhile the sidebar shows the live title for the open note (`Notes::open_title`), and for the
  new note instead of "New Note", so the list matches what the user typed.
- Renames are queued behind the save of the same note (ADR 0060). Name collisions get a number
  from the store (`Ideas 2`); a failed rename shows a toast and the file keeps its name.

## Consequences
- Files appear with their final name in nearly all cases; a title typed in one go and left with
  a click gets its name immediately.
- For a while after the first line is edited, file name and list title differ.
- Notes deleted to the recycle bin and new notes never collide with rename attempts.
