//! GFM tables (ADR 0186): decorations with exact pipe and delimiter row markers, alignment, escaped pipes,
//! ragged rows, tables inside containers and next to other blocks, the table query, region splitting while
//! editing, and how live preview styles and reveals table lines.

use std::ops::Range;

use scratchpad_editor::markdown::{
    Decoration, DecorationKind, MarkdownState, MarkerKind, Table, TableAlignment, TableRowKind,
};
use scratchpad_editor::{Bias, Buffer, ByteOffset, Editor, Selection};

fn decorations(text: &str) -> Vec<Decoration> {
    let buffer = Buffer::from_text(text);
    MarkdownState::new(&buffer).decorations(&buffer, ByteOffset(0)..buffer.end())
}

/// Kind, range, content and markers of each decoration, as the text they cover.
type Described = (String, String, String, Vec<String>);

fn describe(text: &str) -> Vec<Described> {
    let slice = |r: &Range<ByteOffset>| text[r.start.0..r.end.0].to_string();
    decorations(text)
        .iter()
        .map(|d| {
            let kind = match &d.kind {
                DecorationKind::Table { alignments } => format!("Table {alignments:?}"),
                DecorationKind::TableCell { column, alignment } => {
                    format!("Cell {column} {alignment:?}")
                }
                other => format!("{other:?}"),
            };
            let markers = d.markers.iter().map(slice).collect();
            (kind, slice(&d.range), slice(&d.content), markers)
        })
        .collect()
}

fn d(kind: &str, range: &str, content: &str, markers: &[&str]) -> Described {
    (
        kind.to_string(),
        range.to_string(),
        content.to_string(),
        markers.iter().map(|m| m.to_string()).collect(),
    )
}

/// Only the table's own decorations, without inline ones.
fn table_parts(text: &str) -> Vec<Described> {
    describe(text)
        .into_iter()
        .filter(|(kind, ..)| kind.starts_with("Table") || kind.starts_with("Cell"))
        .collect()
}

#[test]
fn a_table_has_rows_cells_alignments_and_exact_markers() {
    assert_eq!(
        describe("| a | b | c | d |\n|:--|:-:|--:|---|\n| x | y | z | w |"),
        [
            d(
                "Table [Left, Center, Right, None]",
                "| a | b | c | d |\n|:--|:-:|--:|---|\n| x | y | z | w |",
                "| a | b | c | d |\n|:--|:-:|--:|---|\n| x | y | z | w |",
                &[]
            ),
            d(
                "TableHeader",
                "| a | b | c | d |",
                " a | b | c | d ",
                &["|", "|", "|", "|", "|"]
            ),
            d("Cell 0 Left", " a ", "a", &[]),
            d("Cell 1 Center", " b ", "b", &[]),
            d("Cell 2 Right", " c ", "c", &[]),
            d("Cell 3 None", " d ", "d", &[]),
            d(
                "TableDelimiter",
                "|:--|:-:|--:|---|",
                "",
                &["|:--|:-:|--:|---|"]
            ),
            d(
                "TableRow",
                "| x | y | z | w |",
                " x | y | z | w ",
                &["|", "|", "|", "|", "|"]
            ),
            d("Cell 0 Left", " x ", "x", &[]),
            d("Cell 1 Center", " y ", "y", &[]),
            d("Cell 2 Right", " z ", "z", &[]),
            d("Cell 3 None", " w ", "w", &[]),
        ]
    );
}

#[test]
fn outer_pipes_are_optional() {
    assert_eq!(
        table_parts("a | b\n- | -\nc | d"),
        [
            d(
                "Table [None, None]",
                "a | b\n- | -\nc | d",
                "a | b\n- | -\nc | d",
                &[]
            ),
            d("TableHeader", "a | b", "a | b", &["|"]),
            d("Cell 0 None", "a ", "a", &[]),
            d("Cell 1 None", " b", "b", &[]),
            d("TableDelimiter", "- | -", "", &["- | -"]),
            d("TableRow", "c | d", "c | d", &["|"]),
            d("Cell 0 None", "c ", "c", &[]),
            d("Cell 1 None", " d", "d", &[]),
        ]
    );
}

#[test]
fn escaped_pipes_and_pipes_in_code_stay_in_their_cell() {
    assert_eq!(
        describe(
            r"| `a\|b` | c \| d |
|---|---|
| \| | x |"
        ),
        [
            d(
                "Table [None, None]",
                "| `a\\|b` | c \\| d |\n|---|---|\n| \\| | x |",
                "| `a\\|b` | c \\| d |\n|---|---|\n| \\| | x |",
                &[]
            ),
            d(
                "TableHeader",
                r"| `a\|b` | c \| d |",
                r" `a\|b` | c \| d ",
                &["|", "|", "|"]
            ),
            d("Cell 0 None", r" `a\|b` ", r"`a\|b`", &[]),
            d("InlineCode", r"`a\|b`", r"a\|b", &["`", "`"]),
            d("Cell 1 None", r" c \| d ", r"c \| d", &[]),
            d("TableDelimiter", "|---|---|", "", &["|---|---|"]),
            d("TableRow", r"| \| | x |", r" \| | x ", &["|", "|", "|"]),
            d("Cell 0 None", r" \| ", r"\|", &[]),
            d("Cell 1 None", " x ", "x", &[]),
        ]
    );
}

#[test]
fn inline_formatting_inside_cells_keeps_its_markers() {
    let parts = describe("| **a** | [l](u) |\n|---|---|\n| *x* | ~~y~~ |");
    for expected in [
        d("Strong", "**a**", "a", &["**", "**"]),
        d("Link { destination: \"u\" }", "[l](u)", "l", &["[", "](u)"]),
        d("Emphasis", "*x*", "x", &["*", "*"]),
        d("Strikethrough", "~~y~~", "y", &["~~", "~~"]),
    ] {
        assert!(parts.contains(&expected), "{expected:?} in {parts:?}");
    }
}

#[test]
fn ragged_rows_have_the_cells_they_have_and_drop_the_rest_into_their_last_marker() {
    assert_eq!(
        table_parts("| a | b |\n|---|---|\n| x |\n| 1 | 2 | 3 |\ny"),
        [
            d(
                "Table [None, None]",
                "| a | b |\n|---|---|\n| x |\n| 1 | 2 | 3 |\ny",
                "| a | b |\n|---|---|\n| x |\n| 1 | 2 | 3 |\ny",
                &[]
            ),
            d("TableHeader", "| a | b |", " a | b ", &["|", "|", "|"]),
            d("Cell 0 None", " a ", "a", &[]),
            d("Cell 1 None", " b ", "b", &[]),
            d("TableDelimiter", "|---|---|", "", &["|---|---|"]),
            // Short: no padding cell.
            d("TableRow", "| x |", " x ", &["|", "|"]),
            d("Cell 0 None", " x ", "x", &[]),
            // Long: the cell past the header's count belongs to the trailing marker.
            d("TableRow", "| 1 | 2 | 3 |", " 1 | 2 ", &["|", "|", "| 3 |"]),
            d("Cell 0 None", " 1 ", "1", &[]),
            d("Cell 1 None", " 2 ", "2", &[]),
            // A line without pipes continues the table as a row of one cell.
            d("TableRow", "y", "y", &[]),
            d("Cell 0 None", "y", "y", &[]),
        ]
    );
    // An empty cell between pipes is a cell.
    let parts = table_parts("| a | b |\n|---|---|\n|| x |");
    assert!(parts.contains(&d("Cell 0 None", "", "", &[])), "{parts:?}");
}

#[test]
fn a_header_must_match_the_delimiter_rows_column_count() {
    assert_eq!(table_parts("| a | b |\n|---|"), []);
    assert_eq!(table_parts("| a |\n| b |"), []);
}

#[test]
fn tables_in_block_quotes_and_list_items_mark_only_their_own_syntax() {
    let quoted = describe("> | a |\n> |:-:|\n> | b |");
    assert_eq!(
        quoted[0],
        d(
            "BlockQuote",
            "> | a |\n> |:-:|\n> | b |",
            "| a |\n> |:-:|\n> | b |",
            &["> ", "> ", "> "]
        )
    );
    assert_eq!(
        &quoted[1..],
        [
            d(
                "Table [Center]",
                "| a |\n> |:-:|\n> | b |",
                "| a |\n> |:-:|\n> | b |",
                &[]
            ),
            d("TableHeader", "| a |", " a ", &["|", "|"]),
            d("Cell 0 Center", " a ", "a", &[]),
            d("TableDelimiter", "|:-:|", "", &["|:-:|"]),
            d("TableRow", "| b |", " b ", &["|", "|"]),
            d("Cell 0 Center", " b ", "b", &[]),
        ]
    );

    let listed = table_parts("- | a |\n  |---|\n  | b |");
    assert_eq!(
        listed,
        [
            d(
                "Table [None]",
                "| a |\n  |---|\n  | b |",
                "| a |\n  |---|\n  | b |",
                &[]
            ),
            d("TableHeader", "| a |", " a ", &["|", "|"]),
            d("Cell 0 None", " a ", "a", &[]),
            d("TableDelimiter", "|---|", "", &["|---|"]),
            d("TableRow", "| b |", " b ", &["|", "|"]),
            d("Cell 0 None", " b ", "b", &[]),
        ]
    );
}

#[test]
fn tables_next_to_other_blocks() {
    // A table interrupts a paragraph (its last line is the header), and headings, list items, quotes, fences
    // and blank lines end it.
    let kinds = |text: &str| -> Vec<String> {
        describe(text)
            .into_iter()
            .filter(|(kind, ..)| !kind.starts_with("Cell"))
            .map(|(kind, range, ..)| format!("{kind}: {range}"))
            .collect()
    };
    assert_eq!(
        kinds("para\n| a |\n|---|\n| b |\n# H"),
        [
            "Table [None]: | a |\n|---|\n| b |",
            "TableHeader: | a |",
            "TableDelimiter: |---|",
            "TableRow: | b |",
            "Heading { level: 1 }: # H",
        ]
    );
    assert_eq!(
        kinds("# H\n| a |\n|---|\n- item\n\n| c |\n|---|\n> q"),
        [
            "Heading { level: 1 }: # H",
            "Table [None]: | a |\n|---|",
            "TableHeader: | a |",
            "TableDelimiter: |---|",
            "ListItem { ordered: false }: - item",
            "Table [None]: | c |\n|---|",
            "TableHeader: | c |",
            "TableDelimiter: |---|",
            "BlockQuote: > q",
        ]
    );
    assert_eq!(
        kinds("| a |\n|---|\n```\nx\n```"),
        [
            "Table [None]: | a |\n|---|",
            "TableHeader: | a |",
            "TableDelimiter: |---|",
            "CodeBlock { info: \"\" }: ```\nx\n```",
        ]
    );
}

#[test]
fn the_table_query_finds_the_table_of_a_line_and_whether_the_selection_reveals_it() {
    let text = "intro\n\n| a | b |\n|:-|-:|\n| 1 | 2 |\n\nafter";
    let buffer = Buffer::from_text(text);
    let mut markdown = MarkdownState::new(&buffer);
    let table_start = ByteOffset(text.find('|').unwrap());
    let table_end = ByteOffset(text.find("\n\nafter").unwrap());
    let expected = |revealed| Table {
        range: table_start..table_end,
        lines: 2..5,
        alignments: vec![TableAlignment::Left, TableAlignment::Right],
        revealed,
    };
    let outside = Some(Selection::cursor(ByteOffset(0)));
    for line in 2..5 {
        assert_eq!(
            markdown.table(&buffer, line, outside),
            Some(expected(false))
        );
    }
    for line in [0, 1, 5, 6, 99] {
        assert_eq!(markdown.table(&buffer, line, outside), None, "line {line}");
    }
    // Anywhere on the table's lines, edges included, and in source mode.
    for at in [table_start, table_end, ByteOffset(table_start.0 + 12)] {
        let selection = Some(Selection::cursor(at));
        assert_eq!(markdown.table(&buffer, 3, selection), Some(expected(true)));
    }
    assert_eq!(markdown.table(&buffer, 4, None), Some(expected(true)));
    let across = Some(Selection::new(ByteOffset(0), ByteOffset(text.len())));
    assert_eq!(markdown.table(&buffer, 4, across), Some(expected(true)));
}

#[test]
fn regions_split_around_tables_while_editing() {
    // A table and the blocks around it, edited the way it is typed: the incremental state always matches a
    // fresh parse, and each table stays whole.
    let mut ed = Editor::from_text("# Notes\n\npara\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nend");
    let mut markdown = MarkdownState::new(ed.buffer());
    markdown.decorations(ed.buffer(), ByteOffset(0)..ed.buffer().end());
    let check = |ed: &Editor, markdown: &mut MarkdownState| {
        let buffer = ed.buffer();
        let all = ByteOffset(0)..buffer.end();
        let incremental = markdown.decorations(buffer, all.clone());
        assert_eq!(
            incremental,
            MarkdownState::new(buffer).decorations(buffer, all),
            "{:?}",
            buffer.normalized_text()
        );
        incremental
            .iter()
            .filter(|d| matches!(d.kind, DecorationKind::Table { .. }))
            .count()
    };
    let end_of_table =
        |ed: &Editor| ByteOffset(ed.buffer().normalized_text().find("\n\nend").unwrap());

    // Rows typed at the end of the table, one keystroke at a time.
    for c in "\n| 3 | 4 |".chars() {
        let at = end_of_table(&ed);
        ed.move_to(at, false);
        ed.insert_text(c.encode_utf8(&mut [0; 4]));
        assert_eq!(check(&ed, &mut markdown), 1);
    }
    // Removing the blank line before the table makes the paragraph's last line... the paragraph, since the
    // header is the line before the delimiter row.
    let blank = ed.buffer().normalized_text().find("para\n\n").unwrap() + 4;
    ed.replace_range(ByteOffset(blank)..ByteOffset(blank + 1), "");
    assert_eq!(check(&ed, &mut markdown), 1);
    // A delimiter row that looks like a list item.
    let delimiter = ed.buffer().normalized_text().find("|---|---|").unwrap();
    ed.replace_range(ByteOffset(delimiter)..ByteOffset(delimiter + 9), "- | -");
    assert_eq!(check(&ed, &mut markdown), 1);
    // Breaking the delimiter row makes it text, and fixing it a table again.
    let delimiter = ed.buffer().normalized_text().find("- | -").unwrap();
    ed.replace_range(ByteOffset(delimiter + 2)..ByteOffset(delimiter + 3), "x");
    assert_eq!(check(&ed, &mut markdown), 0);
    ed.undo();
    assert_eq!(check(&ed, &mut markdown), 1);
    // A blank line typed between the rows splits off the rest as a paragraph.
    let row = ed.buffer().normalized_text().find("| 3 |").unwrap();
    ed.replace_range(ByteOffset(row)..ByteOffset(row), "\n");
    assert_eq!(check(&ed, &mut markdown), 1);
}

#[test]
fn a_table_longer_than_a_region_cut_stays_one_table() {
    let rows = vec!["| x | y |"; 600].join("\n");
    let mut ed = Editor::from_text(&format!("| a | b |\n|---|---|\n{rows}\n\nafter"));
    let mut markdown = MarkdownState::new(ed.buffer());
    let table = markdown
        .table(ed.buffer(), 500, None)
        .expect("row 500 is in the table");
    assert_eq!(table.lines, 0..602);
    // Typing in a row keeps it so.
    let at = ed.buffer().line_start(300);
    ed.move_to(ByteOffset(at.0 + 2), false);
    ed.insert_text("z");
    let table = markdown.table(ed.buffer(), 601, None).unwrap();
    assert_eq!(table.lines, 0..602);
}

// --- Live preview ---

/// The displayed text of each line of `text` with the cursor at byte `at`. (`|` is the cursor in the marked
/// text of other tests, so offsets are given here.)
fn shown(text: &str, at: usize) -> Vec<String> {
    let buffer = Buffer::from_text(text);
    MarkdownState::new(&buffer)
        .styled_lines(
            &buffer,
            0..usize::MAX,
            Some(Selection::cursor(ByteOffset(at))),
        )
        .iter()
        .enumerate()
        .map(|(i, line)| line.display_text(&buffer.line_text(i)))
        .collect()
}

#[test]
fn table_markers_hide_away_from_the_table_and_show_anywhere_in_it() {
    let text = "x\n\n| a | **b** |\n|---|:-:|\n| 1 | `2` |";
    // Away from the table: pipes and the delimiter row are hidden, and so are inline markers in cells.
    assert_eq!(shown(text, 1), ["x", "", " a  b ", "", " 1  2 "]);
    // With the cursor on any line of it, every table marker shows: the table is typed as source. Inline
    // markers in cells show only where the cursor touches them, as anywhere else.
    let header = text.find("| a").unwrap();
    assert_eq!(
        &shown(text, header + 2)[2..],
        ["| a | b |", "|---|:-:|", "| 1 | 2 |"]
    );
    assert_eq!(
        &shown(text, text.len() - 2)[2..],
        ["| a | b |", "|---|:-:|", "| 1 | `2` |"]
    );
    // At the table's first column it shows too; on the line before, not.
    assert_eq!(
        &shown(text, header)[2..],
        ["| a | b |", "|---|:-:|", "| 1 | 2 |"]
    );
    assert_eq!(shown(text, header - 1)[2], " a  b ");
}

fn styled_line(text: &str, line: usize, at: usize) -> scratchpad_editor::markdown::StyledLine {
    let buffer = Buffer::from_text(text);
    MarkdownState::new(&buffer)
        .styled_lines(
            &buffer,
            line..line + 1,
            Some(Selection::cursor(ByteOffset(at))),
        )
        .remove(0)
}

#[test]
fn table_lines_carry_their_row_kind_and_cells_their_column() {
    let text = "| a | b |\n|---|---|\n| c | d |\n\nx";
    let away = text.len();
    for (line, kind) in [
        (0, TableRowKind::Header),
        (1, TableRowKind::Delimiter),
        (2, TableRowKind::Body),
    ] {
        let styled = styled_line(text, line, away);
        assert_eq!(styled.style.table, Some(kind), "line {line}");
    }
    let header = styled_line(text, 0, away);
    let line_text = "| a | b |";
    let described: Vec<(&str, Option<MarkerKind>, bool, Option<u16>)> = header
        .spans
        .iter()
        .map(|s| {
            (
                &line_text[s.columns.clone()],
                s.marker,
                s.hidden,
                s.style.table_column,
            )
        })
        .collect();
    let pipe = Some(MarkerKind::TablePipe);
    assert_eq!(
        described,
        [
            ("|", pipe, true, None),
            (" a ", None, false, Some(0)),
            ("|", pipe, true, None),
            (" b ", None, false, Some(1)),
            ("|", pipe, true, None),
        ]
    );
    let delimiter = styled_line(text, 1, away);
    assert_eq!(delimiter.spans.len(), 1);
    assert_eq!(delimiter.spans[0].marker, Some(MarkerKind::TableDelimiter));
    assert!(delimiter.spans[0].hidden);
    assert_eq!(
        styled_line("x\n\n| a |\n|---|", 3, 0).style.table,
        Some(TableRowKind::Delimiter)
    );
    // Not a table line.
    assert_eq!(styled_line(text, 4, 0).style.table, None);
}

#[test]
fn columns_map_across_hidden_pipes_in_both_directions() {
    let text = "| ab | c |\n|---|---|\n\nx";
    let grid = styled_line(text, 0, text.len());
    let line = "| ab | c |";
    assert_eq!(grid.display_text(line), " ab  c ");
    // Buffer column -> display column: hidden pipes take no room.
    let display: Vec<usize> = (0..=line.len()).map(|c| grid.display_column(c)).collect();
    assert_eq!(display, [0, 0, 1, 2, 3, 4, 4, 5, 6, 7, 7]);
    // Display column -> buffer column: at a hidden pipe, Left stays before it and Right goes after it.
    assert_eq!(grid.buffer_column(0, Bias::Left), 0);
    assert_eq!(grid.buffer_column(0, Bias::Right), 1);
    assert_eq!(grid.buffer_column(4, Bias::Left), 5);
    assert_eq!(grid.buffer_column(4, Bias::Right), 6);
    assert_eq!(grid.buffer_column(7, Bias::Left), 9);
    assert_eq!(grid.buffer_column(7, Bias::Right), 10);
    // Revealed (cursor in the table), the columns are the buffer's.
    let raw = styled_line(text, 0, 3);
    for column in 0..=line.len() {
        assert_eq!(raw.display_column(column), column);
        assert_eq!(raw.buffer_column(column, Bias::Right), column);
    }
}
