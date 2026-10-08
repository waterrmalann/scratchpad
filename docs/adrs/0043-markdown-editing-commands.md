# 0043. Markdown editing commands edit the text
Date: 2026-10-08
Status: Accepted

## Context
Formatting shortcuts and Enter must manipulate Markdown source, toggle cleanly and undo as one step (PLAN §36,
§49, rule 2).

## Decision
- `Editor::toggle_bold/italic/strikethrough/inline_code(&mut MarkdownState)` and `insert_link` are plain text
  edits inside one `Editor::transact`, so each is one undo step that restores the previous selection.
- Toggles behave like a word processor and use the parsed spans of their kind: if every visible character of the
  selection (whitespace and markers aside) is already formatted, the markers of the spans it overlaps are
  removed; otherwise the selection and the spans it overlaps or touches are merged into one span written with
  `**`, `*`, `~~` or a backtick. Whitespace at the selection's ends stays outside the markers.
- With a cursor: an empty pair around it (`**|**`) is removed, else the span it is in or right after is removed,
  else the word around it (word characters on both sides) is formatted, else a pair is inserted around it.
- Where no span is parsed, delimiter runs (same byte, `*`/`_` for emphasis) just outside the target, then just
  inside it, are removed, so stray or unparsable markers still toggle off. Runs are shared between kinds
  (`***x***`), so formatting with an `n`-byte marker counts as present when `run % 2n >= n`.
- The selection keeps covering the same text, with its direction preserved.
- `Editor::insert_markdown_newline(&mut MarkdownState)` (PLAN §49) replaces the selection, then reads the
  cursor line's prefix: quote markers, then an optional bullet/number with its spacing and task box. It inserts
  `\n` plus the same prefix (number + 1, task unchecked) and renumbers following items of the list only while
  they repeat the previous number (lists numbered `1.` throughout stay so). On an item without text it moves the
  item out to its parent item's list (by indentation), or at the top level clears the line and inserts `\n`, so
  the list ends with the blank line CommonMark needs before a paragraph (as PLAN §49 shows). Inside a code block
  (per the parse) it keeps the line's indentation and the markers of the quotes around the block, so code in
  list items and quotes stays inside them; before the end of the prefix or on `* * *` it inserts a plain `\n`.

## Consequences
- Toggling twice restores the text when no span of the kind touches the selection; merging is not undone by a
  second toggle (undo is). Underscore emphasis is removed but re-added as `*`. Both rules are property-tested.
- Delimiters the parser pairs differently than the user meant (stray `*` or backticks nearby) can still give
  surprising results; undo always restores the text exactly.
