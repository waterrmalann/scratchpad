# 0188. Wikilinks to pages in the same folder
Date: 2026-11-11
Status: Accepted; amends ADR 0072 and 0185

## Context
Notes written in Obsidian link to each other with `[[Page]]`, `[[Page|label]]` and
`[[Page#Heading]]`. Scratchpad keeps notes as plain files in one folder (ADR 0013, 0050), so a
page name maps to a file in it. A link must never reach outside that folder, existence checks must
not touch the disk per frame or keystroke (ADR 0041, 0114), and a missing page should be one click
away, as in Obsidian.

## Decision
- **Parsing.** `MarkdownOptions::wikilinks` (default on) turns on pulldown-cmark 0.13's
  `ENABLE_WIKILINKS`. A wikilink is a new `DecorationKind::WikiLink { target }` rather than a
  `Link`: it opens a page, not a URL, and `link_at` and the URL check stay as they were. `target`
  is pulldown-cmark's raw text before the first `|`. Markers are `[[`, `target|` when there is a
  label, and `]]`, so live preview shows only the label or the target; they reveal like a link's
  (the cursor touching the link) and are `MarkerKind::Link`. The label may hold inline markup.
- **Which targets are links.** `markdown::wikilink_page` drops a `#heading` and the spaces around
  the page and accepts only a plain file stem: no `/ \ : * ? " < > |`, control characters or
  whitespace other than single spaces, no `..`, no leading or trailing dot or space, at most 100
  chars, and not a device name (`CON`, `nul.txt`, `COM0`, `COM1`...). These are the stems the
  store's `sanitize_file_stem` leaves unchanged (a test in `scratchpad/tests/wikilinks.rs` checks
  that the two agree); it is the only check of page names, which the app also asks before
  opening or creating a page. Anything else (paths, drives, URLs) is not decorated and
  stays text. Embeds (`![[x]]`) and same-note links (`[[#Heading]]`) stay text too. Headings are
  not navigated to.
- **Resolving.** `scratchpad_core::PageIndex` holds the folder's `.md` and `.txt` files (hidden
  files skipped) by stem, compared as NTFS compares names: each char folded through a
  single-char upper-case mapping (`fold_case`; a char whose upper case is several chars, like
  `ß`, or outside the BMP stays itself), not `str::to_lowercase`, which would tell `ΟΔΟΣ` from
  `οδοσ`. Per page it keeps the preferred file's name and which extensions exist, not paths;
  `resolve` joins the name to the folder (`.md` preferred). Only stems of files in the folder
  are found, so a path names nothing. The notes model keeps one for the notes folder in an
  `Arc`, collected in the same pass as each listing (`NoteStore::list_with_pages`; off the UI
  thread; listings follow the watcher's events) and updated by saves, new notes, renames,
  deletes and `forget` through `Arc::make_mut`. As the editor holds a clone, that would copy
  every page per change, so the index shares the map as listed and keeps later changes in a
  small map of their own, merged in once they outnumber 32 or a sixteenth of the pages. With
  Wikilinks off, listings skip the pages and none are kept; turning it on lists the folder
  again. A file from another folder (ADR 0145) resolves against its own folder, whose pages are
  read in the background when a file from there opens and again when the window becomes active
  if they are over 2 s old (that folder is not watched). The editor holds the index
  (`EditorView::set_pages`) and restyles when the `Arc` changes; `MarkdownState::
  styled_lines_with_pages` sets `SpanStyle::missing_page` on wikilinks whose page is not in it,
  asking only for lines that have links. Lookups are hash map hits; nothing touches the disk while
  drawing or typing. Until the folder is listed, no link is drawn as missing.
- **Drawing.** A link to an existing page looks like any link (text colour, accent underline);
  one to a missing page has muted text and a muted dashed underline.
- **Ctrl+click** emits `OpenPage(page)`; the app window handles it. A page in the index opens
  through `Notes::open_file`, as choosing it in the sidebar (a note of the folder) or File > Open
  (a `.txt`, or a file of another folder) would. Otherwise `Page.md` and `Page.txt` are looked up
  on the disk in the background, since the list may be out of date (on NTFS that ignores case
  too, without listing the folder), and if the page is still missing it is created
  there through `NoteStore::create` (the store's naming and collision rules, `create_new`) and
  written with `NoteStore::save` (atomic, ADR 0010) as `# Page`, so its title matches its name and
  editing its first line keeps the rules of ADR 0061; then it is listed (`note_saved`, ignored for
  other folders) and opened. Errors show the file error toast. While a page is being found or
  created, further clicks on it (by folded name) are ignored, so a slow folder never gets both
  `Page.md` and `Page 2.md`; of several different pages, only the last one clicked opens, and
  any created is listed.
- **Setting.** View > Markdown features > Wikilinks (`ToggleWikilinks`), remembered as
  `markdown_wikilinks` like the other options (ADR 0185).

## Consequences
- With the option on, pulldown-cmark reads some text differently, which the off state keeps:
  `[[a]](b)` is a wikilink followed by the text `(b)` rather than a link with the text `[a]`; a
  link whose text contains a wikilink (`[see [[A]]](u)`) is not a link; `**` that would pair
  across a wikilink's brackets does not; `[[x]]` is a wikilink even where `[x]` is a defined
  reference. Code spans, code blocks, autolinks and HTML still take precedence, and escaped
  brackets are text. Block structure and region boundaries do not change.
- `.txt` files appearing or vanishing in the notes folder are not watched (the watcher reports
  notes only), so their drawing can lag until the next listing; Ctrl+click checks the disk before
  creating, so it never makes a note next to a page that exists. Pages of another folder are
  not watched: they are read when a file there opens and when the window is activated, so a
  change made there while the window stays active shows on the next activation.
- A page name with a character the store would change (`[[a:b]]`) is not a link at all, rather
  than a link to a differently named file.
- Spell checking skips a wikilink without a label (it shows a page name) and checks a label.
- Tests: decorations and option off (`tests/markdown.rs`), reveal and missing-page styling
  (`tests/live_preview.rs`), random Markdown with wikilinks on (`tests/markdown_properties.rs`,
  `markdown::tests`), target validation (`markdown::wikilinks`), resolution and case folding (`core/tests/pages.rs`),
  and opening, creating (once per page), other folders and their re-reading, rejected targets,
  the agreement with the store's names and the toggle end to end
  (`scratchpad/tests/wikilinks.rs`, `menu_bar.rs`).
