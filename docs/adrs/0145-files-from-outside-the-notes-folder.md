# 0145. Files from outside the notes folder are edited in place like notes
Date: 2026-10-09
Status: Accepted

## Context
Notepad parity asks for File > Open (Ctrl+O): any text file, wherever it is. The note session
(ADRs 0060-0066, 0120) is where text is kept safe, and notes have rules a random file must not
get: names follow the first line (ADR 0061) and the sidebar lists the folder's `*.md` files.

## Decision
- The native dialog picks any file (GPUI 0.2.2 offers no file type filter on Windows). A `.md`
  file directly in the notes folder opens as that note. Anything else is an *external document*:
  `Selection::File(path)` and `NotesEvent::OpenFile`, with a document flag in the session.
- External documents go through the same session as notes: one ordered queue, debounced
  autosave, the same flush points, the conflict check against what we last read or wrote,
  recovery snapshots keyed by their path, read-only until "Edit Anyway" when not UTF-8, and the
  buffer's original line endings. They are never renamed after their title and never listed;
  `Notes::note_saved` ignores files that are not notes, also inside the notes folder.
- A second `NoteWatcher` (`start_file`) watches the file's folder and reports only that file
  (names compared ignoring case on Windows), always as the path we opened.
- The sidebar shows an "Opened File" row (name, then folder) above the list while it is open;
  the title bar says `name - Scratchpad`. Picking a note leaves it, saving it first; F2 and
  Delete do not apply to it. Notices say "file" instead of "note".
- `last_opened_file` in the config reopens it on startup before `last_opened_note`; if it is
  gone, the last note opens. A file that cannot be read shows a toast and the newest note opens.
- Changing the notes folder keeps it open: everything is written synchronously as before, but
  the document is not closed and the selection survives the new listing. A file that is a note
  of the new folder is left like a note instead, so it is never listed and open apart at once.
- Recovered text of a file that is not a note (or of the open document) is restored into that
  file in place; recovered text of a `.md` outside the notes folder still becomes a new note
  (ADR 0081), as an old folder's note looks the same; a toast says so.
- `scratchpad.exe <file>` opens the file as Ctrl+O would (it is put in `last_opened_file`); a
  path that is not an existing file is reported in a toast and the last document opens.

## Consequences
- External files get a note's guarantees: other programs' changes are never overwritten unasked.
- A multi-GB or binary file is read whole (ADR 0120); binary files open read-only (not UTF-8).
- A note reached through another spelling of the folder (8.3 name, junction, `\\?\`) opens as
  an external document: edited in place, not renamed, still through the one queue.
- No tabs, recent files or multiple documents: one document is open, as before.
