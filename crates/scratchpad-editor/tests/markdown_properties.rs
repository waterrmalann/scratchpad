//! Randomized Markdown: parsing never panics and decorations are always well formed.

use proptest::prelude::*;
use scratchpad_editor::markdown::{Decoration, MarkdownState};
use scratchpad_editor::{Buffer, ByteOffset};

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
];

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
    fn decorations_of_random_markdown_are_well_formed(text in markdown_text()) {
        let buffer = Buffer::from_text(&text);
        let decorations = MarkdownState::new(&buffer).decorations(&buffer, ByteOffset(0)..buffer.end());
        check_well_formed(&buffer.normalized_text(), &decorations)?;
    }
}
