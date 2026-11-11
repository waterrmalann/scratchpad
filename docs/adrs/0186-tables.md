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
  alignments, whether the selection reveals it) without cloning anything but the alignments.
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
  are hidden like any marker. `TableCache::fill` adds a `GridKey` (row kind and an `Arc` of the
  table's `Columns`) to the line's `LineKey`, and `LineLayout::shape` then moves each cell's text to
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
  column's widest cell. The entry is dropped when a buffer change touches the table's lines or the
  line after it, renumbered when lines are added or removed above it, dropped with the layouts when
  the style changes, and at most 16 tables are kept (least recently used dropped). A key compares
  its columns by pointer first, so checking a grid line's key costs no more than another line's.
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
- Tables longer than 256 lines are one region, so a keystroke in a 10,000-row pasted table reparses
  all of it (~15 ms per 400 KB). That is the price of keeping such tables whole; a table that big is
  rare in notes.
- Rows typed with fewer cells than the header leave their last columns empty in the grid; cells past
  the header's count are hidden with the trailing pipe, as GitHub drops them.
- A table in a list item nested in a quote, or after tabs, may be a few pixels off its header, since
  only leading spaces are widened.
- Tests: decoration ranges, alignments, escaped pipes, ragged rows, tables in quotes and lists and
  next to other blocks, the table query, incremental region updates and long tables
  (`tests/tables.rs`); random tables in `tests/markdown_properties.rs` and table lines in the
  region property tests (`markdown::tests`); the scanner (`blocks::tests`); the grid's column
  positions, alignment, clicks, reveal, wide tables and re-measuring (`scratchpad/tests/tables.rs`).
