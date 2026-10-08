# 0081. Changing the notes folder
Date: 2026-10-08
Status: Accepted

## Context
The settings let the user pick another notes folder (ADR 0080). At that moment the open note may
have unsaved edits, a new note may not have its file yet, a rename may be queued (ADRs 0060-0062),
and the watcher, the note list and the search all point at the old folder. Nothing may be lost or
land in the wrong folder. `SCRATCHPAD_NOTES_DIR` overrides the folder (ADR 0065).

## Decision
- "Change…" opens the native folder dialog (`prompt_for_paths`, folders only). GPUI's test platform
  panics on it, so tests set the `PickFolderForTests` global, which the panel uses instead. It
  exists only with the crate's `test-support` feature, which its tests turn on.
- The picked folder is opened off the UI thread by `NotesLocation::open_writable`: it is created
  if needed and a hidden `.scratchpad-check-<pid>.tmp` is written and removed. On failure a toast
  says why (`Could not write to "Notes". Access is denied.`) and nothing changes.
- On success, in order: `Session::change_folder` leaves the note as switching notes does (edits
  waiting for a conflict decision are offered like recovered text), writes every queued save,
  rename and snapshot synchronously against the old folder (as on exit), closes the note and
  restarts the watcher. Then `Notes::change_folder` takes the new store, clears the list, the
  selection and the search (query, hits and its text cache) and lists the folder; its newest note
  opens, or a new note if it is empty. `notes_dir` is saved and the panel closes.
- Text that cannot be written to the old folder (a failed save, a change there by another
  program, a new note whose file cannot be created) is offered like recovered text in the new
  folder at once, with an error toast; its snapshot alone would only be offered after a restart.
- A save of the old folder that finishes after the switch is not added to the new list. Picking
  the current folder cancels a switch that is still opening another one.
- Recovered text of a note outside the current folder is restored into a new note here, not
  opened in place: renaming that note would move it into the current folder.
- Notes are never moved or copied; the old folder stays as it is.
- With `SCRATCHPAD_NOTES_DIR` set, the panel shows the folder and says the variable sets it;
  "Change…" is disabled and out of the Tab order. Open Folder still works.

## Consequences
- Switching briefly blocks the UI while pending writes finish, normally one small note.
- The remembered last note belongs to the old folder until a note of the new one opens; startup
  ignores a remembered note outside the folder.
