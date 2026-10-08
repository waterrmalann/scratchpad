# 0013. Note naming, collisions and text encoding
Date: 2026-10-08
Status: Accepted

## Context
In V1 the title and the file name stay aligned (PLAN §44), so every title must become a file name
that is valid on Windows, unique in a flat folder, and stable under case-insensitive filesystems.
Notes may also arrive from other tools with BOMs or invalid UTF-8 (PLAN §31, §56).

## Decision
- A note's title is its file stem. `title_from_content` takes the first meaningful line (skipping
  blank lines, thematic breaks and code fences, stripping Markdown block markers, 80 chars max).
- `sanitize_file_stem` turns forbidden and control characters into spaces, collapses whitespace,
  trims leading/trailing dots and spaces (a leading dot would hide the note), appends `_` to
  reserved device names (`CON`, `com1.txt`), and caps at 100 chars. It is idempotent.
- Collisions are detected case-insensitively on every platform and resolved with a numeric suffix:
  `Untitled.md`, `Untitled 2.md`, ... A note never collides with itself, so a case-only rename
  works. Creation reserves the name with `create_new`; rename has a small check-then-rename race
  that is accepted for a single-user folder.
- `read` never fails on bad UTF-8: it decodes lossily and sets `NoteText::lossy` so the app can
  warn before the replacement characters overwrite the original bytes. A UTF-8 BOM is dropped and
  not restored on save: Markdown tools do not need it and tracking it would complicate the API.

## Consequences
- Titles that differ only in punctuation (`a/b`, `a:b`) map to the same stem and get suffixes.
- Files edited and saved by Scratchpad lose a leading BOM.
- The app decides when to call `rename`; core only guarantees the result is valid and unique.
