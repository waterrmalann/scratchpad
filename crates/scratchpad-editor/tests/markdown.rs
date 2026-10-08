//! Markdown decorations: exact ranges for each construct, nesting, malformed input, Unicode and CRLF documents.

use std::ops::Range;

use scratchpad_editor::markdown::{Decoration, DecorationKind, MarkdownState};
use scratchpad_editor::{Buffer, ByteOffset};

fn decorations(text: &str) -> Vec<Decoration> {
    let buffer = Buffer::from_text(text);
    MarkdownState::new(&buffer).decorations(&buffer, ByteOffset(0)..buffer.end())
}

/// A decoration described by the text its ranges cover, which pins down exact byte ranges while staying
/// readable: kind, range, content and markers.
type Described = (String, String, String, Vec<String>);

fn describe(text: &str) -> Vec<Described> {
    let buffer = Buffer::from_text(text);
    let slice = |r: &Range<ByteOffset>| buffer.text_for_range(r.clone()).into_owned();
    decorations(text)
        .iter()
        .map(|d| {
            let kind = match &d.kind {
                DecorationKind::Heading { level } => format!("H{level}"),
                DecorationKind::Link { destination } => format!("Link {destination}"),
                DecorationKind::CodeBlock { info } => format!("Code {info}"),
                DecorationKind::ListItem { ordered: true } => "Ordered".to_string(),
                DecorationKind::ListItem { ordered: false } => "Bullet".to_string(),
                DecorationKind::Task { checked } => format!("Task {checked}"),
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

#[test]
fn atx_headings_of_every_level_with_optional_closing_sequence() {
    assert_eq!(
        describe("# One\n###### Six ##\n####### seven\n#\n## a#"),
        [
            d("H1", "# One", "One", &["# "]),
            d("H6", "###### Six ##", "Six", &["###### ", " ##"]),
            d("H1", "#", "", &["#"]),
            d("H2", "## a#", "a#", &["## "]),
        ]
    );
}

#[test]
fn setext_headings_mark_their_underline() {
    assert_eq!(
        describe("Title\n===\n\nSub\n---"),
        [
            d("H1", "Title\n===", "Title", &["==="]),
            d("H2", "Sub\n---", "Sub", &["---"]),
        ]
    );
}

#[test]
fn emphasis_strong_strikethrough_and_code_spans() {
    assert_eq!(
        describe("*i* __b__ ~~s~~ ~t~ ``a`b``"),
        [
            d("Emphasis", "*i*", "i", &["*", "*"]),
            d("Strong", "__b__", "b", &["__", "__"]),
            d("Strikethrough", "~~s~~", "s", &["~~", "~~"]),
            d("Strikethrough", "~t~", "t", &["~", "~"]),
            d("InlineCode", "``a`b``", "a`b", &["``", "``"]),
        ]
    );
}

#[test]
fn nested_emphasis_is_reported_outermost_first() {
    assert_eq!(
        describe("***both*** **a *b* c**"),
        [
            d("Emphasis", "***both***", "**both**", &["*", "*"]),
            d("Strong", "**both**", "both", &["**", "**"]),
            d("Strong", "**a *b* c**", "a *b* c", &["**", "**"]),
            d("Emphasis", "*b*", "b", &["*", "*"]),
        ]
    );
}

#[test]
fn links_mark_brackets_and_destination() {
    assert_eq!(
        describe("[a **b**](http://x.y \"T\") <https://z.org> [](e) ![img](i.png)"),
        [
            d(
                "Link http://x.y",
                "[a **b**](http://x.y \"T\")",
                "a **b**",
                &["[", "](http://x.y \"T\")"]
            ),
            d("Strong", "**b**", "b", &["**", "**"]),
            d(
                "Link https://z.org",
                "<https://z.org>",
                "https://z.org",
                &["<", ">"]
            ),
            d("Link e", "[](e)", "", &["[", "](e)"]),
        ]
    );
}

#[test]
fn reference_links_stay_plain_text_even_when_defined() {
    assert_eq!(describe("[ref]\n[ref]: http://x"), []);
}

#[test]
fn block_quotes_mark_every_line_including_nested_quotes() {
    let text = "> a\n> > b\n> > c\nlazy\n>\n> d";
    assert_eq!(
        describe(text),
        [
            d(
                "BlockQuote",
                text,
                &text[2..],
                &["> ", "> ", "> ", ">", "> "]
            ),
            d(
                "BlockQuote",
                "> b\n> > c\nlazy",
                "b\n> > c\nlazy",
                &["> ", "> "]
            ),
        ]
    );
    let starts: Vec<Vec<usize>> = decorations("> a\n> > b\n> > c")
        .iter()
        .map(|d| d.markers.iter().map(|m| m.start.0).collect())
        .collect();
    assert_eq!(starts, [vec![0, 4, 10], vec![6, 12]]);
}

#[test]
fn quotes_inside_list_items_find_their_markers_after_the_indent() {
    assert_eq!(
        describe("- > a\n  > b"),
        [
            d("Bullet", "- > a\n  > b", "> a\n  > b", &["- "]),
            d("BlockQuote", "> a\n  > b", "a\n  > b", &["> ", "> "]),
        ]
    );
}

#[test]
fn bullet_ordered_and_task_items() {
    assert_eq!(
        describe("- one\n* two\n+   three\n\n1. a\n10) b\n\n- [ ] todo\n- [x] done\n-\n"),
        [
            d("Bullet", "- one", "one", &["- "]),
            d("Bullet", "* two", "two", &["* "]),
            d("Bullet", "+   three", "three", &["+   "]),
            d("Ordered", "1. a", "a", &["1. "]),
            d("Ordered", "10) b", "b", &["10) "]),
            d("Bullet", "- [ ] todo", "[ ] todo", &["- "]),
            d("Task false", "[ ] todo", "todo", &["[ ] "]),
            d("Bullet", "- [x] done", "[x] done", &["- "]),
            d("Task true", "[x] done", "done", &["[x] "]),
            d("Bullet", "-", "", &["-"]),
        ]
    );
}

#[test]
fn nested_list_items_contain_their_children() {
    assert_eq!(
        describe("- a\n  - b\n    c"),
        [
            d("Bullet", "- a\n  - b\n    c", "a\n  - b\n    c", &["- "]),
            d("Bullet", "- b\n    c", "b\n    c", &["- "]),
        ]
    );
}

#[test]
fn an_indented_list_is_marked_from_its_bullet() {
    assert_eq!(
        describe("  - a\n  - b\n\n   3. c"),
        [
            d("Bullet", "- a", "a", &["- "]),
            d("Bullet", "- b", "b", &["- "]),
            d("Ordered", "3. c", "c", &["3. "]),
        ]
    );
}

#[test]
fn thematic_breaks() {
    assert_eq!(
        describe("---\n\n* * *\n\n___"),
        [
            d("ThematicBreak", "---", "", &["---"]),
            d("ThematicBreak", "* * *", "", &["* * *"]),
            d("ThematicBreak", "___", "", &["___"]),
        ]
    );
}

#[test]
fn fenced_code_blocks_mark_fence_lines_and_keep_info() {
    assert_eq!(
        describe("```rust\nfn x() {}\n\n**no**\n```\n\n~~~\n~~~\n\n````\nopen"),
        [
            d(
                "Code rust",
                "```rust\nfn x() {}\n\n**no**\n```",
                "fn x() {}\n\n**no**",
                &["```rust", "```"]
            ),
            d("Code ", "~~~\n~~~", "", &["~~~", "~~~"]),
            d("Code ", "````\nopen", "open", &["````"]),
        ]
    );
}

#[test]
fn fenced_code_in_a_quote_marks_only_the_fences() {
    assert_eq!(
        describe("> ```\n> x\n> ```"),
        [
            d(
                "BlockQuote",
                "> ```\n> x\n> ```",
                "```\n> x\n> ```",
                &["> ", "> ", "> "]
            ),
            d("Code ", "```\n> x\n> ```", "> x", &["```", "```"]),
        ]
    );
}

#[test]
fn indented_code_blocks_have_no_markers() {
    assert_eq!(
        describe("    let x;\n    y\n\npara"),
        [d("Code ", "let x;\n    y", "let x;\n    y", &[])]
    );
}

#[test]
fn malformed_markdown_degrades_to_plain_text() {
    for text in [
        "**unclosed",
        "*a **b",
        "[text](",
        "`code",
        "~~strike",
        "<http://unclosed",
        "#hashtag",
        "-not a list",
        "1.5 million",
    ] {
        assert_eq!(decorations(text), [], "{text:?}");
    }
}

#[test]
fn escapes_html_images_and_tables_stay_plain_text() {
    for text in [
        r"\*not emphasis\* \`not code\` \[not](a link)",
        "<!-- *hidden* -->",
        "![alt](pic.png)",
        "| a | b |\n| --- | --- |\n| 1 | 2 |",
        "bare a@b.c www.x.y http://plain.url",
    ] {
        assert_eq!(describe(text), [], "{text:?}");
    }
    // Raw HTML is not a construct of its own; Markdown between its tags is still styled.
    assert_eq!(
        describe("<span>**x**</span>"),
        [d("Strong", "**x**", "x", &["**", "**"])]
    );
    assert_eq!(
        describe("<a@b.c>"),
        [d("Link a@b.c", "<a@b.c>", "a@b.c", &["<", ">"])]
    );
}

#[test]
fn unicode_content_keeps_char_boundaries() {
    assert_eq!(
        describe("# 日本語 🇩🇪\n\n**e\u{301}** [👋🏽](ü)"),
        [
            d("H1", "# 日本語 🇩🇪", "日本語 🇩🇪", &["# "]),
            d("Strong", "**e\u{301}**", "e\u{301}", &["**", "**"]),
            d("Link ü", "[👋🏽](ü)", "👋🏽", &["[", "](ü)"]),
        ]
    );
}

#[test]
fn crlf_documents_are_decorated_in_lf_coordinates() {
    let text = "# Title\r\n\r\n- **a**\r\n";
    let ranges: Vec<_> = decorations(text)
        .iter()
        .map(|d| d.range.start.0..d.range.end.0)
        .collect();
    assert_eq!(ranges, [0..7, 9..16, 11..16]);
    assert_eq!(describe(text), describe("# Title\n\n- **a**\n"));
}

#[test]
fn decorations_query_returns_only_those_touching_the_range() {
    let buffer = Buffer::from_text("para *a*\n\n# H\n\n`c` d");
    let mut markdown = MarkdownState::new(&buffer);
    let mut kinds = |range: Range<usize>| {
        markdown
            .decorations(&buffer, ByteOffset(range.start)..ByteOffset(range.end))
            .into_iter()
            .map(|d| d.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(0..4), []);
    assert_eq!(kinds(8..8), [DecorationKind::Emphasis]);
    assert_eq!(kinds(9..9), []);
    assert_eq!(
        kinds(6..16),
        [
            DecorationKind::Emphasis,
            DecorationKind::Heading { level: 1 },
            DecorationKind::InlineCode
        ]
    );
}

#[test]
fn link_at_finds_the_destination_under_the_pointer() {
    let buffer = Buffer::from_text("see [docs](https://a.b/c) now");
    let mut markdown = MarkdownState::new(&buffer);
    assert_eq!(
        markdown.link_at(&buffer, ByteOffset(6)).as_deref(),
        Some("https://a.b/c")
    );
    assert_eq!(markdown.link_at(&buffer, ByteOffset(2)), None);
    // The opening bracket is part of the link, the space after the closing parenthesis is not.
    assert!(markdown.link_at(&buffer, ByteOffset(4)).is_some());
    assert_eq!(markdown.link_at(&buffer, ByteOffset(25)), None);
}

#[test]
fn a_task_whose_text_is_a_setext_heading_keeps_document_order() {
    assert_eq!(
        describe("- [ ] a\n  ---"),
        [
            d("Bullet", "- [ ] a\n  ---", "[ ] a\n  ---", &["- "]),
            d("Task false", "[ ] a\n  ---", "a\n  ---", &["[ ] "]),
            d("H2", "a\n  ---", "a", &["---"]),
        ]
    );
}
