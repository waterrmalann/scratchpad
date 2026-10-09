# 0137. A Notepad-style status bar
Date: 2026-10-09
Status: Accepted

## Context
Notepad users expect a status bar telling them where the caret is, how long the text is, and its
line endings and encoding. It must cost nothing while typing in a 10 MB note.

## Decision
- A bar under the note, as wide as the editor pane (the sidebar is navigation, the bar describes
  the open note). Left: `Ln 44, Col 10 | 2,421 characters`, or `12 of 2,421 characters` while
  text is selected, as in Windows 11 Notepad. Right: `Windows (CRLF)` or `Unix (LF)`, then the
  encoding. A later zoom level (`100%`) goes first on the right, as in Notepad.
- Ln and Col are 1-based and logical (soft wrapping and hidden Markdown markers do not change
  them). Col and the counts are in characters (Unicode scalar values): an emoji is one, an `e`
  with a combining accent two, a line break one. Graphemes would match the caret better but
  cannot be counted without scanning; bytes and UTF-16 units mean nothing to the user.
- Counting uses the rope's indexes (`Buffer::char_count`, `Buffer::char_count_in`): O(1) for the
  total and O(log n) for the column and selection. The bar observes the editor and recomputes
  only when it notifies (edits, caret moves, blinks), redrawing only when the text changed.
- Line endings are those the note will be saved with (ADR 0003): a file of lone CRs is saved
  with LF, so `Macintosh (CR)` never applies. Encoding is `UTF-8`, which is what Scratchpad
  writes; a BOM is dropped on load (ADR 0013), so there is no `UTF-8 with BOM`. While a note
  that is not valid UTF-8 is shown read-only (ADR 0066) it reads `Not UTF-8`: we do not guess
  the legacy encoding.
- `ToggleStatusBar` shows or hides it (no key; View > Status bar), remembered as
  `Config::status_bar_hidden` so the default, shown, needs no custom `Default` impl.
- Numbers use `,` as the thousands separator regardless of the Windows locale, like the dates
  in the sidebar (ADR 0051).

## Consequences
- Col and the counts may differ from Notepad's around emoji and combining marks, whose counting
  rules are not documented.
- The bar takes 26 px of height; users who want the note alone hide it once.
