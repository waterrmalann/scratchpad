# 0112. A size budget for the undo history
Date: 2026-10-09
Status: Accepted

## Context
ADR 0005 left the undo history unbounded. It lives as long as the note is open (opening another
note starts afresh), but within that time every large paste stays in memory, and choosing between
two versions of a 10 MB note (ADR 0063) records two whole-document replacements, about 40 MB. Idle
memory is a V1 criterion (PLAN §37).

## Decision
- The history keeps at most `UNDO_HISTORY_BUDGET_BYTES` = 32 MiB of edit text (deleted plus
  inserted) and forgets its oldest steps beyond that. The newest step is always kept, however large,
  so what a replacement removed stays one Ctrl+Z away.
- The size of every step but the last is kept up to date as steps are pushed and popped; a keystroke
  only sums the open step's edits (normally one), and explicit transactions are checked when they
  end. Keystroke benches are unchanged (3.5-5.5 µs in a 10 MB note).

## Consequences
- Typing never reaches the budget (a typed paragraph is one string); a session of large pastes
  loses its oldest undo steps instead of growing without limit.
- The redo stack is not counted: it only holds what was just undone and is cleared by the next edit.
- Only text is counted, not each step's bookkeeping (~150 bytes), which would take a hundred
  thousand separate steps (each newline is one) to matter.
