# 0187. Bare web and e-mail addresses are links, found in text without changing it
Date: 2026-11-11
Status: Accepted; amends ADR 0040

## Context
Notes are full of pasted addresses (`https://...`, `www.example.com`, `name@example.com`) that nobody wraps
in `<...>`. ADR 0040 left them plain text; PLAN §47 says they "may later auto-link visually without
modifying underlying Markdown". GitHub renders them as links (GFM "extended autolinks"), so users expect
the same here. pulldown-cmark 0.13 has no such extension, and detection runs in every region parse, i.e.
on the keystroke path (ADR 0041).

## Decision
- **Where.** `markdown/autolink.rs` watches the events of each region's parse and scans only runs of
  consecutive `Text` events (merged, since entities and escapes split them) outside code blocks, links and
  images. Code spans, raw HTML, autolinks and link reference definitions are other events and are never
  scanned. Closed front matter (the `front_matter` rule of ADR 0170) is the note's, not a region's, so
  `MarkdownState::decorations` drops bare addresses that start in it; its end is computed only when an
  address lies within its 64 KB reach, once per buffer version.
- **Rules.** GFM's, as cmark-gfm implements them: `http://` or `https://` (any case; the scheme is all the
  letters before `://`) then a domain, which needs no period; `www.` at the start, after whitespace or
  after `*`, `_`, `~` or `(`, then a domain with a period; `local@domain` with an ASCII local part of
  alphanumerics and `.-_+` and a domain of alphanumeric, `-`, `_` labels with a period, not ending in `-`
  or `_`. A domain is segments of alphanumerics (any script), `-` and `_` separated by single periods,
  with no `_` in the last two. The address runs to whitespace or `<`, then drops trailing `?!.,:*_~'"`,
  an entity-like `&name;` and every `)` beyond the `(`s inside it. `ftp://` and other schemes are not
  detected: nothing would open them (ADR 0072).
- **Decorations.** Each address is a `DecorationKind::Link` whose range and content are the address and
  whose markers are empty, so live preview styles it and hides nothing. The destination is made
  openable: `https://` before `www.`, `mailto:` before an e-mail address, a URL as written. `link_at`,
  Ctrl+click and the pointing hand work unchanged; spell checking skips a link without markers whole.
- **Cost.** A text run with no `:`, `@` or `www.` costs three std byte searches (`memchr`-backed for
  single bytes) and allocates nothing; only a match allocates (its destination). No new dependency.
- **Shared rule.** `autolink::looks_like_web_address` (`://`, `@` or a leading `www.`) is what typed
  replacements (ADR 0180) use to leave an address being typed alone, so both agree on what an address
  starts with.

## Consequences
- Addresses are found from the source text, so `&amp;` in an address stays as written in its destination.
- The text before `www.` is checked in the region text, so `**www.x.com**` links and `'www.x.com'` does not,
  as on GitHub. Table cells, once enabled, start after `|`, which does not allow `www.` right after it.
- There is no option to turn it off; it can join `MarkdownOptions` if someone asks.
- The region property test intermittently found a mismatch (from ADR 0185) between pulldown-cmark and the
  region scanner: pulldown-cmark continues a GFM alert across a whitespace-only line indented 4 or more columns. Such
  a line no longer separates regions, which only removes cuts.
- Tests: the spec's examples and exclusions (`tests/autolinks.rs`), region and edit equivalence
  (`markdown::autolink::tests`), the well-formedness properties (`tests/markdown_properties.rs`),
  Ctrl+click end to end (`scratchpad/tests/live_preview.rs`). `cargo bench -- markdown_parse_prose`
  measures 1 MB of prose with and without addresses.
