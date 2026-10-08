//! Line styles for rendering and live preview: which markers are hidden around the cursor, and mapping columns
//! between the buffer and the displayed text.

mod common;

use common::editor;
use scratchpad_editor::markdown::{MarkdownState, StyledLine};
use scratchpad_editor::{Bias, Editor, Selection};

/// What live preview displays for marked text (`|` cursor, `^` anchor): every line without its hidden markers.
fn shown(marked: &str) -> String {
    let ed = editor(marked);
    let buffer = ed.buffer();
    let lines =
        MarkdownState::new(buffer).styled_lines(buffer, 0..usize::MAX, Some(ed.selection()));
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
