//! Highlights (`==text==`) and emoji shortcodes (`:smile:`), found in the text of a region's parse without
//! changing it (ADR 0190). pulldown-cmark 0.13 has neither, so [`TextMarks`] watches the parse's events the way
//! [`Autolinks`](super::autolink::Autolinks) does and scans only text: runs of consecutive `Text` events outside
//! code blocks. Code spans, math, raw HTML, autolinks and link destinations are other events, or are not events
//! at all, so they are never scanned; nor is the text of autolinks and wikilinks (an address or a page name), and
//! emoji are not looked for in any link's text.
//!
//! `==` pairs like an emphasis delimiter run (CommonMark §6.2): a run of exactly two `=` (not escaped) opens if
//! it is left-flanking and closes if it is right-flanking, with the nearest opener before it in the same inline
//! element: a highlight never crosses the edge of another inline element (`*a ==b* c==` has none), but may
//! enclose one (`==a *b* c==`). Runs of one or three or more `=` are text, and setext underlines are not text at
//! all. Everything looked at is ASCII, so all ranges are char boundaries.
//!
//! A run with neither `=` nor `:` costs one pass over its bytes and allocates nothing.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Tag, TagEnd};

use super::autolink::is_reference;
use super::{Decoration, DecorationKind, MarkdownOptions, emoji};
use crate::coords::ByteOffset;

/// Collects the highlights and emoji shortcodes of one region's text while its events go by.
#[derive(Debug)]
pub(super) struct TextMarks {
    highlights: bool,
    emoji: bool,
    /// The run of consecutive text events seen last, not yet scanned.
    run: Option<Range<usize>>,
    /// How many code blocks the next event is in.
    code_blocks: usize,
    /// How deep in inline elements (emphasis, links, ...) of the current block the next event is.
    depth: usize,
    /// The depth from which highlights, and emoji, are not looked for: inside an autolink or wikilink (and for
    /// emoji, any link or image).
    no_highlights_from: Option<usize>,
    no_emoji_from: Option<usize>,
    /// The links and images the next event is in, innermost last, with, for a reference link, where the events
    /// inside it end so far.
    links: Vec<Option<usize>>,
    /// The `==` openers not yet closed, with the depth they were found at, innermost last. Empty (and never
    /// allocated) in text without `==`.
    openers: Vec<(usize, usize)>,
    found: Vec<Decoration>,
}

impl TextMarks {
    pub(super) fn new(options: MarkdownOptions) -> Self {
        Self {
            highlights: options.highlights,
            emoji: options.emoji_shortcodes,
            run: None,
            code_blocks: 0,
            depth: 0,
            no_highlights_from: None,
            no_emoji_from: None,
            links: Vec::new(),
            openers: Vec::new(),
            found: Vec::new(),
        }
    }

    /// Whether there is nothing to look for, so events need not be watched at all.
    pub(super) fn off(&self) -> bool {
        !self.highlights && !self.emoji
    }

    /// Takes the next event of the parse of `text`, with its range. Text from `text_end` on is not prose (a
    /// heading's attribute block) and is not scanned. `addresses` are the bare addresses found so far, in
    /// document order: nothing is found inside them.
    pub(super) fn event(
        &mut self,
        event: &Event<'_>,
        range: &Range<usize>,
        text: &str,
        text_end: Option<usize>,
        addresses: &[Decoration],
    ) {
        if self.off() {
            return;
        }
        let range = &(range.start..range.end.min(text_end.unwrap_or(usize::MAX)));
        let link_end = matches!(event, Event::End(TagEnd::Link | TagEnd::Image));
        if let Some(Some(end)) = self.links.last_mut()
            && !link_end
        {
            *end = (*end).max(range.end);
        }
        match event {
            Event::Text(_) if self.code_blocks == 0 => {
                self.text(text, range.clone(), addresses);
                return;
            }
            // Reference links stay text (ADR 0040): their brackets are text here, as without a definition, and
            // they are no element that highlights may not cross (see `autolink::is_reference`).
            Event::Start(Tag::Link { link_type, .. } | Tag::Image { link_type, .. })
                if is_reference(*link_type) =>
            {
                let opener = if matches!(event, Event::Start(Tag::Image { .. })) {
                    2
                } else {
                    1
                };
                let opener = range.start..(range.start + opener).min(range.end);
                self.links.push(Some(opener.end));
                self.text(text, opener, addresses);
                return;
            }
            Event::End(TagEnd::Link | TagEnd::Image)
                if matches!(self.links.last(), Some(Some(_))) =>
            {
                if let Some(Some(inner_end)) = self.links.pop() {
                    self.text(text, inner_end.min(range.end)..range.end, addresses);
                }
                return;
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => self.links.push(None),
            Event::End(TagEnd::Link | TagEnd::Image) => {
                self.links.pop();
            }
            _ => {}
        }
        self.flush(text, addresses);
        match event {
            Event::Start(tag) if inline(tag) => {
                self.depth += 1;
                let (highlights, emoji) = match tag {
                    Tag::Link {
                        link_type: LinkType::Autolink | LinkType::Email | LinkType::WikiLink { .. },
                        ..
                    } => (false, false),
                    Tag::Link { .. } | Tag::Image { .. } => (true, false),
                    _ => (true, true),
                };
                if !highlights {
                    self.no_highlights_from.get_or_insert(self.depth);
                }
                if !emoji {
                    self.no_emoji_from.get_or_insert(self.depth);
                }
            }
            Event::End(tag) if inline_end(tag) => {
                // Openers inside the element that ends stay text.
                while self
                    .openers
                    .last()
                    .is_some_and(|&(depth, _)| depth >= self.depth)
                {
                    self.openers.pop();
                }
                if self.no_highlights_from == Some(self.depth) {
                    self.no_highlights_from = None;
                }
                if self.no_emoji_from == Some(self.depth) {
                    self.no_emoji_from = None;
                }
                self.depth = self.depth.saturating_sub(1);
            }
            // A block starts or ends: nothing pairs across it.
            Event::Start(tag) => {
                if let Tag::CodeBlock(_) = tag {
                    self.code_blocks += 1;
                }
                self.reset();
            }
            Event::End(tag) => {
                if let TagEnd::CodeBlock = tag {
                    self.code_blocks = self.code_blocks.saturating_sub(1);
                }
                self.reset();
            }
            // Code spans, math, inline HTML and line breaks: inside a highlight, not text to scan.
            _ => {}
        }
    }

    /// The decorations found in `text`, in no particular order.
    pub(super) fn finish(mut self, text: &str, addresses: &[Decoration]) -> Vec<Decoration> {
        self.flush(text, addresses);
        self.found
    }

    /// Adds `piece` of `text` to the run of text it follows, or scans that run and starts a new one.
    fn text(&mut self, text: &str, piece: Range<usize>, addresses: &[Decoration]) {
        if piece.is_empty() {
            return;
        }
        match &mut self.run {
            // Entities, escapes and unmatched delimiters split text into several events.
            Some(run) if run.end == piece.start => run.end = piece.end,
            _ => {
                self.flush(text, addresses);
                self.run = Some(piece);
            }
        }
    }

    fn reset(&mut self) {
        self.links.clear();
        self.depth = 0;
        self.no_highlights_from = None;
        self.no_emoji_from = None;
        self.openers.clear();
    }

    fn flush(&mut self, text: &str, addresses: &[Decoration]) {
        let Some(run) = self.run.take() else {
            return;
        };
        let highlights = self.highlights && self.no_highlights_from.is_none();
        let emoji = self.emoji && self.no_emoji_from.is_none();
        let bytes = text.as_bytes();
        // The quick check, without which most prose is scanned no further: `<[u8]>::contains` is a memchr.
        let run_bytes = &bytes[run.clone()];
        if !(highlights && run_bytes.contains(&b'=') || emoji && run_bytes.contains(&b':')) {
            return;
        }
        // Addresses lie in text runs, so those overlapping this run start in it or end in it.
        let first = addresses.partition_point(|d| d.range.end.0 <= run.start);
        let addresses = &addresses[first..];
        let in_address = |at: usize| {
            addresses
                .iter()
                .take_while(|d| d.range.start.0 <= at)
                .any(|d| d.range.start.0 <= at && at < d.range.end.0)
        };
        let mut i = run.start;
        while i < run.end {
            match bytes[i] {
                b'=' if highlights => {
                    let end = i + bytes[i..].iter().take_while(|&&b| b == b'=').count();
                    if end - i == 2 && end <= run.end && !escaped(bytes, i) && !in_address(i) {
                        self.delimiter(text, i);
                    }
                    i = end;
                }
                b':' if emoji && !in_address(i) => match emoji::shortcode_at(text, i, run.end) {
                    Some((range, found)) if !in_address(range.end - 1) => {
                        i = range.end;
                        self.found.push(emoji::decoration(range, found));
                    }
                    _ => i += 1,
                },
                _ => i += 1,
            }
        }
    }

    /// The `==` at `at` of `text`: closes the nearest open highlight of this element if it can, or opens one.
    fn delimiter(&mut self, text: &str, at: usize) {
        let before = text[..at].chars().next_back();
        let after = text[at + 2..].chars().next();
        let (left, right) = (flanking(after, before), flanking(before, after));
        let open = self
            .openers
            .last()
            .filter(|&&(depth, _)| depth == self.depth)
            .map(|&(_, start)| start);
        match open {
            Some(start) if right => {
                self.openers.pop();
                let markers = vec![offsets(start..start + 2), offsets(at..at + 2)];
                self.found.push(Decoration {
                    kind: DecorationKind::Highlight,
                    range: offsets(start..at + 2),
                    content: offsets(start + 2..at),
                    markers,
                });
            }
            _ if left => self.openers.push((self.depth, at)),
            _ => {}
        }
    }
}

/// Whether a delimiter run with `next` on one side and `previous` on the other is flanking on the `next` side
/// (left-flanking with the char after it as `next`, right-flanking with the char before it): `next` is not
/// whitespace, and is not punctuation or `previous` is whitespace or punctuation. The edge of the text counts as
/// whitespace. Chars other than letters, digits and whitespace count as punctuation.
fn flanking(next: Option<char>, previous: Option<char>) -> bool {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    let punctuation =
        |c: Option<char>| c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace());
    !space(next) && (!punctuation(next) || space(previous) || punctuation(previous))
}

/// Whether the byte at `at` is escaped: an odd number of backslashes come right before it.
fn escaped(bytes: &[u8], at: usize) -> bool {
    bytes[..at]
        .iter()
        .rev()
        .take_while(|&&b| b == b'\\')
        .count()
        % 2
        == 1
}

/// Inline elements, within which highlights pair and outside which they do not.
fn inline(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

fn inline_end(tag: &TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link
            | TagEnd::Image
    )
}

fn offsets(range: Range<usize>) -> Range<ByteOffset> {
    ByteOffset(range.start)..ByteOffset(range.end)
}
