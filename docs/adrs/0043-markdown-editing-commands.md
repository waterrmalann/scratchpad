# 0043. Markdown editing commands edit the text
Date: 2026-10-08
Status: Accepted

## Context
Formatting shortcuts must manipulate Markdown source, toggle cleanly and undo as one step (PLAN §36, rule 2).

## Decision
- `Editor::toggle_bold/italic/strikethrough/inline_code` and `insert_link` are plain text edits inside one
  `Editor::transact`, so each is one undo step that restores the previous selection.
- Target: the selection; with a cursor, an empty pair right around it (`**|**`), else the word around it (word
  characters on both sides), else the cursor itself, where the pair is inserted with the cursor in between.
- Toggling looks at delimiter runs (same byte, `*`/`_` for emphasis) just outside the target, then just inside
  it. Runs are shared between kinds (`***x***` is strong and emphasis), so formatting with an `n`-byte marker
  counts as present when `run % 2n >= n`: adding or removing `n` bytes always flips that, which makes toggling
  its own inverse. Toggling on always writes `**`, `*`, `~~` or a backtick.
- The selection keeps covering the same text (inside the new markers, or what remains after removing them),
  with its direction preserved.

## Consequences
- Toggling twice restores the text (property-tested) unless the selection itself starts or ends with
  delimiters, which the first toggle removes; underscore emphasis is removed but re-added as `*`.
- No parse is needed, so the commands work identically in code blocks; the user asked for them explicitly.
