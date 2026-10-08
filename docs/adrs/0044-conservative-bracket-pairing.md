# 0044. Conservative, stateless bracket and quote pairing
Date: 2026-10-08
Status: Accepted

## Context
PLAN §50 asks for minimal pairing of `()`, `[]`, `{}`, `""`, `''`, backticks and `**`, removed if it fights
the user. Prose is full of apostrophes, list bullets start with `*`, and code fences are three backticks.

## Decision
- `Editor::insert_typed(text)` is the entry point for keystrokes; `insert_text` stays raw for paste, IME and
  programmatic text. Multi-char text and input during IME composition are inserted verbatim.
- Auto-close an opener only when the next char is whitespace, a closing bracket, `.,;:!?` or the line end; for
  quotes and backticks also only when the previous char is not alphanumeric and not the same quote (so `don't`,
  `it's`, `say"` and ```` ``` ```` type normally). The pair is one undo step; plain typing keeps grouping.
- A closer typed before the same closer steps over it only if the cursor's line is balanced for that pair
  (equal counts, or an even number of the quote), i.e. the closer is not needed. This approximates "skip only
  auto-inserted closers" without tracking state that edits, undo and cursor moves would have to invalidate.
- `backspace_typed` deletes both halves of an empty pair around the cursor.
- With a selection, an opener (or `*`) wraps it and keeps it selected; typing `*` twice makes it bold.
- `*` is not auto-paired: it starts bullets and appears in arithmetic, so pairing it would fight the user.

## Consequences
- Stepping over and deleting pairs also applies to pairs the user typed by hand, which is what they would
  usually want; counting ignores code spans and escapes.
- Balance checks read the cursor's line, linear in its length, and only when a closer is typed before another.
