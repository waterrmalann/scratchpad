# 0136. Confirm before deleting a note
Date: 2026-10-09
Status: Accepted

## Context
ADR 0051 deleted a note without asking because it goes to the recycle bin. In use, a stray Delete
key in the note list or a slip in the context menu makes a note vanish from the list with no
visible way back; few users think of the recycle bin. Notepad-like apps ask first.

## Decision
- Delete (context menu) and the Delete key in the note list both ask with the system's own dialog,
  GPUI's `window.prompt(PromptLevel::Warning, "Delete \"<title>\"?", "The note will be moved to
  the Recycle Bin.", ["Delete", "Cancel"])`, a Windows task dialog. No custom dialog: native,
  modal, keyboard-accessible and nothing to theme.
- Cancel, Escape or closing the dialog does nothing.
- The sidebar remembers which note it asked about. The note can still change while the dialog is
  open (a title rename lands, a save renames the file, another program deletes it), so the path
  follows `NotesEvent::Renamed` and is forgotten as soon as the note leaves the list (deleted here,
  or by another program once the list catches up). The answer acts on that note only: never on
  whatever later takes its name.
- One question at a time: a delete request while a dialog is open is ignored.

## Consequences
- Deleting takes one more keystroke (Enter accepts Delete, the first button).
- GPUI 0.2.2 titles the dialog "Warning" and does not let us set the window title or default
  button; acceptable for a native look.
- A note renamed by another program while the dialog is open is not followed (that arrives as a
  delete and a new file): the answer then does nothing. A file replaced before the list catches
  up (as other editors save) is taken to be the same note.
