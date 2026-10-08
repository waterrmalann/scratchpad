# 0062. Autosave timing, flush points and failed saves
Date: 2026-10-08
Status: Accepted

## Context
PLAN §7 asks for saves 200-500 ms after typing stops, immediately when switching notes, losing
focus, closing the window and quitting, and Ctrl+S as an explicit flush (PLAN §35). PLAN §56 asks
that failed saves surface minimally and keep the buffer. A debounce alone never fires for someone
who types without pausing.

## Decision
- Every edit marks the note dirty and restarts a 300 ms timer (GPUI timer, so tests drive it with
  the test clock). The first unsaved edit also sets a 2 s deadline: continuous typing is saved at
  least that often.
- Flush points save at once and may rename (ADR 0061): opening another note or a new note, the
  window losing activation (GPUI reports switching apps that way), Ctrl+S (`SaveNote`), Ctrl+W and
  the close button (`on_window_should_close`), and quitting (`on_app_quit`). Closing and quitting
  write synchronously (ADR 0060); GPUI only gives quit handlers 100 ms, too little for a large
  note on a slow disk.
- A failed save shows `Could not save "Title". <reason>`, keeps the note dirty and writes its text
  to a recovery snapshot. The next edit or flush retries; nothing is discarded. A failed save on
  exit is logged and its text is offered on the next start (ADR 0064).
- Saving updates the note's metadata in the list (`Notes::note_saved`), so it moves to the top
  without re-listing the folder.

## Consequences
- In a hard crash, at most about 500 ms of typing is lost: recovery snapshots cover the time
  between saves while typing non-stop (ADR 0064).
- A read-only or locked note shows the error after each pause in typing until it is fixed.
- Session end by Windows shutdown is not a GPUI quit; the deactivation flush covers it in
  practice.
