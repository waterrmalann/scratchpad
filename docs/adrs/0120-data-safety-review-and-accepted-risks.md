# 0120. Data safety before V1: what the session guarantees and the risks accepted
Date: 2026-10-09
Status: Accepted

## Context
A final review followed a note's text from the keystroke to the disk (ADRs 0060-0066, 0081) and
attacked the races between saves, sidebar renames and deletes, folder changes, quitting and
recovery. Some gaps lost typed text; others are too rare or too costly to close for V1.

## Decision
- A synchronous flush (close, quit, folder change) takes over the job running in the background:
  its outcome is ignored when it arrives, as the flush may already have done newer work.
- A save that ran while its note was renamed or deleted from the sidebar is finished the same way
  everywhere (after the save, and in a synchronous flush): the old file goes if the save
  recreated it, its snapshot under the old name goes, and after a rename the text is saved again
  under the note's current name (followed through further renames), open or not. Unless the save
  wrote, that file must still hold what the save expected, so other programs' changes are kept.
- Offered (recovered or unsaveable) text keeps a snapshot of its own: before the note's own saves
  or snapshots replace or remove it, it is copied to a new `.draft-` key (a new note on restart).
- Recovered text waiting for its note to load is offered again if the notes folder changes.
- A change reported while the open note is still loading is checked once it has loaded.

## Consequences
Accepted, with the reason:
- Keys typed while a note loads (a few ms; the editor is read-only and shows the previous note)
  are dropped: the user sees that nothing was typed, and a note not yet shown has no caret.
- Deleting the open note drops edits not yet written to it. Reaching Delete takes longer
  than the autosave delay; a flush first would need a re-entrant synchronous save.
- A read-only note fails its save only after the rename retries (~150 ms, off the UI thread).
- A rename by another program looks like a delete (ADR 0063). GPUI 0.2.2 does not handle
  `WM_QUERYENDSESSION`/`WM_ENDSESSION`, so logging off is not a quit: the flush when the window
  is deactivated, autosave and 500 ms snapshots bound the loss.
- A second instance is allowed (as with Notepad). It sees the first one's saves as changes by
  another program, but shares the recovery folder: it may offer the other's live snapshots.
- A notes folder that is missing at startup (removed drive) shows a toast and is not retried
  until a restart or a folder change. With both it and the recovery folder full or gone, text
  that cannot be saved lives only in memory and is lost on exit.
- Notes are read whole; a multi-GB `.md` file can exhaust memory when opened or searched.
- Conflicts are detected by content, never by time; a crash snapshot written while the clock was
  ahead is offered only once the clock passes it.
