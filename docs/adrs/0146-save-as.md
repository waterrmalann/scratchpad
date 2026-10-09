# 0146. Save As writes the text to a chosen file and goes on editing that file
Date: 2026-10-09
Status: Accepted

## Context
Notepad parity asks for File > Save As (Ctrl+Shift+S). Notes always save themselves, so the
question is what happens to the note left behind, to a new note that has no file yet (ADR 0061),
and to text typed while the copy is written. Nothing may be lost or written where the user did
not choose.

## Decision
- The native save dialog (`prompt_for_new_path`) starts in the notes folder for notes and new
  notes, in the file's folder for external documents (ADR 0145), with the title plus the
  current extension (`Untitled.md` for an untitled new note, the file name for a file). The
  dialog asks before replacing a file. Tests answer it with `simulate_new_path_selection`.
- Before the dialog opens the note is flushed (saved, renamed if its title changed), so it
  keeps exactly what it held. A new note is not: while Save As is under way its flushes (the
  window loses activation to the dialog) and autosaves only write recovery snapshots, so it
  never gets an `Untitled.md` of its own. Cancelling resumes its usual saving.
- The copy is written atomically by a `Job::SaveAs` in the session's queue, after every write
  queued before it, without the conflict check. The editor is read-only until it is written, so
  no keystroke belongs to neither file.
- Written: the session edits the chosen file with the editor's buffer, cursor and history kept;
  a `.md` in the notes folder becomes a listed, selected note, anything else an external
  document. A snapshot of the document left behind is removed: its text is in the new file.
- Failed: a toast names the file, and editing goes on where it was. Picking the open file
  itself just saves it, so a change by another program is still asked about. A second Save As
  while one is under way is ignored.
- A document that is not valid UTF-8 is not saved as another file until "Edit Anyway": the copy
  would hold replacement characters, the decision the notice asks for (ADR 0066). A toast says so.

## Consequences
- Save As does not convert other encodings; after "Edit Anyway" the copy holds the replacement
  characters and the original keeps its bytes.
- A name typed without an extension is saved without one and edited as plain text.
