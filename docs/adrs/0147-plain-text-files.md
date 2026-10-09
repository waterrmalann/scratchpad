# 0147. Files not named like Markdown are edited as plain text
Date: 2026-10-09
Status: Accepted

## Context
File > Open (ADR 0145) brings `.txt`, `.log`, `.ini` and other files into an editor built for
Markdown. Styling `# include` in an `.ini` as a heading, hiding `**` in a log, continuing
`- ` lists or pairing brackets would misrepresent and alter files that are not Markdown.

## Decision
- `EditorView::set_markdown(bool)` switches the view. In plain text every line is one unstyled
  span (`LineKey::plain`): no Markdown styles, no hidden markers, no bullets or task boxes drawn
  as shapes. Enter inserts a line break, Tab four spaces, typed brackets and Backspace act on
  single characters, and formatting shortcuts, Shift+Tab, task box clicks and Ctrl+click on
  links do nothing.
- Notes and new notes are always Markdown. External documents are Markdown when their
  extension is `.md` or `.markdown` (any case), plain text otherwise; the mode follows the file
  when Save As changes it. The engine and its Markdown state are unchanged: the view only stops
  asking for styles, so switching back restyles from the current text.

## Consequences
- Plain text uses the same body font and wrapping as notes; there is no monospace mode.
- A Markdown file with another extension (e.g. `.mdx`) opens as plain text.
