# 0186. GFM tables: decorations, table-wide reveal and an aligned grid
Date: 2026-11-11
Status: Accepted; amends ADR 0040, 0041 and 0042

## Context
Notes copied from GitHub, Obsidian and chat tools often hold GFM tables, which ADR 0040 left as plain
text: a wall of pipes and dashes. Live preview should show them as a grid, as Obsidian does, without
replacing any text (PLAN §17–20), with the caret, clicks and selections still mapping to buffer
offsets (ADR 0071, 0125). A table's column widths depend on every one of its rows, which line-by-line
layout (ADR 0071) never needed before, and typing in a 200-row table must stay well inside a frame.

## Decision
- **Parsing.** `ENABLE_TABLES` is always on; it is not a [`MarkdownOptions`] toggle, since a table is
  plain GFM and its syntax is unambiguous enough not to surprise anyone. Decorations (in
  `markdown/table.rs`): `Table { alignments }` (the whole table, no markers), `TableHeader` and
  `TableRow` (one row each; markers: the leading pipe, the pipes between cells, and the trailing pipe
  together with anything after it on the row, which takes the cells past the header's count that GFM
  drops), `TableDelimiter` (the `|:--|--:|` row from after any `>`/indentation prefix to its last
  non-blank byte, which is its one marker) and `TableCell { column, alignment }` (the text between two
  pipes; its content is that text without the spaces around it). pulldown-cmark gives the cell ranges;
  the pipes are the bytes next to them, so `\|` and pipes in code spans are exactly what GFM says they
  are (`\|` is cell text; an unescaped `|` in a code span splits the cell). Short rows get no padding
  cells. Inline formatting inside cells is decorated as anywhere else.
- **Styled lines.** `SpanStyle` gets `table: Option<TableRowKind>` (header, delimiter, body; also on
  the line style) and `table_column`. `MarkerKind` gets `TablePipe` and `TableDelimiter`.
  `MarkdownState::table(buffer, line, selection)` returns the table of a line (range, lines,
  alignments, whether the selection reveals it). The alignments are an `Arc<[TableAlignment]>`
  shared by the `Table` decoration and every query, so a query copies a pointer.
- **Queries per line.** A table is one region however long, so a query must not look at every
  decoration of its region: each region keeps, beside its sorted decorations, the nearest enclosing
  decoration of each (they are nested or disjoint, which the parse's property tests check; if not,
  the region falls back to a scan). A query for a range takes the decorations starting in it with
  `partition_point` and those starting before it that reach it by walking up from the last one
  starting before it, as deep as the nesting (table, row, cell, inline span). Styling a line or
  looking up its table then costs the same in a 10,000-row table as in a 20-row one. A property
  test checks every query against the whole document's decorations.
- **Wikilinks in cells.** `|` splits cells, so Obsidian writes a wikilink's label separator as `\|`
  in tables: `[[a\|b]]` links to `a` with label `b`, its marker `a\|` (accepted outside tables too,
  as in Obsidian; ADR 0188).
- **Table-wide reveal.** Every table marker shows while the selection touches any line of the table,
  as for code blocks and headings, rather than only on the cursor's row. Column widths come from all
  rows: with row-wide reveal, typing in one row (shown as typed) would change the widths of the grid
  the other rows are drawn in, so each keystroke would re-measure the whole table, or the grid would
  show stale widths. Showing the whole table as typed also keeps Up/Down between rows in the columns
  the user sees and typed, and costs nothing per keystroke: no grid is laid out while the cursor is in
  the table. Inline markers inside cells keep their own rule (shown where the selection touches them).
- **Regions.** The region scanner (ADR 0041) does not start a region at a delimiter row that follows a
  non-blank line, though `- | -` looks like a list item, and never cuts a table after
  `MAX_REGION_LINES`: a table runs until a blank line or a block that interrupts it, and each of those
  starts a region anyway. A table is therefore always one region (a property test checks both
  splitting and incremental updates with tables in the random documents).
- **The grid** (`editor_view/table.rs`). With the selection off a table, its pipes and delimiter row
  are hidden like any marker. `KeyContext::key`, which builds every line's `LineKey` and applies
  all reveal rules, asks `TableCache::layout` how a table line is laid out: as a grid, with a
  `GridKey` (row kind and an `Arc` of the table's `Columns`) in the key, or as typed, with the
  table's markers shown. `LineLayout::shape` then moves each cell's text to
  its column by widening the room before it (`set_span_width`), aligned left, centre or right in the
  column's width, with 0.6 em of padding each side; the row is made as wide as the grid. Nothing is
  inserted into the text, so `StyledLine::display_column`/`buffer_column` map clicks, Ctrl+clicks and
  IME bounds as for any line with hidden markers; a click puts the caret in the clicked cell, which
  then reveals the table (ADR 0071's "the reveal that follows may shift the text"). Header cells are
  bold (also as typed) on a `surface` background; the delimiter row is collapsed to a 0.5 em row with
  a `border` rule across the table; 1 px `border` lines separate the columns. Both are theme tokens
  with light and dark values. A table in a list item has its continuation rows' leading spaces
  widened like a nested item's, so they line up under the header that follows the bullet.
- **Widths are measured once per table** and kept in the layout cache's `TableCache`, by the table's
  lines and the Markdown options: the first grid line laid out styles the table's lines in chunks of
  64 (so each query copies only those lines' decorations), shapes each row once and takes each
  column's widest cell. The entry is dropped when a buffer change touches the table's lines,
  renumbered when lines are added or removed above it, and dropped when the fonts or the text size
  change (zoom); a new wrap width, theme or margin keeps it, since only the comparison of the grid's
  reach with the wrap width depends on them, and that is made on every lookup. An edit on the line
  right above or below a table (a heading touching it, a row being typed after it) keeps its
  widths: lookups match the table's exact lines, so a table that such an edit grows or shrinks is
  measured again anyway. At most 16 tables are kept (least recently used dropped). A key compares
  its columns by pointer first, so checking a grid line's key costs no more than another line's.
- **Long tables.** A table longer than 500 lines (`MAX_GRID_LINES`) is shown as typed, like a wide
  one. Measuring shapes every row on the UI thread in one frame: a few milliseconds for 500 rows,
  but a pasted 5,000-row table would stall the frame it is opened in, the one the cursor leaves it
  in and every zoom. Measuring only the rows in view, widening the columns as rows scroll in, would
  bound the cost too, but the columns would jump while scrolling, and the widest row could still be
  anywhere; a table that long is rare in notes and reads as well as typed.
- **Wide tables.** A grid row is one screen row and never wraps. With word wrap off it scrolls
  sideways like any long line (ADR 0132). With word wrap on, a table whose grid would reach past the
  text column is shown as typed instead (its markers shown, rows wrapped like text), so no cell is
  ever cut off or unreachable; wrapping text inside cells would need multi-row cells in a geometry
  built for one wrapped line, which costs far more for a rare case.

## Consequences
- Typing in a table reparses its region and re-styles the visible lines, as in any paragraph of that
  size; it never measures the grid. Moving the cursor out of a table measures it once (one shaping
  per row) and re-shapes its visible lines; moving it in re-shapes them as typed. Measured (release,
  `cargo bench -p scratchpad-editor -- table`): a keystroke in the middle of a 200-row table plus
  restyling 50 of its lines and querying their table: ~0.58 ms; styling all 200 rows for measuring:
  ~0.6 ms, once (`docs/performance.md`).
- By table length (release, `cargo bench -p scratchpad-editor -- markdown_table_rows`, loaded
  desktop, 2026-11-12; before and after the bounded queries): a keystroke in the middle row plus
  styling 50 of its lines one at a time and looking up their table: 200 rows 0.44 / 0.45 ms,
  2,000 rows 3.7 / 3.4 ms, 10,000 rows 21.6 / 18.9 ms; the 50 lines' styling and lookups alone:
  0.22 / 0.20 ms, 0.53 / 0.20 ms, 5.2 / 0.19 ms. What is left grows with the table only through
  reparsing its region.
- Tables longer than 256 lines stay one region, so a keystroke in a 10,000-row pasted table reparses
  all of it (~19 ms per 470 KB) and misses a frame. Splitting a long table into regions would need
  each later part parsed with the header and delimiter row of the first, so an edit there would have
  to drop the parse of every part, and regions would no longer depend only on their own text (ADR
  0041); capping it (rows past a limit shown as text) would change what the note means at an
  arbitrary row. Both cost more, in correctness and code, than a slow keystroke in a table nobody
  types into by hand; tables up to a few thousand rows stay within a frame.
- Rows typed with fewer cells than the header leave their last columns empty in the grid; cells past
  the header's count are hidden with the trailing pipe, as GitHub drops them, so nothing is drawn
  past the grid's right edge.
- With a 400-row table under a heading (release, test platform, whose shaping is cheaper than
  DirectWrite's): a keystroke in the heading 4.3 → 1.3 ms, as it no longer re-measures the table; a
  step of dragging the window edge 3.2 → 0.7 ms; opening a 5,000-row table 54 → 9.7 ms.
- Live preview styles the visible lines one at a time when their keys are checked. Styling the
  visible range in one batch would be cheaper in a long table, but the cache checks keys lazily, line
  by line, without knowing the range ahead; the engine makes the per-line query cheap instead.
- A table in a list item nested in a quote, or after tabs, may be a few pixels off its header, since
  only leading spaces are widened.
- Tests: decoration ranges, alignments, escaped pipes, ragged rows, tables in quotes and lists and
  next to other blocks, the table query, incremental region updates and long tables
  (`tests/tables.rs`); random tables in `tests/markdown_properties.rs` and table lines in the
  region property tests (`markdown::tests`); the scanner (`blocks::tests`); the grid's column
  positions, alignment, clicks, reveal, wide and long tables, cells past the header's count and
  re-measuring after edits, resizes and zoom (`scratchpad/tests/tables.rs`, with an ignored timing
  test for typing next to a big table, resizing with it visible and opening a 5,000-row one).
