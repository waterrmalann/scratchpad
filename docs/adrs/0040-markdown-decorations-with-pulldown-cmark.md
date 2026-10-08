# 0040. Markdown decorations parsed with pulldown-cmark
Date: 2026-10-08
Status: Accepted

## Context
PLAN §17/§19 asks for Markdown parsed into decorations over buffer ranges, never replacing the text. Live
preview (§18/§20) must hide and reveal syntax markers, so each decoration needs its markers' exact ranges, not
only the element's. Hand-rolled inline parsing (emphasis delimiter runs, code spans, link brackets) is where
Markdown parsers get subtly wrong; malformed input must never panic.

## Decision
- Use `pulldown-cmark` 0.13 (`default-features = false`, no HTML renderer) with strikethrough and task lists
  enabled. Its offset iterator gives the source range of every element in CommonMark + GFM semantics.
- `Decoration { kind, range, content, markers }` in buffer `ByteOffset`s. Kinds: heading (ATX and setext, level
  1–6), strong, emphasis, strikethrough (`~` or `~~`), inline code, link (inline and autolink, with destination),
  block quote, list item (bullet/ordered), task box, thematic break, code block (fenced with info string, or
  indented without markers).
- Markers are derived from the text inside each element range: delimiter runs at both ends of inline spans,
  `#` runs plus spaces and the optional closing sequence, setext underlines, bullets plus their spaces, the `>`
  of each quote line (found after the enclosing quote's marker, so lazy lines have none), fence lines, `[` and
  `](destination)`. Markers are ASCII found by scanning inside the element, so offsets stay char boundaries.
- Block ranges end before their trailing line break, so "the block covers this line" is a simple range test.
- Reference links (`[text][ref]`, `[ref]`) are left as plain text: definitions may live in another block region
  (ADR 0041), and resolving them only sometimes would be worse than never. Images and raw HTML are plain text
  too; nothing is fetched or executed (PLAN §60). Tables, footnotes and math stay disabled.

## Consequences
- Decorations are ordered by start with enclosing ones first, are nested or disjoint, and lie within the
  document; a property test checks this on random malformed Markdown.
- Rendering only needs ranges; the parser never sees or produces anything but `&str` slices of one region.
- Reference-style links lose link styling until a document-wide definition index exists.
