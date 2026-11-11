# 0185. Optional Markdown syntax: callouts, superscript, subscript, heading IDs; images styled as links
Date: 2026-11-11
Status: Accepted; amends ADR 0040 and 0150; amended by 0188, 0190

## Context
Notes copied from GitHub and Obsidian use GFM alerts (`> [!NOTE]`), `^sup^`, `~sub~` and heading
attribute blocks (`# Title {#id}`), and images (`![alt](src)`). ADR 0040 left images as plain text
and enabled only strikethrough and task lists. Some of these extensions change the meaning of text
that is ordinary Markdown elsewhere (a single `~` is GFM strikethrough; `{...}` at the end of a
heading is text), so users need a way to turn them off, and later phases will add more. Parsing
must stay lazy per block region (ADR 0041) and cost nothing per keystroke.

## Decision
- **Options.** `scratchpad_editor::markdown::MarkdownOptions { callouts, superscript, subscript,
  heading_ids }`, a `Copy` struct of flags, one per extension; `Default` has callouts on and the
  others off, `MarkdownOptions::ALL` has all on. `MarkdownState` holds them
  (`with_options`, `options`, `set_options`) and maps them to pulldown-cmark's `Options` for each
  region it parses. `set_options` does nothing if the options are equal; otherwise it drops every
  region's cached parse and parses nothing itself: regions are reparsed when next queried, which in
  the view is the visible lines. Region boundaries do not depend on the options; the region and
  incremental property tests run with random options, all on in a third of the cases, including
  option changes between edits.
- **Setting.** View > Markdown features (renamed Markdown Extensions by ADR 0190), a submenu of ticked items: Callouts, Superscript,
  Subscript, Heading IDs (actions `ToggleCallouts`, `ToggleSuperscript`, `ToggleSubscript`,
  `ToggleHeadingIds`, no keys). The editor view holds the options (`EditorView::markdown_options`,
  `set_markdown_options`), carries them to every note it opens, and on a change re-checks the
  visible lines' styles and spelling. The config remembers each as `markdown_callouts`,
  `markdown_superscript`, `markdown_subscript`, `markdown_heading_ids` (missing means the default),
  written like the other View settings.
- **Images** are inline-only `Tag::Image` decorations of kind `Link`, so they look, hide their
  markers (`![` and `](src)`) and reveal exactly like links. Reference images stay plain text, for
  the reason reference links do (ADR 0040). Ctrl+click passes the source to the existing check:
  only http, https and mailto open, in the browser. Nothing is fetched, decoded or shown.
- **Callouts** use `ENABLE_GFM`, which in pulldown-cmark 0.13 adds nothing but the alert tags of
  block quotes (checked in its source; re-check on upgrade). A quote whose first line is only
  `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]` or `[!CAUTION]` (any case, nothing after it, as
  on GitHub) gets a `Callout { kind }` decoration with the quote's range right after the quote's own
  decoration; its one marker is the tag to the end of its line. Every line of the quote carries the
  kind in its line style. The tag is never hidden, like a bullet: the view draws it as the kind's
  label (`Note`, ...) in semibold, in the kind's accent, while the selection is off the tag's line,
  and as typed while it is on it. The quote's bar takes the accent and its text the normal text
  colour. Accents are new theme tokens (`Theme::callouts`), GitHub's hues adjusted to 4.5:1 on
  the page and surface in both themes (unit-tested).
- **Superscript and subscript** are inline decorations whose `^`/`~` markers hide and reveal like
  emphasis markers (`MarkerKind::Emphasis`). The view sets them at 0.75 × the size, superscript
  raised by 0.33 em and subscript lowered by 0.15 em: a run's baseline shift is part of its shaped
  segment. With subscript on, a single `~` is subscript and `~~` strikethrough; with it off, `~` is
  strikethrough as before. A tilde run only closes a run as long, so the strikethrough marker is
  the run at its start in both cases. For `^`, pulldown-cmark nests a run of carets with two per
  element and one left for the outermost, so the marker is 1 caret for an odd run, else 2.
- **Heading IDs** are found by the parser, not by pulldown-cmark: `ENABLE_HEADING_ATTRIBUTES` is
  never passed, because pulldown-cmark 0.13.4 (and 0.13.6, the latest checked) panics on some input
  with it (`- a`, `    {#a}`, `  - `: a slice out of range), and with `panic = "abort"` that would
  close the app on a keystroke. The trailing `{...}` block, with the whitespace before it, is found
  with pulldown-cmark's own rule (last non-blank byte `}`, nearest `{`, `}`, `<`, `>`, `\` or line
  break before it a `{`) and becomes the heading's last marker, after a closing `#` sequence, or the
  setext text line's marker before the underline; a block on a line of its own leaves the text before
  it, without its line break, as the content. It is not part of the content, hides with the
  heading's other markers and shows while the cursor is on the heading's lines. Since pulldown-cmark
  now parses the block as heading text, inline decorations reaching into it are dropped (`# *a {*}`
  has no emphasis) and no address is looked for in it; what ends before it pairs as it did with
  the parser's attributes on, because delimiters pair from the left (checked against the old output
  on 200,000 random headings; the only difference, an address before an escaped `\{`, no longer
  takes the backslash).

## Consequences
- Toggling an option costs dropping the cached decorations and reparsing the visible regions; no
  keystroke does more work than before, and no memory is added per region.
- As pulldown-cmark reads them, `^` and a single `~` cannot open inside a word (like `_`): `x^2^`
  and `H~2~O` stay text, and the marker needs a space or punctuation before it (`x ^2^`). Changing
  that would mean patching the parser, which this decision does not do.
- `{}` and `{anything}` at the end of a heading line are attributes once Heading IDs is on, as
  pulldown-cmark reads them; that is why it is off by default.
- A pulldown-cmark upgrade that fixes the panic would let the parser read attributes again; the
  regression inputs (`tests/markdown.rs`) and the heading property test
  (`tests/markdown_properties.rs`, 8,192 cases) guard either way.
- A callout's tag line with text after the tag (`> [!NOTE] Title`) is an ordinary quote, as on
  GitHub.
- Tests: decoration ranges of each feature and of the options turned off (`tests/markdown.rs`),
  reveal and line styles (`tests/live_preview.rs`), the random-Markdown invariants with random and
  all-on options (`tests/markdown_properties.rs`, `markdown::tests`), the view's sizes, label and
  link opening end to end, and the submenu and its persistence across a restart.
