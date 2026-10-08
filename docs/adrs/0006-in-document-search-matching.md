# 0006. Case-insensitive in-document search on original text
Date: 2026-10-08
Status: Accepted

## Context
Find (PLAN §29) is case-insensitive by default. Lowercasing a string can change its byte length (`İ` → `i̇`,
Kelvin sign → `k`, `ẞ` → `ß`), so searching a lowercased copy and reusing its offsets yields ranges that are
wrong or not even char boundaries in the document. Highlighting and selecting matches needs exact byte
ranges of the buffer.

## Decision
- `search::find_all(text, query, CaseSensitivity)` returns non-overlapping byte ranges, left to right.
  Case-sensitive search uses `str::match_indices`.
- Case-insensitive search compares per-char `char::to_lowercase` mappings while walking the original text,
  trying each char boundary as a start. A match must end on a char boundary of the original; a query that
  ends inside one char's multi-char lowercase expansion does not match. No case-folding tables beyond std.
- `next_match` / `prev_match` pick the index of the next/previous match in that sorted list with
  wrap-around, so the UI can highlight all matches and step through them without searching again.
- Search runs on a `&str`; the app passes `buffer.normalized_text()` (a copy, roughly 1 ms per MB).

## Consequences
- Matching is simple lowercase comparison, not full Unicode case folding: `ss` does not match `ß`, final
  sigma `ς` does not match `σ`, and `istanbul` does not match `İstanbul`. Good enough for V1 notes.
- Worst case is O(text × query) for pathological inputs; typical queries fail on the first char.
