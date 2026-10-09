# 0140. Replace in the find bar
Date: 2026-10-09
Status: Accepted

## Context
Notepad parity asks for Find & Replace (PLAN §29 left it for later, ADR 0100). Replacing must not
act on matches of an older query or text (searches are debounced and run in the background for
large notes), must not loop when the replacement contains the query, and Replace All must stay
fast and undoable in 10 MB notes.

## Decision
- A chevron on the find bar's left shows a second row: a replacement field and Replace and Replace
  all buttons, as in VS Code and Notepad. Ctrl+H opens the bar with that row, taking the query
  from the selection as Ctrl+F does, with focus in the replacement; Ctrl+F hides the row again.
  Tab and Shift+Tab go between the two fields. The replacement is literal text.
- Replace (Enter in the replacement field, or the button) replaces the selected match and selects
  the next one, wrapping around. If the selection is not exactly a match, it only selects the next
  match, so the text about to change is on screen first (Notepad and VS Code do the same).
- Replace All (Ctrl+Alt+Enter as in VS Code, Alt+A as Notepad's access key, or the button)
  replaces every match, honouring Match case, as one undo step (`Editor::replace_all`). The count
  shows "Replaced 12" until the next step, query or edit. The cursor stays with its text.
- Both act on a fresh search: if a search is pending or the matches were only moved by edits,
  Replace searches the text on the spot (tens of milliseconds at 10 MB, on an explicit command),
  and `Editor::replace_all` always searches the current text itself.
- Replace puts the cursor after the replacement and searches on from there; Replace All searches
  once and replaces back to front. Neither can find text it inserted.
- One edit per match costs about 10 µs (8.5 s for 600,000 matches in 10 MB). Matches less than
  1 KB apart are replaced as one edit of the text from the first to the last: at most one edit
  per KB, 100-130 ms for any 10 MB note including undo, and the undo step holds at most about
  twice the text it spans.
- In a read-only note (ADR 0066) the buttons are muted and Replace and Replace All do nothing.
  Both only work while the replace row is shown, so a hidden replacement never deletes matches.

## Consequences
- Replace All in a large note with dense matches keeps a copy of most of the note in the undo
  history, within its budget (ADR 0112).
- No regular expressions, whole-word matching or preserving case.
- Both replace the matched text only: "e" in an "é" written with a combining accent is replaced
  and the accent stays, although the find bar selects the whole letter (ADR 0100). A match that
  starts inside a letter (the accent searched for on its own) is never the current match, so only
  Replace All replaces it.
