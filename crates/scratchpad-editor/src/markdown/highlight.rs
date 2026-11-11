//! Highlights (`==text==`) and emoji shortcodes (`:smile:`), found in the text of a region's parse without
//! changing it (ADR 0190). pulldown-cmark 0.13 has neither, so [`TextMarks`] scans the text runs of the walk it
//! shares with the bare-address scanner (`text_runs`): runs of consecutive `Text` events outside code blocks.
//! Code spans, math, raw HTML and link destinations are other events, or are not events at all, so they are
//! never scanned; nor is the text of autolinks and wikilinks (an address or a page name), and emoji are not
//! looked for in the text of inline links and images either. Reference links are read as text (their
//! definitions may be in another region), so their text is prose: `[:smile:][r]` shows the emoji whether or not
//! `r` is defined.
//!
//! `==` pairs like an emphasis delimiter run of `_` (CommonMark §6.2): a run of exactly two `=` (not escaped)
//! opens if it is left-flanking and, when also right-flanking, comes after punctuation; it closes if it is
//! right-flanking and, when also left-flanking, comes before punctuation. So `==a==` and `a ==b== c` highlight,
//! but `x==1 and y==2` does not. It closes the nearest opener before it in the same inline element: a highlight
//! never crosses the edge of another inline element (`*a ==b* c==` has none), but may enclose one
//! (`==a *b* c==`). Runs of one or three or more `=` are text, and setext underlines are not text at all.
//! Everything looked at is ASCII, so all ranges are char boundaries.
//!
//! A run with neither `=` nor `:` costs a memchr over its bytes and allocates nothing.

use std::ops::Range;

use super::text_runs::Step;
use super::{Decoration, DecorationKind, MarkdownOptions, emoji};
use crate::coords::ByteOffset;

/// Collects the highlights and emoji shortcodes of one region's text from the walk's steps.
#[derive(Debug)]
pub(super) struct TextMarks {
    highlights: bool,
    emoji: bool,
    /// How deep in inline elements (emphasis, links, ...) of the current block the next step is.
    depth: usize,
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
            depth: 0,
            openers: Vec::new(),
            found: Vec::new(),
        }
    }

    /// Takes the next step of the walk over the parse of `text`. `addresses` are the bare addresses found so far,
    /// in document order: nothing is found inside them.
    pub(super) fn step(&mut self, text: &str, step: &Step, addresses: &[Decoration]) {
        if !self.highlights && !self.emoji {
            return;
        }
        match step {
            Step::Run { range, context } => {
                let highlights = self.highlights && !context.in_address_or_page;
                let emoji = self.emoji && !context.in_link;
                self.scan(text, range.clone(), highlights, emoji, addresses);
            }
            Step::Enter => self.depth += 1,
            Step::Leave => {
                // Openers inside the element that ends stay text.
                while self
                    .openers
                    .last()
                    .is_some_and(|&(depth, _)| depth >= self.depth)
                {
                    self.openers.pop();
                }
                self.depth = self.depth.saturating_sub(1);
            }
            Step::Block => {
                self.depth = 0;
                self.openers.clear();
            }
        }
    }

    /// The decorations found in `text`, in no particular order.
    pub(super) fn finish(self) -> Vec<Decoration> {
        self.found
    }

    /// Scans the text run `run` of `text` for highlight delimiters and shortcodes, as wanted.
    fn scan(
        &mut self,
        text: &str,
        run: Range<usize>,
        highlights: bool,
        emoji: bool,
        addresses: &[Decoration],
    ) {
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
        // The rule of `_`: a run flanked on both sides opens only after punctuation and closes only before it.
        let can_open = left && (!right || punctuation(before));
        let can_close = right && (!left || punctuation(after));
        let open = self
            .openers
            .last()
            .filter(|&&(depth, _)| depth == self.depth)
            .map(|&(_, start)| start);
        match open {
            Some(start) if can_close => {
                self.openers.pop();
                let markers = vec![offsets(start..start + 2), offsets(at..at + 2)];
                self.found.push(Decoration {
                    kind: DecorationKind::Highlight,
                    range: offsets(start..at + 2),
                    content: offsets(start + 2..at),
                    markers,
                });
            }
            _ if can_open => self.openers.push((self.depth, at)),
            _ => {}
        }
    }
}

/// Whether a delimiter run with `next` on one side and `previous` on the other is flanking on the `next` side
/// (left-flanking with the char after it as `next`, right-flanking with the char before it): `next` is not
/// whitespace, and is not punctuation or `previous` is whitespace or punctuation. The edge of the text counts as
/// whitespace.
fn flanking(next: Option<char>, previous: Option<char>) -> bool {
    let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
    !space(next) && (!punctuation(next) || space(previous) || punctuation(previous))
}

/// Chars other than letters, digits and whitespace count as punctuation; the edge of the text does not.
fn punctuation(c: Option<char>) -> bool {
    c.is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace())
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

fn offsets(range: Range<usize>) -> Range<ByteOffset> {
    ByteOffset(range.start)..ByteOffset(range.end)
}
