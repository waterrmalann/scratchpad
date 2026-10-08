# 0041. Block regions with lazy, incremental Markdown parsing
Date: 2026-10-08
Status: Accepted

## Context
Typing in a 10 MB note must stay well under the 16 ms frame budget (PLAN §21, §37, rules 5 and 8), so a
keystroke cannot reparse the document, and opening one should not parse text nobody looks at. CommonMark
blocks mostly end at blank lines, but fenced code blocks (and HTML comments) continue across them.

## Decision
- `MarkdownState` splits the buffer into block regions, each parsed on its own with pulldown-cmark (ADR 0040).
  A line scanner starts a region at an unindented line that begins a top-level block whatever precedes it: a
  non-blank line after a blank line, or an ATX heading or list item (they interrupt paragraphs and are never
  lazy), outside fences and comments (tracked with the list item they are in), and for headings and items
  outside a possible HTML block. A property test checks that parsing region by region gives the decorations of
  parsing the whole document. Where nesting cannot be told from lines (an indented fence in a list closed by an
  outdented one, a fence line in an HTML block), the scan stops starting regions.
- After 256 lines without a region start, any unindented line starts one: such runs are logs or pasted data, and
  this bounds the text a keystroke reparses. An emphasis, link, lazy quote line or setext heading spanning such
  a cut loses its style.
- Regions are parsed lazily, on the first query that touches them, and their decorations are stored relative to
  the region start, so moving a region costs one addition.
- `sync(buffer)` merges `Buffer::changes_since` into one replacement and rescans lines from the start of the
  region before the edit until it meets a region start that existed before the edit, at or after the edited
  text. Region starts depend only on the text since the previous start, so all later regions are unchanged and
  only shifted. Only rescanned regions lose their parse. Without a usable change log (or for another buffer)
  the whole document is rescanned, still without parsing. A property test checks incremental updates (edits,
  undo/redo, transactions, queries in between) against a fresh `MarkdownState`.

## Consequences
- Per-keystroke cost is the edited region (a paragraph, item or at most 256 lines) plus shifting later region
  starts. Measured (release, `cargo bench -p scratchpad-editor -- markdown`, loaded desktop): keystroke + sync +
  restyle of its line in a parsed 10 MB note, twice: 20–95 µs (in a 10 MB list of one-line items, one region
  per item: 0.2–0.5 ms per keystroke, a few ms when the region count first grows); region scan 10 MB 27 ms
  (1 MB 2.5 ms); styling 50 visible lines 36 µs. Parsing everything is ~35–45 ms per MB, so nothing parses
  regions that are not queried.
- Opening or closing a fence legitimately changes everything up to the next fence: an unclosed fence near the
  top of a 10 MB note makes each keystroke rescan to the end (~50 ms) until it is closed.
- Constructs spanning a blank line followed by unindented text other than fences and comments (`<pre>`/`<script>`
  HTML blocks) are split; each part is parsed as if alone.
