# 0032. Editor input: keys, clipboard, IME and mouse
Date: 2026-10-08
Status: Accepted

## Context
The editor must behave like a native Windows text control (PLAN §15, §35, §46) on top of an engine that
works in LF-normalised byte offsets (ADR 0002, 0003), while platform text input speaks UTF-16.

## Decision
- Commands are GPUI actions in `actions::editor`, bound under the `Editor` key context. Beyond the plan's
  list: Ctrl+Shift+D duplicates lines (Ctrl+D stays free for selecting the next match later), Alt+Up/Down
  move lines, Ctrl+Y and Ctrl+Shift+Z redo, and the classic Shift+Delete / Ctrl+Insert / Shift+Insert
  cut, copy and paste.
- Copy and cut put CRLF line breaks on the clipboard on Windows, as Windows apps expect; GPUI passes text
  through unchanged. Pasted text is normalised by the engine.
- `EntityInputHandler` converts UTF-16 at the boundary with the buffer's helpers. The selection inside a
  composition arrives in UTF-16 relative to the composed text and is converted against that text. Typed
  characters, dead-key results and IME commits all go through `Editor::insert_text`, which replaces the
  marked text when there is any. An empty replacement with no composition (Japanese IMEs report "no
  composition" that way) is ignored rather than deleting the selection.
- Losing focus breaks the undo group. GPUI reports a window deactivation as a focus loss, so switching
  away also ends the step, and the caret stops blinking and hides until focus returns.
- `EditorEvent::Changed` is emitted after any input that changed the text, never for `set_text`, so loading
  a note does not look like an edit to autosave.
- Mouse: click places the cursor at the nearest character boundary, Shift+click extends, double- and
  triple-click select a word or line, and dragging extends by the unit of the first click. Dragging past
  the top or bottom edge scrolls every 16 ms by half the overshoot.

## Consequences
- Copying on Windows and pasting into the same note round-trips exactly (CRLF back to LF).
- IME behaviour is covered by tests that call the input handler directly; real Windows IMEs and dead keys
  need manual checks.
