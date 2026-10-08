# 0041. Block regions with lazy, incremental Markdown parsing
Date: 2026-10-08
Status: Accepted

## Context
Typing in a 10 MB note must stay well under the 16 ms frame budget (PLAN §21, §37, rules 5 and 8), so a
keystroke cannot reparse the document, and opening one should not parse text nobody looks at. CommonMark
blocks mostly end at blank lines, but fenced code blocks (and HTML comments) continue across them.

## Decision
- `MarkdownState` splits the buffer into block regions. A region starts at a non-blank line that has no leading
  whitespace and follows a blank line, unless a fence or `<!--` comment is open. A cheap line scanner tracks
  fences (after up to three spaces and any list markers, closed by a long enough run of the same byte) and
  comments. Each region is parsed on its own with pulldown-cmark (ADR 0040).
- Regions are parsed lazily, on the first query that touches them, and their decorations are stored relative to
  the region start, so moving a region costs one addition.
- `sync(buffer)` merges `Buffer::changes_since` into one replacement and rescans lines from the start of the
  region before the edit (an edit can join a region to the previous one) until it meets a region start that
  existed before the edit, at or after the edited text. The scanner's state at a region start is always the
  same, so all later regions are unchanged and are only shifted. Only rescanned regions lose their parse.
  Without a usable change log (or for another buffer) the whole document is rescanned, still without parsing.
- "Full parse" is defined as this region-wise parse. A property test applies random edits (including fences,
  blank lines, indentation, `**`, `#`, undo/redo and multi-edit transactions, with queries in between) and checks
  regions and decorations against a fresh `MarkdownState`.

## Consequences
- Per-keystroke cost is proportional to the edited region (usually one paragraph or list), plus shifting the
  starts of later regions; opening costs one line scan.
- Opening or closing a fence legitimately changes everything up to the next fence, which is rescanned (not
  parsed). A huge region (megabytes without a blank line, or an unclosed fence) is reparsed whole when it is
  visible after an edit.
- Constructs spanning a blank line followed by unindented text other than fences and comments (`<pre>`/`<script>`
  HTML blocks, a fence opened after a `>` marker) are split; each part is parsed as if alone.
