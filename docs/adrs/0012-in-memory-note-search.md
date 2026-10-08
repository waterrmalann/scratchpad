# 0012. Note search: in-memory substring scan with a small text cache
Date: 2026-10-08
Status: Accepted

## Context
PLAN §28 asks for title and content search and forbids search engines or indexes until there is
demonstrated need. Search runs while typing, so it must not re-read every file per keystroke.
Snippets and highlight ranges must be valid byte ranges even where Unicode lower-casing changes
byte lengths (`İ` becomes two chars, `ẞ` a shorter `ß`).

## Decision
`NoteSearch` scans notes directly:
- The query is whitespace-normalized and matched as one case-insensitive phrase.
- Text is cached per note, keyed by (path, modified time, size) taken from `Note`, together with
  a folded copy. The cache is dropped for notes that left the list and capped at 64 MiB; notes
  that do not fit are re-read each time.
- Folding lower-cases a char only when the result is a single char of the same UTF-8 length;
  other chars (`İ`, `ẞ`, the Kelvin sign) are kept as they are. Offsets in the folded text are
  therefore offsets in the original and `str::find` can be used directly, with no mapping table.
- Title matches come first, then content matches, each newest first. The snippet is a single-line
  excerpt (30 chars before, 70 after the first match, `…` where cut).

## Consequences
- 1,000 notes of ~3 KB: warm search 3-5 ms, first search after startup ~134 ms (criterion bench
  `search`). Memory is about twice the size of the notes; the first search should run off the UI
  thread.
- Edge cases such as `İ` not matching `i` are accepted; they match themselves only.
- Whitespace inside notes is not normalized, so "a  b" (two spaces) does not match "a b".
- If collections grow beyond what a scan handles, an index can be added behind the same API.
