//! Randomized Markdown: parsing never panics and decorations are always well formed.

use proptest::prelude::*;
use scratchpad_editor::markdown::{
    Decoration, DecorationKind, MarkdownOptions, MarkdownState, MarkerKind, StyledSpan,
    wikilink_page,
};
use scratchpad_editor::{Bias, Buffer, ByteOffset, Selection};

/// Markdown syntax fragments, prose and hostile Unicode that combine into well-formed and malformed documents.
const ATOMS: &[&str] = &[
    "#",
    "# ",
    "## ",
    "**",
    "*",
    "_",
    "__",
    "~~",
    "~",
    "`",
    "```",
    "~~~",
    "\n",
    "\n\n",
    "> ",
    "- ",
    "* ",
    "1. ",
    "2) ",
    "[ ] ",
    "[x] ",
    "[",
    "](",
    ")",
    "<",
    ">",
    "http://a.b",
    "---",
    "===",
    "    ",
    "  ",
    "\t",
    "<!--",
    "-->",
    "a",
    "word",
    "é",
    "e\u{301}",
    "日本",
    "👋🏽",
    "\r\n",
    "\\",
    "!",
    "^",
    "[!NOTE]",
    "[!tip]",
    "{#a .b}",
    "{",
    "}",
    "[[",
    "]]",
    "|",
    "Page",
    "[[Page|label]]",
    "https://",
    "www.",
    "x@y.z",
    ".com",
    "(",
    "&amp;",
    ";",
    "'",
    "|",
    " | ",
    "|---|",
    ":-:",
    "- | -",
    r"\|",
    "| a | b |\n|---|---|\n",
];

/// Any options, every one on in a third of the cases.
fn options() -> impl Strategy<Value = MarkdownOptions> {
    prop_oneof![
        1 => Just(MarkdownOptions::ALL),
        2 => any::<[bool; 5]>().prop_map(|[callouts, superscript, subscript, heading_ids, wikilinks]| {
            MarkdownOptions { callouts, superscript, subscript, heading_ids, wikilinks }
        }),
    ]
}

fn markdown_text() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::collection::vec(prop::sample::select(ATOMS), 0..60).prop_map(|atoms| atoms.concat()),
        1 => any::<String>(),
    ]
}

/// Checks that every range lies on char boundaries of `text`, that content and markers lie within their
/// decoration, that markers are ordered and disjoint, and that decorations are ordered by start (enclosing ones
/// first) and either nested or disjoint.
fn check_well_formed(text: &str, decorations: &[Decoration]) -> Result<(), TestCaseError> {
    let valid = |r: &std::ops::Range<ByteOffset>| {
        r.start <= r.end
            && r.end.0 <= text.len()
            && text.is_char_boundary(r.start.0)
            && text.is_char_boundary(r.end.0)
    };
    let within = |inner: &std::ops::Range<ByteOffset>, outer: &std::ops::Range<ByteOffset>| {
        outer.start <= inner.start && inner.end <= outer.end
    };
    for d in decorations {
        prop_assert!(valid(&d.range) && valid(&d.content), "{d:?} in {text:?}");
        prop_assert!(within(&d.content, &d.range), "{d:?} in {text:?}");
        for marker in &d.markers {
            prop_assert!(
                valid(marker) && within(marker, &d.range),
                "{d:?} in {text:?}"
            );
        }
        for pair in d.markers.windows(2) {
            prop_assert!(pair[0].end <= pair[1].start, "{d:?} in {text:?}");
        }
    }
    for pair in decorations.windows(2) {
        let (a, b) = (&pair[0].range, &pair[1].range);
        prop_assert!(
            a.start < b.start || (a.start == b.start && a.end >= b.end),
            "unordered {pair:?} in {text:?}"
        );
    }
    for (i, a) in decorations.iter().enumerate() {
        for b in &decorations[i + 1..] {
            let nested = b.range.end <= a.range.end;
            let disjoint = b.range.start >= a.range.end;
            prop_assert!(nested || disjoint, "{a:?} overlaps {b:?} in {text:?}");
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn decorations_of_random_markdown_are_well_formed(text in markdown_text(), options in options()) {
        let buffer = Buffer::from_text(&text);
        let decorations = MarkdownState::with_options(&buffer, options).decorations(&buffer, ByteOffset(0)..buffer.end());
        check_well_formed(&buffer.normalized_text(), &decorations)?;
    }

    /// Styled lines cover each line with consecutive spans on char boundaries, and mapping a column to the
    /// display and back lands on the same column or across the hidden markers next to it.
    #[test]
    fn styled_lines_cover_lines_and_columns_round_trip(
        text in markdown_text(),
        anchor in 0..200usize,
        head in 0..200usize,
        options in options(),
    ) {
        let buffer = Buffer::from_text(&text);
        let selection = Selection::new(
            buffer.clip_offset(ByteOffset(anchor), Bias::Left),
            buffer.clip_offset(ByteOffset(head), Bias::Left),
        );
        let lines = MarkdownState::with_options(&buffer, options).styled_lines(&buffer, 0..buffer.line_count(), Some(selection));
        prop_assert_eq!(lines.len(), buffer.line_count());
        for (i, line) in lines.iter().enumerate() {
            let line_text = buffer.line_text(i);
            let mut end = 0;
            for span in &line.spans {
                prop_assert_eq!(span.columns.start, end);
                prop_assert!(span.columns.start < span.columns.end);
                prop_assert!(line_text.is_char_boundary(span.columns.end));
                end = span.columns.end;
            }
            prop_assert_eq!(end, line_text.len());
            let display = line.display_text(&line_text);
            for column in (0..=line_text.len()).filter(|&c| line_text.is_char_boundary(c)) {
                let shown = line.display_column(column);
                prop_assert!(display.is_char_boundary(shown));
                let left = line.buffer_column(shown, Bias::Left);
                let right = line.buffer_column(shown, Bias::Right);
                prop_assert!(left <= column && column <= right, "{column} -> {shown} -> {left}..{right}");
                prop_assert_eq!(line.display_column(left), shown);
                prop_assert_eq!(line.display_column(right), shown);
            }
        }
    }

    /// Bare addresses (links without markers) keep the invariants of every decoration, never reach into code or
    /// another decoration's markers, and always get a destination that opens on Ctrl+click.
    #[test]
    fn bare_addresses_are_well_formed(text in markdown_text(), options in options()) {
        let buffer = Buffer::from_text(&text);
        let decorations = MarkdownState::with_options(&buffer, options).decorations(&buffer, ByteOffset(0)..buffer.end());
        let text = buffer.normalized_text();
        check_well_formed(&text, &decorations)?;
        for bare in decorations.iter().filter(|d| d.markers.is_empty()) {
            let DecorationKind::Link { destination } = &bare.kind else { continue };
            let shown = &text[bare.range.start.0..bare.range.end.0];
            prop_assert_eq!(&bare.content, &bare.range);
            prop_assert!(
                destination.ends_with(shown)
                    && ["http://", "https://", "mailto:"]
                        .iter()
                        .any(|scheme| destination.get(..scheme.len()).is_some_and(|s| s.eq_ignore_ascii_case(scheme))),
                "{destination:?} for {shown:?} in {text:?}"
            );
            prop_assert!(!shown.contains(char::is_whitespace), "{shown:?} in {text:?}");
            for other in &decorations {
                let code = matches!(other.kind, DecorationKind::InlineCode | DecorationKind::CodeBlock { .. });
                let overlaps = |r: &std::ops::Range<ByteOffset>| r.start < bare.range.end && bare.range.start < r.end;
                prop_assert!(!(code && overlaps(&other.content)), "{bare:?} in code {other:?} in {text:?}");
                prop_assert!(!other.markers.iter().any(overlaps), "{bare:?} overlaps a marker of {other:?} in {text:?}");
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// With wikilinks on, every wikilink names a plain page, is `[[...]]` with `[[`, `]]` and any `target|` as its
    /// markers, and asking which pages exist only flags its spans as missing, changing nothing else.
    #[test]
    fn wikilinks_are_well_formed_and_missing_pages_only_mark_their_spans(
        text in markdown_text(),
        options in options(),
    ) {
        let options = MarkdownOptions { wikilinks: true, ..options };
        let buffer = Buffer::from_text(&text);
        let mut markdown = MarkdownState::with_options(&buffer, options);
        let decorations = markdown.decorations(&buffer, ByteOffset(0)..buffer.end());
        check_well_formed(&buffer.normalized_text(), &decorations)?;
        for d in &decorations {
            let DecorationKind::WikiLink { target } = &d.kind else { continue };
            prop_assert!(wikilink_page(target).is_some(), "{d:?}");
            let slice = |r: &std::ops::Range<ByteOffset>| buffer.text_for_range(r.clone()).into_owned();
            let markers: Vec<String> = d.markers.iter().map(slice).collect();
            prop_assert_eq!(markers.first().map(String::as_str), Some("[["));
            prop_assert_eq!(markers.last().map(String::as_str), Some("]]"));
            if markers.len() == 3 {
                // `\|` separates the label too (Obsidian's form for tables).
                prop_assert!(
                    markers[1] == format!("{target}|") || markers[1] == format!("{target}\\|"),
                    "{d:?}"
                );
            } else {
                prop_assert_eq!(markers.len(), 2);
                prop_assert_eq!(&slice(&d.content), target);
            }
        }
        let all = 0..buffer.line_count();
        let plain = markdown.styled_lines(&buffer, all.clone(), None);
        let marked = markdown.styled_lines_with_pages(&buffer, all, None, |target| target.len() % 2 == 0);
        prop_assert_eq!(plain.len(), marked.len());
        for (plain, marked) in plain.iter().zip(&marked) {
            prop_assert!(marked.spans.iter().all(|s| !s.style.missing_page || s.style.link));
            // The flag may split a span that runs from one link into the next (`[[A]][[B]]`, the `]][[`
            // markers): without it, the spans merge back into those of `styled_lines`.
            let mut unmarked: Vec<StyledSpan> = Vec::new();
            for span in &marked.spans {
                let mut span = span.clone();
                span.style.missing_page = false;
                match unmarked.last_mut() {
                    Some(last)
                        if last.style == span.style
                            && last.marker == span.marker
                            && last.hidden == span.hidden =>
                    {
                        last.columns.end = span.columns.end;
                    }
                    _ => unmarked.push(span),
                }
            }
            prop_assert_eq!(&plain.spans, &unmarked);
        }
    }
}

/// Headings, setext text and list items around attribute blocks and the syntax that may reach into them: the
/// input pulldown-cmark 0.13 panics on with its own heading attributes (`- a\n    {#a}\n  - `) and its variants.
fn heading_text() -> impl Strategy<Value = String> {
    const ATOMS: &[&str] = &[
        "# ", "## ", "- ", "* ", "1. ", "> ", "  ", "    ", "\t", "\n", "\r\n", "\n---", "\n===",
        "a", " ", "{", "}", "{#a}", " {.b}", "{}", "\\", "*", "_", "`", "[[a]]", "[x](y)",
        "http://x", "www.x.y", "<", ">", "#",
    ];
    prop::collection::vec(prop::sample::select(ATOMS), 0..16).prop_map(|atoms| atoms.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    /// With heading IDs on, nothing panics and decorations stay well formed (ADR 0185).
    #[test]
    fn heading_attribute_blocks_never_break_the_parse(text in heading_text(), options in options()) {
        let options = MarkdownOptions { heading_ids: true, ..options };
        let buffer = Buffer::from_text(&text);
        let mut markdown = MarkdownState::with_options(&buffer, options);
        let decorations = markdown.decorations(&buffer, ByteOffset(0)..buffer.end());
        check_well_formed(&buffer.normalized_text(), &decorations)?;
    }
}

/// Tables built from cells of text, inline syntax, escaped pipes and code spans, rows with or without outer
/// pipes and with too few or too many cells, random delimiter rows, inside quotes and list items or after a
/// paragraph, followed by other blocks (ADR 0186).
fn table_text() -> impl Strategy<Value = String> {
    const CELLS: &[&str] = &[
        "", " ", "a", " b ", "**c**", "`d`", r"e\|f", r"`g\|h`", "*", "`", "é", "[x](y)", "~~s~~",
    ];
    const DELIMITERS: &[&str] = &["---", ":--", "--:", ":-:", "-", " - "];
    /// What the table starts with, and what each later line of it starts with.
    const CONTAINERS: &[(&str, &str)] = &[
        ("", ""),
        ("", ""),
        ("> ", "> "),
        ("- ", "  "),
        ("intro\n", ""),
    ];
    const AFTER: &[&str] = &["", "\n", "\n\npara", "\n# h", "\n- item", "\nlazy", "\n> q"];
    let row = |columns: usize| {
        (
            prop::collection::vec(prop::sample::select(CELLS), 1..=columns + 1),
            any::<bool>(),
        )
            .prop_map(|(cells, outer)| {
                let inner = cells.join("|");
                if outer { format!("|{inner}|") } else { inner }
            })
    };
    (1..4usize).prop_flat_map(move |columns| {
        (
            prop::collection::vec(prop::sample::select(CELLS), columns),
            prop::collection::vec(prop::sample::select(DELIMITERS), columns),
            prop::collection::vec(row(columns), 0..5),
            prop::sample::select(CONTAINERS),
            prop::sample::select(AFTER),
        )
            .prop_map(|(header, delimiters, rows, (first, rest), after)| {
                let header: Vec<String> = header.iter().map(|cell| format!("h{cell}")).collect();
                let mut lines = vec![
                    format!("|{}|", header.join("|")),
                    format!("|{}|", delimiters.join("|")),
                ];
                lines.extend(rows);
                format!("{first}{}{after}", lines.join(&format!("\n{rest}")))
            })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Table decorations are well formed, every line of a table (and only those) is styled as one, and its
    /// markers hide or show together: shown exactly while the cursor is on one of the table's lines.
    #[test]
    fn tables_are_well_formed_and_reveal_as_a_whole(text in table_text(), at in 0..300usize) {
        let buffer = Buffer::from_text(&text);
        let mut markdown = MarkdownState::new(&buffer);
        let decorations = markdown.decorations(&buffer, ByteOffset(0)..buffer.end());
        check_well_formed(&text, &decorations)?;
        let cursor = Selection::cursor(buffer.clip_offset(ByteOffset(at), Bias::Left));
        let lines = markdown.styled_lines(&buffer, 0..buffer.line_count(), Some(cursor));
        for (i, line) in lines.iter().enumerate() {
            let table = markdown.table(&buffer, i, Some(cursor));
            prop_assert_eq!(table.is_some(), line.style.table.is_some(), "line {} of {:?}", i, text);
            let Some(table) = table else { continue };
            prop_assert!(table.lines.contains(&i));
            let on_table = table.lines.contains(&buffer.line_of(cursor.head));
            prop_assert_eq!(table.revealed, on_table);
            for span in &line.spans {
                if matches!(span.marker, Some(MarkerKind::TablePipe | MarkerKind::TableDelimiter)) {
                    prop_assert_eq!(span.hidden, !on_table, "line {} of {:?}", i, text);
                }
            }
        }
    }
}
