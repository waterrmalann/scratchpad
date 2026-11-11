//! Bare web and e-mail addresses in prose, styled and opened as links without a single character of Markdown
//! around them (GFM "extended autolinks"; ADR 0187).
//!
//! [`Autolinks`] watches the events of a region's parse and scans only text: runs of consecutive `Text` events
//! outside code blocks, links and images. Code spans, raw HTML and autolinks (`<...>`) are other events, so they
//! are never scanned. What is found becomes a [`DecorationKind::Link`] with no markers, whose destination is
//! the address made openable: `www.` addresses get `https://`, e-mail addresses `mailto:`.
//!
//! The rules are GFM's (spec 0.29, "Autolinks (extension)", as cmark-gfm implements them): `http://` and
//! `https://` followed by a domain, `www.` followed by a domain with a period after start of line, whitespace
//! or one of `*_~(`, and `local@domain.tld`. A domain is alphanumeric segments (`-` and `_` allowed, no `_` in
//! the last two) separated by periods. The address runs to whitespace or `<`, then loses trailing punctuation
//! (`?!.,:*_~'"`), an entity-like `&name;` and any `)` without a matching `(`. Every byte the rules look at or
//! cut at is ASCII, so all ranges are char boundaries.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Tag, TagEnd};

use super::{Decoration, DecorationKind, MAX_FRONT_MATTER_BYTES, MarkdownState, front_matter};
use crate::buffer::Buffer;
use crate::coords::ByteOffset;

/// How a `www.` address starts.
const WWW: &str = "www.";

/// Whether `run`, a run of non-whitespace text, holds or starts a web or e-mail address, possibly one still being
/// typed: it contains `://` or `@`, or starts with `www.`. Typed replacements leave such runs alone (ADR 0180).
pub(crate) fn looks_like_web_address(run: &str) -> bool {
    run.starts_with(WWW) || run.contains("://") || run.contains('@')
}

/// Whether `text` may contain an address: it has a `:` (of a scheme), an `@` or `www.`. Most prose has none of
/// them, and this one pass over the bytes is the whole cost of scanning it. (`str::contains` with a `&str`
/// builds a searcher on every call, which costs more than the search on the short texts of most events.)
fn may_contain_address(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        b == b':' || b == b'@' || (b == b'.' && i >= 3 && &bytes[i - 3..i] == b"www")
    })
}

/// Whether `decoration` is found in a region's text rather than given by pulldown-cmark's block structure, and is
/// not syntax of front matter, which is YAML: a bare address (a link without markers), a highlight, an emoji
/// shortcode or math (ADR 0187, 0190).
fn is_text_mark(decoration: &Decoration) -> bool {
    match decoration.kind {
        DecorationKind::Link { .. } => decoration.markers.is_empty(),
        DecorationKind::Highlight | DecorationKind::Emoji { .. } | DecorationKind::Math { .. } => {
            true
        }
        _ => false,
    }
}

impl MarkdownState {
    /// Drops the bare addresses, highlights, emoji shortcodes and math in the note's closed front matter from
    /// `found`, decorations of the synced `buffer`. Front matter is YAML, not prose, and it is a property of the
    /// whole note, which a region's parse cannot see (ADR 0041), so it is applied to query results. It is looked
    /// for only when such a decoration lies where front matter could be, and then once per buffer version.
    pub(super) fn drop_front_matter_text_marks(
        &mut self,
        buffer: &Buffer,
        found: &mut Vec<Decoration>,
    ) {
        // Front matter is its first line, of at most 8 bytes, and at most this many more.
        let reach = MAX_FRONT_MATTER_BYTES + 9;
        if !found
            .iter()
            .any(|d| is_text_mark(d) && d.range.start.0 < reach)
        {
            return;
        }
        let key = (buffer.version(), buffer.len());
        let end = match self.front_matter_end {
            Some((version, len, end)) if (version, len) == key => end,
            _ => {
                let end = front_matter(buffer, usize::MAX)
                    .filter(|front_matter| front_matter.closed)
                    .map_or(0, |front_matter| {
                        if front_matter.end < buffer.line_count() {
                            buffer.line_start(front_matter.end).0
                        } else {
                            buffer.len()
                        }
                    });
                self.front_matter_end = Some((key.0, key.1, end));
                end
            }
        };
        if end > 0 {
            found.retain(|d| !(is_text_mark(d) && d.range.start.0 < end));
        }
    }
}

/// Whether links of `link_type` resolve against reference definitions, which may be in another region: those
/// stay text (ADR 0040), so text scanners see through them, reading their brackets as text. Whether a definition
/// exists then changes nothing found in the text around them, which a region's parse could not know.
pub(super) fn is_reference(link_type: LinkType) -> bool {
    matches!(
        link_type,
        LinkType::Reference
            | LinkType::ReferenceUnknown
            | LinkType::Collapsed
            | LinkType::CollapsedUnknown
            | LinkType::Shortcut
            | LinkType::ShortcutUnknown
    )
}

/// A link or image the next event is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenLink {
    Wiki,
    Other,
    /// A reference link (see [`is_reference`]), with where the events inside it end so far.
    Reference(usize),
}

/// Collects the bare addresses of one region's text while its events go by.
#[derive(Debug, Default)]
pub(super) struct Autolinks {
    /// The run of consecutive text events seen last, not yet scanned.
    run: Option<Range<usize>>,
    /// Where text starts that is a text node of its own in GFM, as at the start of a line: a table cell's text,
    /// and (an Obsidian addition) text right after a wikilink. `www.` may start an address there.
    node_start: Option<usize>,
    /// How many code blocks the next event is in.
    code_blocks: usize,
    /// The links and images the next event is in, innermost last.
    links: Vec<OpenLink>,
    /// How many of them are not reference links: text in those is not scanned.
    opaque: usize,
    found: Vec<Decoration>,
}

impl Autolinks {
    /// Takes the next event of the parse of `text`, with its range. Text from `text_end` on is not prose (a
    /// heading's attribute block) and is not scanned.
    pub(super) fn event(
        &mut self,
        event: &Event<'_>,
        range: &Range<usize>,
        text: &str,
        text_end: Option<usize>,
    ) {
        let range = &(range.start..range.end.min(text_end.unwrap_or(usize::MAX)));
        let link_end = matches!(event, Event::End(TagEnd::Link | TagEnd::Image));
        if let Some(OpenLink::Reference(end)) = self.links.last_mut()
            && !link_end
        {
            *end = (*end).max(range.end);
        }
        let scanned = self.code_blocks == 0 && self.opaque == 0;
        match event {
            Event::Text(_) if scanned => {
                self.text(text, range.clone());
                return;
            }
            Event::Start(Tag::CodeBlock(_)) => self.code_blocks += 1,
            Event::End(TagEnd::CodeBlock) => self.code_blocks = self.code_blocks.saturating_sub(1),
            Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. })
                if is_reference(*link_type) =>
            {
                // Its `[` or `![` is text, as without a definition.
                let opener = if matches!(event, Event::Start(Tag::Image { .. })) {
                    2
                } else {
                    1
                };
                let opener = range.start..(range.start + opener).min(range.end);
                self.links.push(OpenLink::Reference(opener.end));
                if scanned {
                    self.text(text, opener);
                }
                return;
            }
            Event::Start(Tag::Link { link_type, .. }) => {
                let wikilink = matches!(link_type, LinkType::WikiLink { .. });
                self.links.push(if wikilink {
                    OpenLink::Wiki
                } else {
                    OpenLink::Other
                });
                self.opaque += 1;
            }
            Event::Start(Tag::Image { .. }) => {
                self.links.push(OpenLink::Other);
                self.opaque += 1;
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                let open = self.links.pop();
                if let Some(OpenLink::Reference(inner_end)) = open {
                    // Its `]`, `][ref]` or `][]` is text too.
                    if scanned {
                        self.text(text, inner_end.min(range.end)..range.end);
                    }
                    return;
                }
                self.opaque = self.opaque.saturating_sub(1);
                self.flush(text);
                if open == Some(OpenLink::Wiki) {
                    self.node_start = Some(range.end);
                }
                return;
            }
            Event::Start(Tag::TableCell) => {
                self.flush(text);
                self.node_start = Some(range.start);
                return;
            }
            _ => {}
        }
        self.flush(text);
    }

    /// Adds `piece` of `text` to the run of text it follows, or scans that run and starts a new one.
    fn text(&mut self, text: &str, piece: Range<usize>) {
        if piece.is_empty() {
            return;
        }
        match &mut self.run {
            // Entities, escapes and unmatched delimiters split text into several events.
            Some(run) if run.end == piece.start => run.end = piece.end,
            _ => {
                self.flush(text);
                self.run = Some(piece);
            }
        }
    }

    /// The addresses found so far, in document order. A text run is scanned when the event after it arrives, so
    /// those of a run are here once [`Autolinks::event`] has taken the event that ends it.
    pub(super) fn found(&self) -> &[Decoration] {
        &self.found
    }

    /// Scans the last text run; the addresses are then all in [`Autolinks::found`].
    pub(super) fn flush_last(&mut self, text: &str) {
        self.flush(text);
    }

    /// The decorations of the addresses found in `text`, in document order.
    pub(super) fn finish(mut self, text: &str) -> Vec<Decoration> {
        self.flush(text);
        self.found
    }

    fn flush(&mut self, text: &str) {
        if let Some(run) = self.run.take() {
            let starts_node = self.node_start == Some(run.start);
            scan(text, run, starts_node, &mut self.found);
        }
    }
}

/// Finds the addresses in `text[run]`, a run of text, and pushes their decorations to `found`. `starts_node` says
/// whether the run starts a text node of its own, where `www.` may start an address whatever comes before.
fn scan(text: &str, run: Range<usize>, starts_node: bool, found: &mut Vec<Decoration>) {
    if !may_contain_address(&text[run.clone()]) {
        return;
    }
    let bytes = text.as_bytes();
    // Where the next address may start: not inside the one found before.
    let mut floor = run.start;
    let mut i = run.start;
    while i < run.end {
        let address = match bytes[i] {
            b':' => url(text, floor, i, run.end),
            b'w' => www(text, i, starts_node && i == run.start, run.end),
            b'@' => email(text, floor, i, run.end),
            _ => None,
        };
        match address {
            Some((range, destination)) => {
                found.push(Decoration {
                    kind: DecorationKind::Link { destination },
                    range: ByteOffset(range.start)..ByteOffset(range.end),
                    content: ByteOffset(range.start)..ByteOffset(range.end),
                    markers: Vec::new(),
                });
                i = range.end;
                floor = range.end;
            }
            None => i += 1,
        }
    }
}

/// An `http://` or `https://` address whose `:` is at `colon`, starting at or after `floor` and ending by `end`.
fn url(text: &str, floor: usize, colon: usize, end: usize) -> Option<(Range<usize>, String)> {
    let bytes = text.as_bytes();
    if !bytes[colon..end].starts_with(b"://") {
        return None;
    }
    // The scheme is all the letters before the colon, so `xhttp://` is no address.
    let start = colon
        - bytes[floor..colon]
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_alphabetic())
            .count();
    let scheme = &text[start..colon];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    // A domain without a period (`localhost`) is allowed after a scheme.
    let domain_end = domain(text, colon + 3, end, false)?;
    let link_end = trim(text, start, path_end(text, domain_end, end));
    (link_end > colon + 3).then(|| (start..link_end, text[start..link_end].to_owned()))
}

/// A `www.` address starting at `at`, if `www.` is there. `node_start` says `at` starts a text node, which counts
/// as a start of line (as each table cell's text does in cmark-gfm), so `|www.x.com|` in a table cell and
/// `[[a]]www.x.com` link the address.
fn www(text: &str, at: usize, node_start: bool, end: usize) -> Option<(Range<usize>, String)> {
    let bytes = text.as_bytes();
    if !bytes[at..end].starts_with(WWW.as_bytes()) {
        return None;
    }
    let after_allowed = node_start
        || at.checked_sub(1).is_none_or(|before| {
            matches!(bytes[before], b'*' | b'_' | b'~' | b'(')
                || bytes[before].is_ascii_whitespace()
        });
    if !after_allowed {
        return None;
    }
    let domain_end = domain(text, at, end, true)?;
    let link_end = trim(text, at, path_end(text, domain_end, end));
    (link_end > at + WWW.len()).then(|| (at..link_end, format!("https://{}", &text[at..link_end])))
}

/// An e-mail address whose `@` is at `at`: a local part of alphanumerics and `.-_+` back to `floor` at most, and
/// a domain of alphanumeric, `-` and `_` segments with at least one period, not ending in `-` or `_`.
fn email(text: &str, floor: usize, at: usize, end: usize) -> Option<(Range<usize>, String)> {
    let bytes = text.as_bytes();
    let local = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+');
    let start = at
        - bytes[floor..at]
            .iter()
            .rev()
            .take_while(|b| local(b))
            .count();
    if start == at {
        return None;
    }
    let label =
        |i: usize| i < end && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b'_'));
    let mut i = at + 1;
    let mut periods = 0;
    while i < end {
        if label(i) {
            i += 1;
        } else if bytes[i] == b'.' && i > at + 1 && label(i + 1) {
            periods += 1;
            i += 1;
        } else {
            break;
        }
    }
    if periods == 0 || matches!(bytes[i - 1], b'-' | b'_') {
        return None;
    }
    Some((start..i, format!("mailto:{}", &text[start..i])))
}

/// The end of the domain starting at `start`: segments of alphanumerics (any script), `-` and `_` separated by
/// single periods, with no `_` in the last two segments and, if `need_period`, at least one period. `None` if
/// there is no such domain.
fn domain(text: &str, start: usize, end: usize, need_period: bool) -> Option<usize> {
    let bytes = text.as_bytes();
    // The length of the domain char at `i`, if it is one.
    let char_len = |i: usize| -> Option<usize> {
        let b = *bytes.get(i).filter(|_| i < end)?;
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_') {
            Some(1)
        } else if b.is_ascii() {
            None
        } else {
            text[i..end]
                .chars()
                .next()
                .filter(|c| c.is_alphanumeric())
                .map(char::len_utf8)
        }
    };
    let mut i = start;
    let mut periods = 0;
    // Underscores in the segment before the last and in the last.
    let (mut previous_underscores, mut underscores) = (0, 0);
    while i < end {
        if bytes[i] == b'.' {
            // A period must sit between two segments.
            if i == start || char_len(i + 1).is_none() {
                break;
            }
            periods += 1;
            previous_underscores = underscores;
            underscores = 0;
            i += 1;
        } else if let Some(len) = char_len(i) {
            underscores += usize::from(bytes[i] == b'_');
            i += len;
        } else {
            break;
        }
    }
    let valid =
        i > start && (periods > 0 || !need_period) && previous_underscores == 0 && underscores == 0;
    valid.then_some(i)
}

/// Where the address continuing at `from` ends: at whitespace, `<` or `end`.
fn path_end(text: &str, from: usize, end: usize) -> usize {
    text[from..end]
        .find(|c: char| c.is_whitespace() || c == '<')
        .map_or(end, |i| from + i)
}

/// `end` moved back over what does not belong to the address `start..end`: trailing punctuation, an entity-like
/// `&name;` and closing parentheses beyond the opening ones in the address.
fn trim(text: &str, start: usize, mut end: usize) -> usize {
    let bytes = text.as_bytes();
    let count = |byte: u8| bytes[start..end].iter().filter(|&&b| b == byte).count();
    let opening = count(b'(');
    let mut closing = count(b')');
    while end > start {
        match bytes[end - 1] {
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b'\'' | b'"' => end -= 1,
            b';' => {
                let name = bytes[start..end - 1]
                    .iter()
                    .rev()
                    .take_while(|b| b.is_ascii_alphanumeric())
                    .count();
                let ampersand = end - 1 - name;
                end = if name > 0 && ampersand > start && bytes[ampersand - 1] == b'&' {
                    ampersand - 1
                } else {
                    end - 1
                };
            }
            b')' if closing > opening => {
                closing -= 1;
                end -= 1;
            }
            _ => break,
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::Editor;
    use crate::markdown::tests::{line, options};

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2048))]

        /// Typing addresses in and out keeps the decorations, front matter included, those of a fresh parse.
        /// (That regions find the addresses of the whole document is `regions_parse_like_the_whole_document`.)
        #[test]
        fn edits_keep_the_addresses_of_a_fresh_parse(
            lines in prop::collection::vec(line(), 0..8),
            options in options(),
            edits in prop::collection::vec((0..120usize, 0..8usize, line()), 0..12),
        ) {
            let mut editor = Editor::from_text(&lines.join("\n"));
            let mut markdown = MarkdownState::with_options(editor.buffer(), options);
            for (at, len, text) in edits {
                markdown.decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end());
                editor.replace_range(ByteOffset(at)..ByteOffset(at + len), &text);
            }
            let buffer = editor.buffer();
            let all = ByteOffset(0)..buffer.end();
            let fresh = MarkdownState::with_options(buffer, options).decorations(buffer, all.clone());
            prop_assert_eq!(markdown.decorations(buffer, all), fresh);
        }
    }

    #[test]
    fn web_addresses_being_typed_look_like_addresses() {
        for run in [
            "www.",
            "https://",
            "http://x",
            "a@",
            "x@y.z",
            "(https://a.b)",
        ] {
            assert!(looks_like_web_address(run), "{run}");
        }
        for run in ["word", "www", "a.b", "x:y", "->"] {
            assert!(!looks_like_web_address(run), "{run}");
        }
    }

    /// Whatever typed replacements take for an address being typed is scanned for addresses, so the two rules
    /// cannot disagree about where one may start.
    #[test]
    fn runs_that_look_like_addresses_are_scanned() {
        for run in [
            "www.",
            "https://",
            "a@",
            "x@y.z",
            "(https://a.b)",
            "www.x.com/p",
        ] {
            assert!(
                looks_like_web_address(run) && may_contain_address(run),
                "{run}"
            );
        }
    }

    #[test]
    fn text_without_a_colon_at_sign_or_www_is_not_scanned() {
        assert!(!may_contain_address(
            "Plain prose, with café and 日本語. www"
        ));
        assert!(may_contain_address("time: 10"));
        assert!(may_contain_address("a@b"));
        assert!(may_contain_address("see www.x"));
    }
}
