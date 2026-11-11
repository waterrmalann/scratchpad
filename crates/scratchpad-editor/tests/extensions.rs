//! Highlights, math and emoji shortcodes (ADR 0190): exact decorations, what is left as text, live preview
//! reveal, and typed replacements leaving math alone.

mod common;

use std::ops::Range;

use common::{editor, state};
use scratchpad_editor::markdown::{
    DecorationKind, MarkdownOptions, MarkdownState, MarkerKind, emoji_for_shortcode,
};
use scratchpad_editor::{Buffer, ByteOffset, Editor};

/// The decorations of `text` with every option on, as `(kind, range text, content text, marker texts)`, of the
/// kinds this ADR adds.
fn found(text: &str) -> Vec<(String, String, String, Vec<String>)> {
    found_with(text, MarkdownOptions::ALL)
}

fn found_with(text: &str, options: MarkdownOptions) -> Vec<(String, String, String, Vec<String>)> {
    let buffer = Buffer::from_text(text);
    let slice = |r: &Range<ByteOffset>| buffer.text_for_range(r.clone()).into_owned();
    MarkdownState::with_options(&buffer, options)
        .decorations(&buffer, ByteOffset(0)..buffer.end())
        .iter()
        .filter_map(|d| {
            let kind = match &d.kind {
                DecorationKind::Highlight => "Highlight".to_owned(),
                DecorationKind::Math { display } => format!("Math {display}"),
                DecorationKind::Emoji { emoji } => format!("Emoji {emoji}"),
                _ => return None,
            };
            let markers = d.markers.iter().map(slice).collect();
            Some((kind, slice(&d.range), slice(&d.content), markers))
        })
        .collect()
}

fn d(
    kind: &str,
    range: &str,
    content: &str,
    markers: &[&str],
) -> (String, String, String, Vec<String>) {
    (
        kind.to_owned(),
        range.to_owned(),
        content.to_owned(),
        markers.iter().map(|m| (*m).to_owned()).collect(),
    )
}

fn highlights(text: &str) -> Vec<String> {
    found(text)
        .into_iter()
        .filter(|(kind, ..)| kind == "Highlight")
        .map(|(_, range, ..)| range)
        .collect()
}

#[test]
fn highlights_pair_their_double_equals_signs() {
    assert_eq!(
        found("a ==marked== b"),
        [d("Highlight", "==marked==", "marked", &["==", "=="])]
    );
    assert_eq!(highlights("==a== and ==b=="), ["==a==", "==b=="]);
    // Inside words, and around other inline markup.
    assert_eq!(highlights("x==y==z"), ["==y=="]);
    assert_eq!(highlights("==a *b* c=="), ["==a *b* c=="]);
    assert_eq!(highlights("**==a==**"), ["==a=="]);
    assert_eq!(highlights("==a `code` b=="), ["==a `code` b=="]);
    assert_eq!(highlights("[==a==](u)"), ["==a=="]);
    // Across a soft line break, like emphasis.
    assert_eq!(highlights("==a\nb=="), ["==a\nb=="]);
    // Nested ones pair inside out.
    assert_eq!(highlights("==a ==b== c=="), ["==a ==b== c==", "==b=="]);
    // In headings, quotes, list items and table cells.
    assert_eq!(highlights("# ==t=="), ["==t=="]);
    assert_eq!(highlights("> ==q=="), ["==q=="]);
    assert_eq!(highlights("- ==i=="), ["==i=="]);
    assert_eq!(
        highlights("| ==c== |\n|---|\n| ==d== |"),
        ["==c==", "==d=="]
    );
}

#[test]
fn equals_signs_that_are_not_highlights_stay_text() {
    for text in [
        "a == b == c",
        "===a===",
        "=a=",
        "== a==",
        "==a ==",
        "a ==b",
        r"\==a==",
        r"==a\==",
        "`==a==`",
        "```\n==a==\n```",
        "    ==a==",
        "<b title=\"==a==\">x</b>",
        "<!-- ==a== -->",
        "[x](==a==)",
        "<https://x.com/==a==>",
        "https://x.com/?a==b==",
        "[[==a==]]",
        // Not across the edge of another inline element.
        "*a ==b* c==",
        "==a *b== c*",
        "==a\n\nb==",
        // A setext underline is no delimiter.
        "a\n==",
        "==a\n==",
    ] {
        assert_eq!(highlights(text), Vec::<String>::new(), "{text:?}");
    }
    // An address in a highlight is a link inside it.
    assert_eq!(
        highlights("==see www.x.com now=="),
        ["==see www.x.com now=="]
    );
}

#[test]
fn highlights_follow_their_option_and_front_matter_has_none() {
    let off = MarkdownOptions {
        highlights: false,
        ..MarkdownOptions::ALL
    };
    assert!(found_with("==a==", off).is_empty());
    assert!(MarkdownOptions::default().highlights);
    assert_eq!(highlights("---\ntitle: ==a==\n---\n==b=="), ["==b=="]);
}

#[test]
fn math_is_off_by_default_and_hides_its_dollars_once_on() {
    assert!(found_with("$x$ and $$y$$", MarkdownOptions::default()).is_empty());
    assert!(!MarkdownOptions::default().math);
    assert_eq!(
        found("a $x^2$ b $$\\sum_i i$$"),
        [
            d("Math false", "$x^2$", "x^2", &["$", "$"]),
            d("Math true", "$$\\sum_i i$$", "\\sum_i i", &["$$", "$$"]),
        ]
    );
    // Display math across lines of a paragraph is one block; a blank line ends it as it ends the paragraph.
    assert_eq!(
        found("$$\na + b\n$$"),
        [d("Math true", "$$\na + b\n$$", "\na + b\n", &["$$", "$$"])]
    );
    assert!(found("$$\na\n\nb\n$$").is_empty());
    // Prices, escapes, code and unbalanced braces are not math.
    for text in ["$5 and $ 6", r"\$x$", "`$x$`", "$}$", "$ x $"] {
        assert!(found(text).is_empty(), "{text:?}");
    }
    // Math takes precedence over emphasis, as code spans do.
    let buffer = Buffer::from_text("*a $b*$");
    let all = MarkdownState::with_options(&buffer, MarkdownOptions::ALL)
        .decorations(&buffer, ByteOffset(0)..buffer.end());
    assert!(
        all.iter()
            .all(|d| !matches!(d.kind, DecorationKind::Emphasis))
    );
    // Found by the property test: pulldown-cmark reports math starting in a wikilink's target with a range
    // that is not delimited by its dollars; such math is dropped.
    assert!(found("#**[[$$|---|$]]").is_empty());
    assert!(found("[[$$|a|$]]").is_empty());
    assert_eq!(
        found("[[a|$b$]]"),
        [d("Math false", "$b$", "b", &["$", "$"])]
    );
    // Nothing is found inside math.
    assert_eq!(found("$a==b==c :smile:$").len(), 1);
}

#[test]
fn known_shortcodes_become_emoji_and_unknown_ones_stay_text() {
    assert_eq!(
        found("Nice :+1: and :smile:!"),
        [
            d("Emoji 👍", ":+1:", "", &[":+1:"]),
            d("Emoji 😄", ":smile:", "", &[":smile:"]),
        ]
    );
    assert_eq!(emoji_for_shortcode("tada"), Some("🎉"));
    // Next to each other, and in other inline markup.
    assert_eq!(found(":smile::smile:").len(), 2);
    assert_eq!(found("**:tada:** ==:fire:==").len(), 3);
    for text in [
        ":nope:",
        "10:30:45",
        "a:smile:",
        ":smile:a",
        "`:smile:`",
        "    :smile:",
        "[:smile:](u)",
        "[x](:smile:)",
        "https://x.com/:smile:",
        "<https://x.com/:smile:>",
        "[[:smile:]]",
        "<span title=\":smile:\">x</span>",
        r"\:smile:",
        "---\ntags: :smile:\n---",
    ] {
        assert!(found(text).is_empty(), "{text:?}");
    }
    let off = MarkdownOptions {
        emoji_shortcodes: false,
        ..MarkdownOptions::ALL
    };
    assert!(found_with(":smile:", off).is_empty());
}

/// Regions parse like the whole document with these constructs, at the edges where regions start.
#[test]
fn extensions_parse_alike_in_regions_and_the_whole_document() {
    let text =
        "==a==\n\n# ==b== :smile:\n- $x$ ==c\n- d==\n\n$$\ne\n$$\n\n| :+1: |\n|---|\n| ==f== |";
    let buffer = Buffer::from_text(text);
    let by_region = MarkdownState::with_options(&buffer, MarkdownOptions::ALL)
        .decorations(&buffer, ByteOffset(0)..buffer.end());
    let kinds = |decorations: &[scratchpad_editor::markdown::Decoration]| {
        decorations
            .iter()
            .filter(|d| {
                matches!(
                    d.kind,
                    DecorationKind::Highlight
                        | DecorationKind::Math { .. }
                        | DecorationKind::Emoji { .. }
                )
            })
            .count()
    };
    assert_eq!(kinds(&by_region), 7);
}

/// What live preview displays on each line for marked text (`|` cursor).
fn shown(marked: &str) -> String {
    let ed = editor(marked);
    let buffer = ed.buffer();
    MarkdownState::with_options(buffer, MarkdownOptions::ALL)
        .styled_lines(buffer, 0..usize::MAX, Some(ed.selection()))
        .iter()
        .enumerate()
        .map(|(i, line)| line.display_text(&buffer.line_text(i)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn highlight_and_math_markers_hide_away_from_the_cursor() {
    assert_eq!(shown("a ==b== c $x$\nnext|"), "a b c x\nnext");
    assert_eq!(shown("a ==b|== c $x$"), "a ==b== c x");
    assert_eq!(shown("a ==b== c $x|$"), "a b c $x$");
    // Display math shows its `$$` while the cursor is on any of its lines.
    assert_eq!(shown("$$\na|\n$$\n\nnext"), "$$\na\n$$\n\nnext");
    assert_eq!(shown("$$\na\n$$\n\nnext|"), "\na\n\n\nnext");
}

#[test]
fn styles_mark_highlighted_and_math_text_and_emoji_shortcodes() {
    let ed = editor("==h== $m$ :smile: :smile:\nnext|");
    let buffer = ed.buffer();
    let mut markdown = MarkdownState::with_options(buffer, MarkdownOptions::ALL);
    let line = markdown
        .styled_lines(buffer, 0..1, Some(ed.selection()))
        .remove(0);
    let text = buffer.line_text(0);
    let span = |needle: &str| {
        let at = text.find(needle).unwrap();
        line.spans
            .iter()
            .find(|s| s.columns.start == at)
            .unwrap()
            .clone()
    };
    assert!(span("h").style.highlight);
    assert!(span("m").style.math);
    assert_eq!(span("$").marker, Some(MarkerKind::Math));
    // Each shortcode is a span of its own, drawn as its emoji, and not hidden.
    let emoji: Vec<_> = line
        .spans
        .iter()
        .filter(|s| s.marker == Some(MarkerKind::Emoji { emoji: "😄" }))
        .collect();
    assert_eq!(emoji.len(), 2);
    assert!(emoji.iter().all(|s| !s.hidden && s.columns.len() == 7));

    // With the cursor on one, it is text; the other is still an emoji. In source mode both are text.
    let ed = editor(":smile:| :smile:");
    let buffer = ed.buffer();
    let mut markdown = MarkdownState::with_options(buffer, MarkdownOptions::ALL);
    let line = markdown
        .styled_lines(buffer, 0..1, Some(ed.selection()))
        .remove(0);
    let emoji = |line: &scratchpad_editor::markdown::StyledLine| {
        line.spans
            .iter()
            .filter(|s| matches!(s.marker, Some(MarkerKind::Emoji { .. })))
            .map(|s| s.columns.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(emoji(&line), vec![8..15]);
    let source = markdown.styled_lines(buffer, 0..1, None).remove(0);
    assert!(emoji(&source).is_empty());
}

/// Types `keys` one at a time into marked text, with the given options, and returns the marked result.
fn typed_with(marked: &str, keys: &str, options: MarkdownOptions) -> String {
    let mut ed: Editor = editor(marked);
    let mut markdown = MarkdownState::with_options(ed.buffer(), options);
    for c in keys.chars() {
        ed.insert_typed(c.encode_utf8(&mut [0; 4]), &mut markdown);
    }
    state(&ed)
}

#[test]
fn typed_replacements_leave_math_alone_once_it_is_on() {
    let math = MarkdownOptions::ALL;
    // Closed math, and math still being typed.
    assert_eq!(typed_with("$a | b$", "->", math), "$a ->| b$");
    assert_eq!(typed_with("$$x |", "<=", math), "$$x <=|");
    assert_eq!(typed_with("$x$ |", "->", math), "$x$ →|");
    // With Math off, `$` is a dollar.
    let off = MarkdownOptions::default();
    assert_eq!(typed_with("$a |", "->", off), "$a →|");
    assert_eq!(typed_with("costs $5 |", "->", off), "costs $5 →|");
    // Highlights and emoji are prose.
    assert_eq!(typed_with("==a | b==", "->", math), "==a →| b==");
}
