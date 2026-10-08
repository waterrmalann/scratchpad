# 0072. Markdown shortcuts, task boxes and links
Date: 2026-10-08
Status: Accepted

## Context
Milestone 4 asks for formatting shortcuts, Markdown-aware Enter and pairing (PLAN §36, §49, §50),
the engine provides them (ADR 0043, 0044), and links open with Ctrl+click without Markdown ever
executing anything (PLAN §47, §60).

## Decision
- Bindings in the `Editor` context: Ctrl+B bold, Ctrl+I italic, Ctrl+Shift+X strikethrough (as in
  Slack; Ctrl+Shift+S reads as "save as"), Ctrl+E inline code (as in Notion), Ctrl+K link, Ctrl+/
  source mode. Enter continues lists and quotes; Shift+Enter inserts a plain line break.
- Keystrokes (`replace_text_in_range` without a range) go through `Editor::insert_typed`, which inserts
  IME commits verbatim; paste keeps `paste`. Backspace uses `backspace_typed`.
- Clicking a task box toggles `[ ]`/`[x]` as one undo step, keeping the selection and scroll position.
- Ctrl+click opens the link under the pointer (not the nearest cursor position) without moving the
  cursor, only for `http`, `https` and `mailto`. Anything else (`file:`, `javascript:`, app schemes,
  relative paths) is ignored and logged. Holding Ctrl over a link shows a pointing hand.
- A read-only note (ADR 0066) changes for none of these: shortcuts, Enter and pairing do nothing and
  a click on a task box places the cursor. Links still open and source mode still toggles.

## Consequences
- Ctrl+I, Ctrl+E and Ctrl+K are no longer free for other commands in the editor.
- Links to local files cannot be opened from a note; that can be revisited with a confirmation.
