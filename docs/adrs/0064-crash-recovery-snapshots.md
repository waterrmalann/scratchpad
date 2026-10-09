# 0064. Crash recovery: snapshots of unsaved text
Date: 2026-10-08
Status: Accepted

## Context
Release builds abort on panic (ADR 0021), so recovery data must be on disk before a crash (PLAN
§40 asks for a periodic snapshot of the active buffer). Autosave writes a note 300 ms after typing
pauses, but someone typing without pauses is saved only every 2 s (ADR 0062), and some text cannot
be saved for a while: new notes without a file, notes waiting for a decision about a change by
another program, and notes whose save failed. Saving the note itself more often would churn the
folder for sync clients and Git; a snapshot goes to a private folder.

## Decision
- A recovery snapshot (`RecoveryStore`, ADR 0014, outside the notes folder) is written 500 ms
  after the first unsaved edit and every 500 ms while edits keep coming; a pause saves the note
  first, so normal typing with pauses writes none. It is also written at each autosave tick for a
  dirty note that cannot be saved: a new note whose title line is unfinished (ADR 0061), a note
  with an open notice (ADR 0063), and as part of every save that failed or found the file changed
  by another program. A successful save of a note removes its snapshot; a new note's snapshot is
  removed after its first save. New notes are keyed by a hidden `.draft-<time>` path in the notes
  folder, which can never be a note.
- Text that could not be saved to a note the user has since left is offered at once like
  recovered text, since that note's next save would remove its snapshot.
- Closing and quitting write pending snapshots synchronously (ADR 0060).
- At startup, after the window renders, `RecoveryStore::leftovers` lists snapshots written before
  this run whose text differs from the note on disk (or whose note is gone), removing those that
  match. The notice bar offers each in turn: "Scratchpad recovered unsaved changes to "Ideas"."
  with Restore and Discard. Restore opens the note (or a new note for drafts and missing notes)
  and puts the text in the editor as an unsaved, undoable edit, so it autosaves and what it
  replaced is one Ctrl+Z away; Discard deletes the snapshot.

## Consequences
- A crash loses at most about the last 500 ms of typing.
- Snapshots never appear next to the user's notes and never hold text that is already saved.
- A restored snapshot replaces what the editor showed for that note; the version on disk is still
  there until the restored text is saved.
- Amended by ADR 0155: leftovers are the snapshots of instances that are no longer running,
  not those written before this run, so a second instance never offers a running one's.
