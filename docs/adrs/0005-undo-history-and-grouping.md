# 0005. Operation-based undo history and grouping rules
Date: 2026-10-08
Status: Accepted

## Context
PLAN §16 asks for undo built from edit operations, not snapshots, with typing such as "hello world" undoing
in one step. Undo must restore the cursor, IME composition must not leave a trail of intermediate states, and
later features (Markdown toggles, list continuation) need a generic undoable primitive.

## Decision
- Every buffer change is an `Edit { start, deleted, inserted }`. A transaction is a list of edits plus the
  selection before and after; undo reverts its edits in reverse order and restores the selection before, redo
  re-applies them and restores the selection after. Any new edit clears the redo stack.
- Adjacent edits inside a transaction are folded (typing appends, backspace prepends, forward delete appends,
  replacing exactly the previous insertion overwrites it), so a typed paragraph is one string in memory.
- An edit joins the previous transaction only if all hold:
  - both are of the same mergeable kind: typing (text without line breaks), backspace (single cluster),
    forward delete (single cluster), or IME composition;
  - the selection before the new edit equals the selection after the previous one;
  - the group has not been broken since.
- The group is broken by: any selection change that is not an edit (motions, clicks, select-all), undo/redo,
  an edit of another kind, `break_undo_group()` (the app calls it on focus loss, save, etc.), committing or
  unmarking an IME composition.
- Never merged, each its own step: newline, paste, cut, word deletion, duplicate/move lines,
  `replace_range`. Typing over a selection starts a new step that further typing joins.
- No time-based grouping: it would need an injected clock and has not been asked for.
- History is unbounded for now.

## Consequences
- Undo granularity is coarse for long uninterrupted typing; a pause does not split it.
- A whole IME composition, including its commit, is one undo step that restores the pre-composition
  selection.
- Multi-edit transactions for future Markdown commands only need a new public entry point; the history
  already stores edit lists.
