# 0190. Highlights, math and emoji shortcodes; View > Markdown Extensions
Date: 2026-11-12
Status: Accepted; amends ADR 0040, 0150, 0170, 0180, 0185, 0187 and 0188

## Context
Notes written in Obsidian and on GitHub use `==highlights==`, `$math$` and `:smile:` shortcodes. pulldown-cmark
0.13 knows math (`ENABLE_MATH`) but not highlights or shortcodes. All three must keep the rules of live preview
(the buffer is never rewritten, decorations are byte ranges, markers hide away from the cursor; ADR 0042),
lazy per-region parsing (ADR 0041) and the keystroke budget (ADR 0114). `$` is a dollar in most prose, so math
must be optional; the other two are rarely typed by accident. The submenu of ADR 0185 was named
"Markdown features"; it is renamed as requested.

## Decision
- **Menu.** View > Markdown Extensions (the user's title case; the other View items stay sentence case) holds
  Callouts, Superscript, Subscript, Heading IDs, Wikilinks, then Highlights, Math and Emoji shortcodes (actions
  `ToggleHighlights`, `ToggleMath`, `ToggleEmojiShortcodes`, no keys). `MarkdownOptions` gets `highlights` (on),
  `math` (off) and `emoji_shortcodes` (on); the config remembers them as `markdown_highlights`,
  `markdown_math` and `markdown_emoji_shortcodes` (missing means the default), like the others (ADR 0185).
- **One list per layer.** The options are declared once in the editor (`markdown_options!`: field, the
  `MarkdownOption` variant, menu label, default), which generates the struct, `ALL`, `Default`, `get`/`set`
  and what the property generators draw from; the app pairs each option with its toggle action once
  (`actions::view::markdown_toggles!`, from which the submenu and the window's handlers are built) and with
  its config key once (`settings`). The config's serde fields stay written out, so the file format is as
  before. Only options that differ from their default are written (amends ADR 0185, which wrote all of
  them): a default changed later reaches everyone who did not set that option. Configs that stored every
  value read the same; values equal to the default are dropped on the next write.
- **Highlights** (`DecorationKind::Highlight`, markers the two `==`, `MarkerKind::Emphasis`) are found by
  `markdown/highlight.rs` in the text runs of one walk over each region's events that it shares with the
  bare-address scanner (`markdown/text_runs.rs`: runs of consecutive `Text` events, the links and elements
  around them; both scanners consume the same steps). `==` pairs like an emphasis delimiter run of `_`: a run of
  exactly two unescaped `=` opens if left-flanking and closes if right-flanking (CommonMark's definitions; chars
  other than letters, digits and whitespace count as punctuation), but a run flanked on both sides opens only
  after punctuation and closes only before it, so comparisons such as `x==1 and y==2` or `if a==b then c==d`
  stay text. It closes the nearest opener of the same inline element. So a
  highlight may enclose emphasis, links, code or math (`==a *b* c==`) but never crosses an element's edge
  (`*a ==b* c==`), nor a block's. `=` and `===` runs are text; setext underlines and front matter (dropped at
  query time like bare addresses) are not text. Code spans, code blocks, math, HTML, link destinations,
  autolinks, wikilinks and bare addresses are never scanned. The view draws the theme's `highlight` background
  (a pale yellow, a dark olive) behind the content; text keeps its colour, at 7:1 or more (unit-tested).
- **Math** passes `ENABLE_MATH`. `InlineMath`/`DisplayMath` become `DecorationKind::Math { display }` with `$`
  or `$$` markers (`MarkerKind::Math`). Inline markers reveal like emphasis; display markers while the selection
  is on any of the block's lines, like a fence. Nothing is typeset: the content is drawn in Cambria Math
  (Cambria, else the body font; picked from the installed fonts at startup, as the code font is, since GPUI
  does not walk a fallback list) in the theme's `math` colour (4.5:1 on page, surface and highlight).
  pulldown-cmark parses math inside a paragraph, so display math spans lines but not a blank line, and the
  region scanner, which only cuts where a paragraph ends, never splits it (except the 256-line cut of ADR 0041,
  which already splits long paragraphs). Checked in pulldown-cmark 0.13.4's source, `ENABLE_MATH` changes
  nothing else: it makes `$` a special byte (splitting text events; the scanners merge adjacent ones) and tracks
  `{`/`}` for math's brace balance (`$}$` is no math), and math spans take precedence over emphasis and links,
  as code spans do (`*a $b*$` has no emphasis), so an address is cut at a `$` that opens math. A property test
  checks that decorations of text without `$` are identical with Math on and off. pulldown-cmark reports a
  wrong range for math starting in a wikilink's target (`[[$$|a|$]]` gives `a|$`), so math is kept only where
  its range starts and ends with its dollars.
- **Emoji shortcodes** (`DecorationKind::Emoji { emoji }`, one marker: the whole `:name:`; empty content) are
  found in the same scan: `:` + gemoji name chars (`a-z0-9_+-`) + `:`, a known name, no letter or digit before
  or after it (so `10:30:45` and `a:b:c` stay text), not escaped, outside bare addresses and the text of
  inline links, images, autolinks and wikilinks. Reference links are text (below), so `[:smile:][r]` shows the
  emoji whether or not `r` is defined; skipping them would need to know which brackets would resolve, which a
  region cannot.
  Unknown names stay text. The marker is never hidden; while the selection is off it (live preview only), the
  styled span carries `MarkerKind::Emoji { emoji }` and the view draws the emoji in its place, in Segoe UI
  Emoji with `paint_emoji` for colour glyphs, as the callout label is drawn (ADR 0185): the shortcode's glyphs
  are transparent and take the emoji's width, clicks inside it land at one of its ends, and touching it shows
  the shortcode as typed (ADR 0071, 0125). Each shortcode is its own span, also next to the same one; table
  cells measure it as drawn.
- **Data.** The `emojis` crate 0.9.1 (`(MIT OR Apache-2.0) AND Unicode-3.0`, both compatible with this MIT
  repo; gemoji's 1,913 shortcodes, looked up through a compile-time `phf` map) was measured: linked into the app
  it added 539 KB (8,490,496 → 9,042,432 bytes, release), because its table carries every emoji's metadata. So
  it is a dev-dependency only: `markdown/emoji_table.rs` is generated from it (1,913 sorted `(name, emoji)`
  pairs, binary-searched, no allocation), and a unit test fails if the table differs from the crate's data,
  rewriting it with `SCRATCHPAD_WRITE_EMOJI_TABLE=1`. The dev-dependency is pinned (`=0.9.1`), so
  `cargo update` cannot fail that test: bumping it is a deliberate step that regenerates the table and reviews
  its diff.
- **Reference links are seen through.** The region property test found that a reference definition
  (`[!NOTE]:smile:` is one) in a later region turns `[!NOTE]` earlier into a link in the whole document only,
  which cut a bare address there. Reference links stay text (ADR 0040), so the shared walk reads their
  brackets and text as text and does not treat them as an element: what the scanners find no longer depends on
  a definition anywhere. An address then ends at a `]` it did not open (as an unmatched `)` is trimmed), so
  `[see www.a.com][r]` links `www.a.com`, defined or not (amends ADR 0187).
- **Typing and spelling.** Typed replacements skip math as they skip code (`MarkdownState::in_code_or_math`),
  and with Math on also math still being typed (`replacements::in_math_span`: a `$` or `$$` that can open, with
  no run as long after it that can close, outside code spans), so `$a -> b$` keeps its arrow (amends ADR 0180).
  Spell checking skips math and shortcodes whole and checks highlighted text without its `==` (amends
  ADR 0170).
- **Cost.** One more pass over each text run, only while Highlights or Emoji is on; a run without `=` or `:`
  returns after it and allocates nothing (no state is allocated until an opener is found). Shortcode lookup is a
  binary search of a static table.

## Consequences
- Benchmarks (release, `cargo bench -p scratchpad-editor -- markdown`, the old and new bench binaries run back
  to back on the same desktop; default options, so highlights and emoji on): parsing 1 MB of prose
  12.6 → 13.0 ms, 1 MB of prose with addresses (which has `:`) 16.3 → 17.9 ms, the mixed 1 MB document
  31.0 → 32.8 ms; two keystrokes in 10 MB 36 → 38 µs, styling 50 lines 29 → 30 µs, region scan unchanged.
  New, every option on: `markdown_parse_extensions_1MB/none` 13.2 ms, `/many` (highlights, math, shortcodes and
  lookalikes in every paragraph) 30.4 ms, styling 50 such lines 95 µs. The shared walk changed nothing
  measurable (same runs, before → after: 1 MB mixed 30.9 → 29.9 ms, prose 12.5 → 12.5 ms, extensions
  none 12.4 → 12.3 ms, many 28.8 → 28.7 ms, two keystrokes 37.9 → 36.6 µs, styling 50 lines 28.5 → 29.7 µs).
- The release binary grows by 113 KB (8,490,496 → 8,606,208 bytes), the table and the new code.
- With Math on, `$` pairs as pulldown-cmark pairs it: `$5 and $6` is text (a closing `$` needs a non-space before
  it), but `$5 and 6$` is math. And typed replacements wait after an opening `$` on the line until it closes.
- Highlights follow the emphasis rules of `_`, so `a == b == c` and `x==y==z` are text, `==a==` and
  `a ==b==, c` highlights.
- A shortcode is drawn as the emoji GitHub shows; whether it is coloured depends on Segoe UI Emoji.
- Tests: decorations, exclusions and reveal (`tests/extensions.rs`), the random-Markdown invariants and
  region/whole and incremental parity with every option in the generators (`tests/markdown_properties.rs`,
  `markdown::tests`), the table's freshness (`markdown::emoji`), typed replacements and spell checking, the
  colours' contrast, the emoji width and click mapping end to end (`scratchpad/tests/live_preview.rs`) and the
  submenu and its persistence across a restart (`scratchpad/tests/menu_bar.rs`).
