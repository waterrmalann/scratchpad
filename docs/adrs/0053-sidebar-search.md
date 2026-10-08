# 0053. Sidebar search runs in the background and only the latest query wins
Date: 2026-10-08
Status: Accepted

## Context
PLAN §28 asks for title and content search with snippets from a search field (Ctrl+P,
Ctrl+Shift+F). `NoteSearch` (ADR 0012) reads every note on its first run (~130 ms for 1,000
notes) and searches run on every keystroke, so results can arrive out of order.

## Decision
- The query lives in `Notes`; the search field mirrors it. A non-blank query replaces the list
  with the hits; an empty one restores the grouped list.
- Each search runs `NoteSearch::search` on the background executor. The `NoteSearch` (and its
  text cache) is shared behind a `Mutex`. The task that applies the results is stored in
  `Notes`; starting a search replaces it, which drops and cancels the previous one, so an
  older query can never overwrite a newer one. Previous hits stay visible until new ones land.
- Searches re-run after the list changes (refresh, rename, new note). A renamed or deleted
  note is patched in the current hits immediately so a click never targets a stale path.
- Hits show the title and the snippet (or the date for title-only matches) with matched
  ranges highlighted using the `selection` token.
- Escape clears the query and returns focus to where it was before Ctrl+P (else the list);
  Enter opens the first hit. Ctrl+N leaves search so the new note is visible.

## Consequences
- The stale-results test types a query with no parking between keystrokes and runs with 20
  seeds, so the test executor orders the competing searches differently each time.
- Hits are not limited; with very large collections the search itself, not the list, is the
  cost (ADR 0012).
