//! Bare web and e-mail addresses (GFM extended autolinks, ADR 0187): which text becomes a link, with what
//! destination, and where it never does.

use scratchpad_editor::markdown::{DecorationKind, MarkdownState};
use scratchpad_editor::{Buffer, ByteOffset};

/// The bare addresses of `text`: the text each covers and its destination.
fn bare(text: &str) -> Vec<(String, String)> {
    let buffer = Buffer::from_text(text);
    MarkdownState::new(&buffer)
        .decorations(&buffer, ByteOffset(0)..buffer.end())
        .into_iter()
        .filter(|d| d.markers.is_empty())
        .filter_map(|d| match d.kind {
            DecorationKind::Link { destination } => {
                assert_eq!(d.range, d.content, "a bare address is all content");
                Some((buffer.text_for_range(d.range).into_owned(), destination))
            }
            _ => None,
        })
        .collect()
}

fn check(cases: &[(&str, &[(&str, &str)])]) {
    for (text, expected) in cases {
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(t, d)| (t.to_string(), d.to_string()))
            .collect();
        assert_eq!(bare(text), expected, "in {text:?}");
    }
}

/// The GFM spec's examples of extended `www.` autolinks (spec 0.29-gfm, 6.9).
#[test]
fn www_addresses_follow_the_gfm_examples() {
    check(&[
        (
            "www.commonmark.org",
            &[("www.commonmark.org", "https://www.commonmark.org")],
        ),
        (
            "Visit www.commonmark.org/help for more information.",
            &[("www.commonmark.org/help", "https://www.commonmark.org/help")],
        ),
        (
            "Visit www.commonmark.org.",
            &[("www.commonmark.org", "https://www.commonmark.org")],
        ),
        (
            "Visit www.commonmark.org/a.b.",
            &[("www.commonmark.org/a.b", "https://www.commonmark.org/a.b")],
        ),
        (
            "www.google.com/search?q=Markup+(business)",
            &[(
                "www.google.com/search?q=Markup+(business)",
                "https://www.google.com/search?q=Markup+(business)",
            )],
        ),
        (
            "www.google.com/search?q=Markup+(business)))",
            &[(
                "www.google.com/search?q=Markup+(business)",
                "https://www.google.com/search?q=Markup+(business)",
            )],
        ),
        (
            "(www.google.com/search?q=Markup+(business))",
            &[(
                "www.google.com/search?q=Markup+(business)",
                "https://www.google.com/search?q=Markup+(business)",
            )],
        ),
        (
            "(www.google.com/search?q=Markup+(business)",
            &[(
                "www.google.com/search?q=Markup+(business)",
                "https://www.google.com/search?q=Markup+(business)",
            )],
        ),
        (
            "www.google.com/search?q=(business))+ok",
            &[(
                "www.google.com/search?q=(business))+ok",
                "https://www.google.com/search?q=(business))+ok",
            )],
        ),
        (
            "www.google.com/search?q=commonmark&hl=en",
            &[(
                "www.google.com/search?q=commonmark&hl=en",
                "https://www.google.com/search?q=commonmark&hl=en",
            )],
        ),
        (
            "www.google.com/search?q=commonmark&hl;",
            &[(
                "www.google.com/search?q=commonmark",
                "https://www.google.com/search?q=commonmark",
            )],
        ),
        (
            "www.commonmark.org/he<lp",
            &[("www.commonmark.org/he", "https://www.commonmark.org/he")],
        ),
    ]);
}

/// The spec's examples of extended URL and e-mail autolinks.
#[test]
fn urls_and_email_addresses_follow_the_gfm_examples() {
    check(&[
        (
            "http://commonmark.org",
            &[("http://commonmark.org", "http://commonmark.org")],
        ),
        (
            "(Visit https://encrypted.google.com/search?q=Markup+(business))",
            &[(
                "https://encrypted.google.com/search?q=Markup+(business)",
                "https://encrypted.google.com/search?q=Markup+(business)",
            )],
        ),
        // Only web schemes: FTP is GFM's too, but nothing here would open it.
        ("Anonymous FTP is available at ftp://foo.bar.baz.", &[]),
        ("foo@bar.baz", &[("foo@bar.baz", "mailto:foo@bar.baz")]),
        (
            "hello@mail+xyz.example isn't valid, but hello+xyz@mail.example is.",
            &[("hello+xyz@mail.example", "mailto:hello+xyz@mail.example")],
        ),
        ("a.b-c_d@a.b", &[("a.b-c_d@a.b", "mailto:a.b-c_d@a.b")]),
        ("a.b-c_d@a.b.", &[("a.b-c_d@a.b", "mailto:a.b-c_d@a.b")]),
        ("a.b-c_d@a.b-", &[]),
        ("a.b-c_d@a.b_", &[]),
    ]);
}

#[test]
fn domains_must_be_valid() {
    check(&[
        // At least one period after `www`, no underscores in the last two segments, no empty segments.
        ("www.", &[]),
        ("www.com", &[("www.com", "https://www.com")]),
        ("www.a_b.com", &[]),
        ("www.a.b_c", &[]),
        ("www.a_b.c.d", &[("www.a_b.c.d", "https://www.a_b.c.d")]),
        ("www..com", &[]),
        // After a scheme a domain needs no period.
        (
            "http://localhost:8080/x",
            &[("http://localhost:8080/x", "http://localhost:8080/x")],
        ),
        ("https://", &[]),
        ("https:// x", &[]),
        ("https://_a", &[]),
        ("http:/x.com", &[]),
        // The scheme is every letter before `://`.
        ("xhttps://a.b", &[]),
        ("HTTPS://A.B", &[("HTTPS://A.B", "HTTPS://A.B")]),
        // Other schemes stay text.
        ("javascript:alert(1) file:///C:/x mailto:x", &[]),
        // `www.` starts a link only at the start, after whitespace or after `*`, `_`, `~` or `(`.
        ("awww.example.com", &[]),
        ("x.www.example.com", &[]),
        (
            "_www.example.com_",
            &[("www.example.com", "https://www.example.com")],
        ),
        ("WWW.EXAMPLE.COM", &[]),
        ("'www.example.com'", &[]),
        // A `<` that opens no autolink is text, and so is a definition that cannot interrupt a paragraph.
        (
            "<http://unclosed",
            &[("http://unclosed", "http://unclosed")],
        ),
        ("[ref]\n[ref]: http://x", &[("http://x", "http://x")]),
        ("[ref]: http://x", &[]),
    ]);
}

#[test]
fn trailing_punctuation_and_quotes_are_not_part_of_the_address() {
    check(&[
        (
            "See https://example.com/a?, or https://example.com/b!",
            &[
                ("https://example.com/a", "https://example.com/a"),
                ("https://example.com/b", "https://example.com/b"),
            ],
        ),
        (
            "\"https://example.com\": it's 'at www.example.com'.",
            &[
                ("https://example.com", "https://example.com"),
                ("www.example.com", "https://www.example.com"),
            ],
        ),
        (
            "https://example.com/a;b; and https://example.com/&amp;",
            &[
                ("https://example.com/a;b", "https://example.com/a;b"),
                ("https://example.com/", "https://example.com/"),
            ],
        ),
        (
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            &[(
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            )],
        ),
    ]);
}

#[test]
fn several_addresses_and_emphasis_around_them() {
    check(&[
        (
            "a https://a.com b www.b.com c x@c.com",
            &[
                ("https://a.com", "https://a.com"),
                ("www.b.com", "https://www.b.com"),
                ("x@c.com", "mailto:x@c.com"),
            ],
        ),
        (
            "**https://a.com** *www.b.com* ~~x@c.com~~",
            &[
                ("https://a.com", "https://a.com"),
                ("www.b.com", "https://www.b.com"),
                ("x@c.com", "mailto:x@c.com"),
            ],
        ),
        // The address with the user in it is one link, not a link and an e-mail address.
        (
            "https://user@example.com/x",
            &[("https://user@example.com/x", "https://user@example.com/x")],
        ),
        (
            "# Title https://a.com",
            &[("https://a.com", "https://a.com")],
        ),
        (
            "- item www.a.com\n> quote x@y.org",
            &[
                ("www.a.com", "https://www.a.com"),
                ("x@y.org", "mailto:x@y.org"),
            ],
        ),
        // Entities split the text into several events; the address runs across them.
        (
            "https://a.com/?x=1&amp;y=2",
            &[("https://a.com/?x=1&amp;y=2", "https://a.com/?x=1&amp;y=2")],
        ),
    ]);
}

#[test]
fn non_ascii_text_around_and_inside_addresses() {
    check(&[
        (
            "日本語https://example.jp/パス、です",
            &[(
                "https://example.jp/パス、です",
                "https://example.jp/パス、です",
            )],
        ),
        (
            "café https://例え.jp/ü. 👋🏽",
            &[("https://例え.jp/ü", "https://例え.jp/ü")],
        ),
        (
            "(é www.bücher.de)",
            &[("www.bücher.de", "https://www.bücher.de")],
        ),
        // `www.` and local parts need ASCII before them.
        ("éwww.example.com", &[]),
        ("é@example.com", &[]),
        (
            "ü x@example.com",
            &[("x@example.com", "mailto:x@example.com")],
        ),
        (
            "https://a.b\u{301}",
            &[("https://a.b\u{301}", "https://a.b\u{301}")],
        ),
    ]);
}

#[test]
fn code_links_html_and_autolinks_are_left_alone() {
    check(&[
        ("`https://a.com` and `www.b.com`", &[]),
        ("```\nhttps://a.com\n```", &[]),
        ("    https://a.com x@b.com", &[]),
        ("<div>\nhttps://a.com\n</div>", &[]),
        ("<a href=\"https://a.com\">", &[]),
        ("<!-- www.a.com -->", &[]),
        ("[https://a.com](https://b.com)", &[]),
        ("[text www.a.com](https://b.com)", &[]),
        ("![https://a.com](https://b.com/p.png)", &[]),
    ]);
    // Autolinks keep their markers and stay what they were.
    let buffer = Buffer::from_text("<https://a.com> <x@b.com>");
    let decorations = MarkdownState::new(&buffer).decorations(&buffer, ByteOffset(0)..buffer.end());
    assert_eq!(decorations.len(), 2);
    assert!(decorations.iter().all(|d| d.markers.len() == 2));
}

#[test]
fn closed_front_matter_has_no_bare_links() {
    let note = "---\nurl: https://a.com\nmail: x@b.com\n---\nSee https://c.com";
    check(&[(note, &[("https://c.com", "https://c.com")])]);
    // Front matter still being written, or a thematic break, does not hide them.
    check(&[(
        "---\nurl: https://a.com",
        &[("https://a.com", "https://a.com")],
    )]);
    check(&[(
        "---\n\nhttps://a.com\n---",
        &[("https://a.com", "https://a.com")],
    )]);
    // Front matter that is the whole note.
    check(&[("---\nurl: https://a.com\n---", &[])]);
}

#[test]
fn front_matter_follows_edits() {
    let mut editor = scratchpad_editor::Editor::from_text("---\nurl: https://a.com\n---\n");
    let mut markdown = MarkdownState::new(editor.buffer());
    let links = |editor: &scratchpad_editor::Editor, markdown: &mut MarkdownState| {
        markdown
            .decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end())
            .len()
    };
    // The decorations of the closed front matter, which has no link.
    let without_link = links(&editor, &mut markdown);
    // Removing the closing line opens the front matter, and the address is a link again.
    editor.replace_range(ByteOffset(23)..ByteOffset(26), "");
    assert_eq!(
        markdown.link_at(editor.buffer(), ByteOffset(10)).as_deref(),
        Some("https://a.com")
    );
    editor.replace_range(ByteOffset(23)..ByteOffset(23), "---");
    assert_eq!(markdown.link_at(editor.buffer(), ByteOffset(10)), None);
    assert_eq!(links(&editor, &mut markdown), without_link);
}

#[test]
fn link_at_finds_bare_addresses_with_their_destination() {
    let text = "go www.a.com or mail x@b.org now";
    let buffer = Buffer::from_text(text);
    let mut markdown = MarkdownState::new(&buffer);
    let at = |markdown: &mut MarkdownState, i: usize| markdown.link_at(&buffer, ByteOffset(i));
    assert_eq!(at(&mut markdown, 2), None);
    assert_eq!(at(&mut markdown, 3).as_deref(), Some("https://www.a.com"));
    assert_eq!(at(&mut markdown, 11).as_deref(), Some("https://www.a.com"));
    assert_eq!(at(&mut markdown, 12), None, "the char after the address");
    assert_eq!(at(&mut markdown, 21).as_deref(), Some("mailto:x@b.org"));
}

/// Bare addresses are styled like links and have nothing to hide, wherever the cursor is.
#[test]
fn bare_addresses_are_styled_as_links_with_nothing_hidden() {
    let text = "see https://a.com now";
    let buffer = Buffer::from_text(text);
    let mut markdown = MarkdownState::new(&buffer);
    for cursor in [0, 6, text.len()] {
        let selection = scratchpad_editor::Selection::cursor(ByteOffset(cursor));
        let line = &markdown.styled_lines(&buffer, 0..1, Some(selection))[0];
        assert_eq!(line.display_text(text), text);
        let link: Vec<_> = line
            .spans
            .iter()
            .filter(|span| span.style.link)
            .map(|span| &text[span.columns.clone()])
            .collect();
        assert_eq!(link, ["https://a.com"]);
    }
}
