# 0002. Rope buffer with byte offsets as the canonical coordinate
Date: 2026-10-08
Status: Accepted

## Context
The editor must stay responsive on 10 MB documents (PLAN §12, §37) and must never conflate byte, char,
grapheme, UTF-16 and visual positions (PLAN §13). GPUI shapes text per line and indexes shaped lines by UTF-8
byte offsets; platform IME APIs speak UTF-16.

## Decision
- Store text in `ropey::Rope` 1.x, wrapped in our own `Buffer`. Ropey is mature, O(log n) for edits and for
  byte/char/line/UTF-16 conversions. Its `unicode_lines`/`cr_lines` features are disabled because only `\n`
  separates lines in the buffer (ADR 0003); a test fails if feature unification ever re-enables them.
- `ByteOffset(usize)` into the buffer text is the canonical position. Selections, history, search results and
  change records all use it.
- `Point { line, column }` uses a **byte** column within the line, matching per-line shaping APIs.
  `Utf16Offset` exists only for IME conversion. Visual columns belong to the view.
- Cursor positions always lie on extended grapheme cluster boundaries. Boundaries are computed by feeding
  `unicode_segmentation::GraphemeCursor` rope chunks, so movement never copies a (possibly huge) line; multi-step
  walks reuse one cursor so long runs of regional indicators (flags) stay linear.
- `Buffer` is read-only outside the crate; only `Editor` mutates it so history, selection and the change log
  cannot drift apart.

## Consequences
- ropey stays an implementation detail: the public API speaks `ByteOffset`, `Point`, `&str` and `String` (e.g.
  `line_text`, `text_for_range`), so the rope could be swapped without touching the GUI crate.
- Every conversion clamps and rounds instead of panicking, so stale offsets from the UI are harmless.
