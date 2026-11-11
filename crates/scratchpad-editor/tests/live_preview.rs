//! Line styles for rendering and live preview: which markers are hidden around the cursor, and mapping columns
//! between the buffer and the displayed text.

mod common;

use common::editor;
use scratchpad_editor::markdown::{
    CalloutKind, MarkdownOptions, MarkdownState, MarkerKind, StyledLine,
};
use scratchpad_editor::{Bias, Editor, Selection};

/// What live preview displays for marked text (`|` cursor, `^` anchor): every line without its hidden markers.
fn shown(marked: &str) -> String {
    shown_with(marked, MarkdownOptions::default())
}

fn shown_with(marked: &str, options: MarkdownOptions) -> String {
    let ed = editor(marked);
    let buffer = ed.buffer();
    let lines = MarkdownState::with_options(buffer, options).styled_lines(
        buffer,
        0..usize::MAX,
        Some(ed.selection()),
    );
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| line.display_text(&buffer.line_text(i)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn styled(ed: &Editor, line: usize, selection: Option<Selection>) -> StyledLine {
    let buffer = ed.buffer();
    let mut lines = MarkdownState::new(buffer).styled_lines(buffer, line..line + 1, selection);
    lines.remove(0)
}

/// Each span of a line as its text and a compact description of its style.
fn spans(text: &str, line: usize) -> Vec<(String, String)> {
    let ed = Editor::from_text(text);
    let line_text = ed.buffer().line_text(line).into_owned();
    styled(&ed, line, None)
        .spans
        .iter()
        .map(|span| {
            let s = span.style;
            let mut flags = Vec::new();
            if s.heading > 0 {
                flags.push(format!("h{}", s.heading));
            }
            for (on, name) in [
                (s.bold, "bold"),
                (s.italic, "italic"),
                (s.strikethrough, "strike"),
                (s.code, "code"),
                (s.link, "link"),
                (s.quote, "quote"),
                (s.code_block, "block"),
            ] {
                if on {
                    flags.push(name.to_string());
                }
            }
            if let Some(marker) = span.marker {
                flags.push(format!("{marker:?}"));
            }
            (line_text[span.columns.clone()].to_string(), flags.join(" "))
        })
        .collect()
}

fn span(text: &str, flags: &str) -> (String, String) {
    (text.to_string(), flags.to_string())
}

#[test]
fn heading_line_spans_carry_the_level_and_inline_styles() {
    assert_eq!(
        spans("## A **b** `c`", 0),
        [
            span("## ", "h2 Heading"),
            span("A ", "h2"),
            span("**", "h2 bold Emphasis"),
            span("b", "h2 bold"),
            span("**", "h2 bold Emphasis"),
            span(" ", "h2"),
            span("`", "h2 code Code"),
            span("c", "h2 code"),
            span("`", "h2 code Code"),
        ]
    );
}

#[test]
fn spans_cover_the_whole_line_including_plain_text() {
    assert_eq!(
        spans("x ~~y~~ [z](u) w", 0),
        [
            span("x ", ""),
            span("~~", "strike Emphasis"),
            span("y", "strike"),
            span("~~", "strike Emphasis"),
            span(" ", ""),
            span("[", "link Link"),
            span("z", "link"),
            span("](u)", "link Link"),
            span(" w", ""),
        ]
    );
}

#[test]
fn quote_list_and_task_markers_are_reported_per_line() {
    let text = "> q\n> - [x] t";
    assert_eq!(
        spans(text, 0),
        [span("> ", "quote Quote"), span("q", "quote")]
    );
    assert_eq!(
        spans(text, 1),
        [
            span("> ", "quote Quote"),
            span("- ", "quote ListBullet"),
            span("[x] ", "quote TaskBox { checked: true }"),
            span("t", "quote"),
        ]
    );
}

#[test]
fn block_styles_cover_whole_lines_even_when_empty() {
    let ed = Editor::from_text("```\na\n\n```\n\n> b\n\n# H");
    let lines = MarkdownState::new(ed.buffer()).styled_lines(ed.buffer(), 0..8, None);
    let code_block: Vec<bool> = lines.iter().map(|l| l.style.code_block).collect();
    assert_eq!(
        code_block,
        [true, true, true, true, false, false, false, false]
    );
    assert!(lines[2].spans.is_empty());
    assert!(lines[5].style.quote);
    assert_eq!(lines[7].style.heading, 1);
    assert_eq!(lines[6].style, Default::default());
}

#[test]
fn block_styles_cover_lines_where_the_block_starts_after_container_markers() {
    let ed = Editor::from_text("> # H\n- > q\n  > r\n\n- ```\n  x\n  ```");
    let lines = MarkdownState::new(ed.buffer()).styled_lines(ed.buffer(), 0..7, None);
    let styles: Vec<(u8, bool, bool)> = lines
        .iter()
        .map(|l| (l.style.heading, l.style.quote, l.style.code_block))
        .collect();
    assert_eq!(
        styles,
        [
            (1, true, false),
            (0, true, false),
            (0, true, false),
            (0, false, false),
            (0, false, true),
            (0, false, true),
            (0, false, true),
        ]
    );
}

#[test]
fn inline_markers_show_only_while_the_cursor_touches_the_span() {
    assert_eq!(shown("Some **bold** and *it*|"), "Some bold and *it*");
    assert_eq!(shown("Some **bo|ld** and *it*"), "Some **bold** and it");
    assert_eq!(shown("Some |**bold** and *it*"), "Some **bold** and it");
    assert_eq!(shown("Some **bold**| and *it*"), "Some **bold** and it");
    assert_eq!(shown("Some **bold** |and *it*"), "Some bold and it");
}

#[test]
fn nested_spans_reveal_only_the_spans_the_cursor_touches() {
    assert_eq!(shown("***a|b*** c"), "***ab*** c");
    assert_eq!(shown("**x *y* z|**"), "**x y z**");
    assert_eq!(shown("**x *y|* z**"), "**x *y* z**");
}

#[test]
fn block_markers_show_while_the_cursor_is_on_their_line() {
    assert_eq!(shown("# Title|\n\ntext"), "# Title\n\ntext");
    assert_eq!(shown("# Title\n\ntext|"), "Title\n\ntext");
    assert_eq!(shown("> one\n> two|"), "one\n> two");
    assert_eq!(shown("a\n\n---\n\nb|"), "a\n\n\n\nb");
}

#[test]
fn list_bullets_and_task_boxes_are_never_hidden() {
    assert_eq!(
        shown("- one\n1. two\n- [ ] three\n\nx|"),
        "- one\n1. two\n- [ ] three\n\nx"
    );
}

#[test]
fn code_fences_show_while_the_cursor_is_anywhere_in_the_block() {
    let text = "```rust\nlet a;\n```\n\nafter";
    assert_eq!(
        shown(&format!("{}|{}", &text[..10], &text[10..])),
        "```rust\nlet a;\n```\n\nafter"
    );
    assert_eq!(shown(&format!("{text}|")), "\nlet a;\n\n\nafter");
}

#[test]
fn a_selection_reveals_every_marker_it_touches() {
    assert_eq!(
        shown("# H\n\n^**a** *b* `c`|\n\n**d**"),
        "H\n\n**a** *b* `c`\n\nd"
    );
}

#[test]
fn source_mode_hides_nothing() {
    let ed = Editor::from_text("# H **b**");
    assert!(styled(&ed, 0, None).spans.iter().all(|s| !s.hidden));
}

#[test]
fn columns_map_between_the_buffer_and_the_display() {
    let ed = editor("a **b** c\n|");
    let line = styled(&ed, 0, Some(ed.selection()));
    assert_eq!(line.display_text("a **b** c"), "a b c");
    assert_eq!(
        (0..=9).map(|c| line.display_column(c)).collect::<Vec<_>>(),
        [0, 1, 2, 2, 2, 3, 3, 3, 4, 5]
    );
    // Display column 2 sits where the hidden `**` is: before it or after it.
    assert_eq!(line.buffer_column(2, Bias::Left), 2);
    assert_eq!(line.buffer_column(2, Bias::Right), 4);
    assert_eq!(line.buffer_column(3, Bias::Left), 5);
    assert_eq!(line.buffer_column(3, Bias::Right), 7);
    assert_eq!(line.buffer_column(1, Bias::Right), 1);
    assert_eq!(line.buffer_column(99, Bias::Left), 9);
}

#[test]
fn styles_follow_edits() {
    let mut ed = editor("plain|");
    let mut markdown = MarkdownState::new(ed.buffer());
    assert_eq!(
        markdown.styled_lines(ed.buffer(), 0..1, None)[0]
            .style
            .heading,
        0
    );
    ed.move_cursor(scratchpad_editor::Motion::LineStart, false);
    ed.insert_text("# ");
    assert_eq!(
        markdown.styled_lines(ed.buffer(), 0..1, None)[0]
            .style
            .heading,
        1
    );
    ed.undo();
    assert_eq!(
        markdown.styled_lines(ed.buffer(), 0..1, None)[0]
            .style
            .heading,
        0
    );
}

#[test]
fn images_hide_their_markers_like_links() {
    assert_eq!(
        shown("See ![a plot](p.png) here\n\nx|"),
        "See a plot here\n\nx"
    );
    assert_eq!(shown("See ![a pl|ot](p.png)"), "See ![a plot](p.png)");
    let ed = Editor::from_text("![a](p.png)");
    let line = styled(&ed, 0, None);
    assert!(line.spans.iter().all(|span| span.style.link));
    assert_eq!(line.spans[0].marker, Some(MarkerKind::Link));
}

#[test]
fn superscript_and_subscript_hide_their_markers_and_style_their_text() {
    // `^` marks the anchor in `shown`'s text, so these lines are styled directly. As pulldown-cmark reads them,
    // neither works inside a word: `x^2^` is text.
    let text = "x ^2^ H ~2~ O ~~no~~ y^3^";
    let ed = Editor::from_text(text);
    let buffer = ed.buffer();
    let mut markdown = MarkdownState::with_options(buffer, MarkdownOptions::ALL);
    let line = |markdown: &mut MarkdownState, cursor: usize| {
        let selection = Selection::cursor(scratchpad_editor::ByteOffset(cursor));
        markdown
            .styled_lines(buffer, 0..1, Some(selection))
            .remove(0)
    };
    assert_eq!(
        line(&mut markdown, 0).display_text(text),
        "x 2 H 2 O no y^3^"
    );
    assert_eq!(
        line(&mut markdown, 3).display_text(text),
        "x ^2^ H 2 O no y^3^"
    );
    let styles: Vec<(&str, bool, bool)> = line(&mut markdown, 0)
        .spans
        .iter()
        .map(|span| {
            let style = span.style;
            (
                &text[span.columns.clone()],
                style.superscript,
                style.subscript,
            )
        })
        .collect();
    assert_eq!(
        &styles[..7],
        [
            ("x ", false, false),
            ("^", true, false),
            ("2", true, false),
            ("^", true, false),
            (" H ", false, false),
            ("~", false, true),
            ("2", false, true),
        ]
    );
}

#[test]
fn heading_attribute_blocks_hide_until_the_cursor_is_on_the_heading() {
    let ids = MarkdownOptions {
        heading_ids: true,
        ..MarkdownOptions::default()
    };
    assert_eq!(shown_with("# Title {#t}\n\nx|", ids), "Title\n\nx");
    assert_eq!(shown_with("# Ti|tle {#t}\n\nx", ids), "# Title {#t}\n\nx");
    assert_eq!(shown_with("Setext {#s}\n===\n\nx|", ids), "Setext\n\n\nx");
}

#[test]
fn callout_tags_are_never_hidden_and_every_line_of_the_quote_has_its_kind() {
    assert_eq!(
        shown("> [!WARNING]\n>\n> body\n\nx|"),
        "[!WARNING]\n\nbody\n\nx"
    );
    let ed = Editor::from_text("> [!WARNING]\n>\n> body\n\nx");
    for line in 0..3 {
        let styled = styled(&ed, line, None);
        assert!(styled.style.quote, "{line}");
        assert_eq!(styled.style.callout, Some(CalloutKind::Warning), "{line}");
    }
    let tag = styled(&ed, 0, Some(Selection::cursor(ed.buffer().end())));
    let marker = tag.spans.iter().find(|span| span.columns == (2..12));
    assert_eq!(
        marker.map(|span| (span.marker, span.hidden)),
        Some((
            Some(MarkerKind::Callout {
                kind: CalloutKind::Warning
            }),
            false
        ))
    );
    assert_eq!(styled(&ed, 4, None).style.callout, None);
}

#[test]
fn wikilinks_show_only_their_label_or_page_away_from_the_cursor() {
    // `|` marks the cursor in `shown`, so these texts place it themselves.
    let shown_at = |text: &str, cursor: usize| {
        let ed = Editor::from_text(text);
        let buffer = ed.buffer();
        let at = scratchpad_editor::ByteOffset(cursor);
        MarkdownState::new(buffer)
            .styled_lines(buffer, 0..usize::MAX, Some(Selection::new(at, at)))
            .iter()
            .enumerate()
            .map(|(i, line)| line.display_text(&buffer.line_text(i)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = "[[Page]] and [[Page|label]] and [[Page#Top]]\n";
    assert_eq!(shown_at(text, text.len()), "Page and label and Page#Top\n");
    assert_eq!(
        shown_at(text, 16),
        "Page and [[Page|label]] and Page#Top\n",
        "the cursor on the link reveals its markers"
    );
    let off = MarkdownOptions {
        wikilinks: false,
        ..MarkdownOptions::default()
    };
    assert_eq!(shown_with("[[Page]]\n|", off), "[[Page]]\n");
}

#[test]
fn wikilinks_to_missing_pages_are_marked_and_only_links_are_asked_about() {
    let ed = Editor::from_text("[[Here]] [[Gone|g]] **b**\n\nplain\n[[Gone]]");
    let buffer = ed.buffer();
    let asked = std::cell::RefCell::new(Vec::new());
    let lines = MarkdownState::new(buffer).styled_lines_with_pages(buffer, 0..4, None, |target| {
        asked.borrow_mut().push(target.to_owned());
        target == "Here"
    });
    let missing = |line: usize| -> Vec<String> {
        let text = buffer.line_text(line).into_owned();
        lines[line]
            .spans
            .iter()
            .filter(|span| span.style.missing_page)
            .map(|span| text[span.columns.clone()].to_owned())
            .collect()
    };
    assert_eq!(missing(0), ["[[Gone|", "g", "]]"]);
    assert!(
        lines[0]
            .spans
            .iter()
            .all(|s| !s.style.missing_page || s.style.link)
    );
    assert_eq!(missing(2), Vec::<String>::new());
    assert_eq!(missing(3), ["[[", "Gone", "]]"]);
    assert_eq!(*asked.borrow(), ["Here", "Gone", "Gone"]);

    // Lines without links ask nothing.
    asked.borrow_mut().clear();
    MarkdownState::new(buffer).styled_lines_with_pages(buffer, 1..3, None, |target| {
        asked.borrow_mut().push(target.to_owned());
        false
    });
    assert!(asked.borrow().is_empty());
}
