# 0044. Conservative, stateless bracket and quote pairing
Date: 2026-10-08
Status: Accepted; amended by 0180

## Context
PLAN §50 asks for minimal pairing of `()`, `[]`, `{}`, `""`, `''`, backticks and `**`, removed if it fights
the user. Prose is full of apostrophes, list bullets start with `*`, and code fences are three backticks.

## Decision
- `Editor::insert_typed(text, markdown)` is the entry point for keystrokes in Markdown; `insert_text` stays
  raw for paste, IME, programmatic text and plain text files. Multi-char text and input during IME
  composition are inserted verbatim.
- Auto-close an opener only when the next char is whitespace, a closing bracket, `.,;:!?` or the line end; for
  quotes and backticks also only when the previous char is not alphanumeric and not the same quote (so `don't`,
  `it's`, `say"` and ```` ``` ```` type normally). The pair is one undo step; plain typing keeps grouping.
- A closer typed before the same closer steps over it only if the cursor's line is balanced for that pair
  (equal counts, or an even number of the quote), i.e. the closer is not needed. This approximates "skip only
  auto-inserted closers" without tracking state that edits, undo and cursor moves would have to invalidate.
- `backspace_typed` deletes both halves of an empty pair around the cursor. (Amended by ADR 0180: right after
  a typed replacement it first takes that back, and `insert_typed` also replaces sequences such as `->`.)
- With a selection, an opener, `*` or `~` wraps it and keeps it selected; typing `*` twice makes it bold,
  `~` twice strikes it through (`~~text~~`). Each wrap is one undo step.
- `_` over a selection wraps it only where that makes emphasis, and otherwise replaces the selection as plain
  typing does. It wraps when the selection holds a letter or digit, no `_` and no whitespace at either end, is
  not in code, and has as many `_` right before it as right after (none, or the ones a previous `_` added, so
  `_` twice makes `__strong__`), with no letter or digit beyond them. A letter followed by combining marks
  counts as a letter. `_` inside a word is not emphasis in CommonMark, and selecting a space, a `-`, part of a
  word or an identifier such as `foo_bar` and typing `_` is usually meant as a replacement (`foo bar` to
  `foo_bar`, `for i in` to `for _ in`).
- "In code" is `MarkdownState::in_code(buffer, range)`: the range overlaps the text of an inline code span
  or a code block, or, if empty, lies in that text or at either end of it (between a span's backticks). So
  `insert_typed` takes the document's `MarkdownState`, as `insert_markdown_newline` does. Typing rules that
  write Markdown or rewrite what is typed (such as typed replacements) use it to leave code alone.
- Brackets, quotes, `*` and `~` still wrap in code: wrapping `x` in parentheses or quotes is as useful in code
  as in prose, and `*` or `~` typed over a selection in code is rarely meant as a replacement, unlike `_`
  over a name. Plain text files (ADR 0147) do not pair or wrap at all.
- `*`, `~` and `_` are not auto-paired without a selection: `*` starts bullets and appears in arithmetic, `~`
  means "about" and `_` joins words, so pairing them would fight the user.

## Consequences
- Stepping over and deleting pairs also applies to pairs the user typed by hand, which is what they would
  usually want; counting ignores code spans and escapes.
- Balance checks read the cursor's line, linear in its length, and only when a closer is typed before another.
