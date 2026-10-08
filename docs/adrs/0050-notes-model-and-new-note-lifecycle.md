# 0050. Notes model, background listing and the new-note lifecycle
Date: 2026-10-08
Status: Accepted

## Context
The sidebar, the editor and later the file watcher all need one view of the notes folder: the
list, the open note and the search. PLAN §39 wants the window on screen before the folder is
scanned, and PLAN §9 wants Ctrl+N to create an in-memory note whose file only appears once it
has content. Tests must run against a temp folder and never touch the real recycle bin.

## Decision
- `notes::Notes` is a GPUI entity that owns the `NoteStore`, the list (metadata only, newest
  first), the `Selection` (`None` / `Draft(id)` / `Note(path)`) and the search state. Views
  only call its methods; it emits `NotesEvent::{OpenNote, OpenDraft(id), Renamed, Deleted}`.
- Events arrive after the selection has changed, so the editor tracks which note or draft its
  buffer belongs to and, on `OpenNote`/`OpenDraft`, saves that buffer *before* showing the new
  one; on `Renamed` it retargets; on `Deleted` it drops the buffer unsaved. The full contract
  is documented on `NotesEvent`.
- Opening the store and listing run on the background executor; the list appears when ready.
  Folder errors become a toast. `refresh()` re-lists (a newer refresh replaces an older one).
- Selected means open (as in iCloud Notes); selecting the open note again emits nothing.
- New notes: `new_note()` selects a fresh `Draft(id)` and emits `OpenDraft(id)`; nothing is
  written. The integration calls `save_draft(id, title)` on the first save with content: it
  creates the file via `NoteStore::create`, puts it at the top and, if that draft is still
  open, selects it without another `OpenNote`; otherwise (the user moved to a note or pressed
  Ctrl+N again) it is only added to the list. The id makes this unambiguous: without it, the
  first draft saved while switching to a second one would steal the selection.
- Delete opens the next visible note (previous at the end) after emitting `Deleted`.
- The window takes a `NotesLocation { dir, deleter }`. `run()` uses `SCRATCHPAD_NOTES_DIR` if
  set, else `default_notes_dir()`; tests pass a temp dir and a deleter into `.trash`.

## Consequences
- A crash before the first save loses only an empty note; Ctrl+N never litters `Untitled N.md`.
- `Renamed`/`Deleted` must be honoured by the editor integration, or an autosave would write
  to the old path and resurrect a deleted file.
- Unsaved edits of a note deleted from the sidebar are not in its recycle bin copy.
- The config's `notes_dir` is not read yet; the integration should prefer it over the default.
