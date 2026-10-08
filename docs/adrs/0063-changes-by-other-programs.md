# 0063. Changes made to notes by other programs
Date: 2026-10-08
Status: Accepted

## Context
Notes are plain files (PLAN §5, §31) that Git, sync clients and other editors change. PLAN §30:
reload a clean note, never silently overwrite conflicting changes. The core watcher (`NoteWatcher`)
also reports our own atomic saves, sometimes several times, and Windows reports a replaced file as
removed and created.

## Decision
- A task polls the watcher every 250 ms (no thread blocks on it) and passes each batch to
  `Session::disk_events`, which is also the seam tests use instead of the OS watcher.
- Events for other notes re-list the folder after 250 ms of quiet. An event for the open note
  queues a check (ADR 0060), which reads the file after any pending save of it:
  - same text as we last read or wrote: our own save, ignored;
  - different, and no unsaved edits (in the editor or queued): reloaded with
    `EditorView::reload_text`, which keeps the cursor's line and column and the scroll position
    as far as the new text allows;
  - different, with unsaved edits: a notice bar above the editor says the note was changed by
    another program, with Keep My Version (save over the file) and Load Their Version. Until the
    user chooses, nothing is saved to the file; edits go to a recovery snapshot (ADR 0064).
    Either choice replaces the editor's text as one undoable edit, so the other version stays one
    Ctrl+Z away. Their version loads read-only if it is not valid UTF-8 (ADR 0066). Leaving the
    note before choosing offers the edits like recovered text.
  - gone: a notice offers Keep Note (save recreates the file) and Close Note. The buffer is kept
    either way until the user decides; a file that comes back clears the notice.
- The watcher reports a change up to 250 ms late, and an autosave may come first. So each save
  reads the file and compares it with the text we last read or wrote; if it differs, nothing is
  written and the same notice shows (or, when the user has already left that note, the edits are
  offered like recovered text, ADR 0064). Choosing Keep My Version saves without this check.
- Notices are a bar at the top of the editor pane (surface colour, border, native-style buttons),
  not a modal: typing stays possible and the decision is never forced.

## Consequences
- Our own saves never reload or prompt; comparing text costs one read of the open note per event.
- A rename by another program looks like a delete of the open note.
- The version not chosen lives on only in the editor's undo history, for as long as the note is
  open.
