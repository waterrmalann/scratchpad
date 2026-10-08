# 0003. Normalize line endings to LF in the buffer
Date: 2026-10-08
Status: Accepted

## Context
Scratchpad is Windows-first, so notes created elsewhere may use CRLF, LF, or a mix, and old files may contain
lone CR. Keeping `\r` in the buffer would make every line computation, cursor motion and Markdown rule handle
two-byte line breaks, and `\r\n` is a single grapheme cluster, so a "line end" before `\n` would not even be a
valid cursor position. Saving a note must not silently rewrite a file's line endings.

## Decision
- On load (`Buffer::from_text`), detect the dominant line ending: CRLF if CRLF line breaks strictly outnumber
  all other line breaks, otherwise LF (also for text without line breaks). Then convert `\r\n` and lone `\r`
  to `\n`. The buffer never contains `\r`.
- All inserted text (typing, paste, IME, programmatic replacements) is normalized the same way.
- On save (`Buffer::to_text`), every `\n` is written as the detected line ending.
- Contract, enforced by property tests:
  - text whose line breaks are all LF, or all CRLF, round-trips byte-for-byte;
  - any other text is saved with every line break (`\r\n`, `\r`, `\n`) replaced by the dominant ending and
    nothing else changed; loading the saved text again is stable.

## Consequences
- Mixed-ending and lone-CR files are rewritten with uniform endings on their first save. This matches what
  most editors do and is the price of a simple, `\n`-only engine.
- The app owns OS clipboard conversion: `copy`/`cut` return `\n` text.
- New documents default to LF. Choosing CRLF for new notes would need a small setter; none exists until a
  feature asks for it.
