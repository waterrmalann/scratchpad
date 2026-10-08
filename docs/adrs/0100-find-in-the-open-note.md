# 0100. Find in the open note
Date: 2026-10-09
Status: Accepted

## Context
PLAN §29 asks for Ctrl+F with next/previous, case-insensitive matching (ADR 0006), highlights on
the visible matches and Escape to close. Searching must never slow typing (rule 8), also in 10 MB
notes, and highlights must line up with live preview's hidden markers (ADR 0071).

## Decision
- A small bar floats over the editor's top right corner, so opening it does not move the text.
  It reuses `TextInput` (ADR 0052) for the query and shows "3 of 12", "12 matches" or "No
  results", an "Aa" toggle for matching case (Alt+C) and previous/next/close buttons. Ctrl+F opens
  it with the selection as the query if that is on one line and at most 1000 bytes, and selects
  the query.
- Enter and F3 select the next match, Shift+Enter and Shift+F3 the previous one, wrapping around;
  F3 also works from the note and reopens a closed bar on its last query. Escape (in the bar or
  the note) closes it, puts the caret back in the note and keeps the match selected.
- The selected match is the current one ("n of m"); there is no separate index to go stale. A
  jump puts a match that is off screen (or within two rows of an edge) in the middle.
- `EditorView` owns the query and the matches; the bar is only the field and the count. The
  query and the text are searched 120 ms after they last changed, except that a new query in a
  note up to 64 KB is searched at once (well under a millisecond), so the count follows each
  key. Notes over 256 KB are copied (a rope clone, then `normalized_text`) and searched on the
  background executor. The pending
  search is one task that every change replaces, so an older query or text never lands.
- Between searches the matches follow edits (`search::adjust_matches`): later ones move, those
  an edit touched are dropped. They stay valid byte ranges, so highlights do not flicker while
  typing. A query change keeps the old matches until the new ones land, as the note search does
  (ADR 0053). Next/previous asked for while a search is pending happens when it lands, unless
  the text changes first (typing on, another note opened).
- Each frame finds the first match on the top visible line by binary search and walks matches
  only as far as the visible lines; they are painted through `LineLayout::spans`, which maps
  buffer columns to displayed ones, under the selection in the `selection` token at half
  opacity. The selected match is painted as the selection. No new theme token was needed.

## Consequences
- Typing a query in a note over 64 KB shows results 120 ms after the last key.
- New matches an edit creates appear 120 ms after typing stops.
- A match ending inside a grapheme cluster ("e" in an "e" with a combining accent) selects the
  whole cluster; one starting inside it (the accent on its own) is not counted as current.
- Replace and regular expressions are left for later (PLAN §29).
