# 0125. Cursor targets settle on the markers they reveal
Date: 2026-10-09
Status: Accepted

## Context
Live preview reveals the markers the selection touches (ADR 0042), so the line the cursor moves to
is laid out differently once it is there. ADR 0071 accepted two consequences: Up/Down kept the
character under the goal x, so the caret jumped sideways when the markers appeared, and End on a
wrapped row ending in a word with hidden markers could push that word, and the caret, onto the next
row, where a second End went further.

## Decision
- A target is checked against the layout its line gets with the selection there
  (`Lines::layout_with`, a cache lookup keyed by that `LineKey`; usually the layout is shaped anyway
  for the next frame), for at most three rounds.
- Up/Down/PageUp/PageDown take the column nearest the goal x in that layout. Of the candidates
  seen, the one on the intended row and nearest x wins, so a candidate that hides the markers again
  cannot make it oscillate. The caret keeps its x; the text moves around it.
- End takes the end of the row in that layout. If the row's last word moved to the next row, End
  stops after the word before it: the caret never leaves the row and End twice stays put.

## Consequences
- Moving onto a line with markers may shape it once more; lines without markers near the target are
  never re-shaped (their key does not change).
- End on such a row stops one word short of what was shown before the key press, since there is no
  position at the end of that word that stays on the row.
