# 0060. The open note's session and one ordered queue for its file operations
Date: 2026-10-08
Status: Accepted

## Context
Autosave must never lose text (PLAN §7, §67) and never block typing, also for 10 MB notes. Loads,
saves, checks against the disk (ADR 0063), renames and recovery snapshots all touch the same
files, often for the note being switched away from while the next one loads. Run concurrently, an
older save can land after a newer one, a load can read a file a pending save is about to replace,
and a save racing a sidebar rename or delete can recreate the old file.

## Decision
- `session::Session` owns the open note's lifecycle: what the editor shows (`None`, `Loading`,
  `Draft`, `Note`), whether it is dirty, the text last read from or written to its file, and the
  notice waiting for a decision. Views stay thin; the editor stays the source of the text.
- Every file operation is a `Job` in one FIFO queue worked off by a single task: file work runs on
  the background executor, one job at a time; renames (which go through the notes model) run on
  the UI thread in their turn. A queued save of the same file is updated in place, so the latest
  text wins and keeps its place before a later rename; only the latest of several notes opened in
  quick succession is loaded, after everything queued before it.
- Opening a note first hands the current note's edits to the queue, then queues the load. Until it
  arrives the editor is read-only and still shows the previous note, so no keystroke can go to the
  wrong note and nothing flashes. A note that cannot be read shows a toast and an empty, read-only
  editor.
- Renames and deletes from the sidebar retarget or drop queued jobs. A save already running
  against the old path is marked; when it finishes, the old file is removed if it holds exactly
  what that save wrote, and the text is saved under the new name (or dropped after a delete, as
  the notes model's contract asks).
- Closing the window or quitting writes everything synchronously on the UI thread: tasks do not
  run after that. A shared counter marks every job started before as done, so a background save
  that has not run yet skips itself instead of overwriting newer text.

## Consequences
- The UI thread never waits for the disk while the app runs; only closing does.
- A slow disk delays later loads behind earlier saves, never the other way round.
- One queue for all notes is simpler than per-file queues and fast enough for one user's edits.
