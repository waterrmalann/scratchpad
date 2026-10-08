//! Markdown decorations (PLAN §17–21): styling information that refers to byte ranges of the buffer and never
//! replaces its text.
//!
//! [`MarkdownState`] splits the document into block regions (ADR 0041) and parses a region with pulldown-cmark
//! (ADR 0040) only when something asks for its decorations.

mod blocks;
mod parse;

use std::ops::Range;

use crate::buffer::Buffer;
use crate::coords::ByteOffset;

/// A Markdown construct found in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoration {
    pub kind: DecorationKind,
    /// The whole construct, markers included. Block ranges end at the end of their last line, before its line
    /// break.
    pub range: Range<ByteOffset>,
    /// The text the construct styles: `range` without its leading and trailing markers.
    pub content: Range<ByteOffset>,
    /// The syntax markers in document order, e.g. both `**` of a strong span, `## ` of a heading, `[` and
    /// `](url)` of a link, the fence lines of a code block or the `> ` on each line of a block quote. They lie
    /// within `range` and never overlap.
    pub markers: Vec<Range<ByteOffset>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecorationKind {
    /// An ATX (`## Title`) or setext (`Title` underlined with `===`/`---`) heading, level 1–6.
    Heading {
        level: u8,
    },
    Strong,
    Emphasis,
    Strikethrough,
    InlineCode,
    /// An inline link `[text](destination)` or an autolink `<destination>`.
    Link {
        destination: String,
    },
    BlockQuote,
    /// A list item; its marker is the bullet or number with the spaces after it.
    ListItem {
        ordered: bool,
    },
    /// The `[ ]`/`[x]` box of a task list item and the rest of the item.
    Task {
        checked: bool,
    },
    ThematicBreak,
    /// A fenced or indented code block. `info` is the fence's info string, e.g. the language.
    CodeBlock {
        info: String,
    },
}

impl Decoration {
    fn shifted(&self, by: usize) -> Self {
        let shift = |range: &Range<ByteOffset>| {
            ByteOffset(range.start.0 + by)..ByteOffset(range.end.0 + by)
        };
        Self {
            kind: self.kind.clone(),
            range: shift(&self.range),
            content: shift(&self.content),
            markers: self.markers.iter().map(shift).collect(),
        }
    }
}

/// The Markdown structure of one document, kept in step with its [`Buffer`].
///
/// Create one per document and pass the same buffer to every call: each query first catches up with the buffer's
/// edits. Create a new one when the view switches to another document.
#[derive(Debug, Clone)]
pub struct MarkdownState {
    /// The buffer version and length the regions describe.
    version: u64,
    len: usize,
    /// Block regions covering the document in order. Never empty; the first starts at 0.
    regions: Vec<Region>,
}

#[derive(Debug, Clone)]
struct Region {
    start: usize,
    /// Parsed on first use, with offsets relative to `start`.
    decorations: Option<Vec<Decoration>>,
}

impl MarkdownState {
    pub fn new(buffer: &Buffer) -> Self {
        let mut state = Self {
            version: buffer.version(),
            len: buffer.len(),
            regions: Vec::new(),
        };
        state.rescan(buffer);
        state
    }

    /// Catches up with the edits made to `buffer` since the last call.
    pub fn sync(&mut self, buffer: &Buffer) {
        if self.version == buffer.version() && self.len == buffer.len() {
            return;
        }
        self.version = buffer.version();
        self.len = buffer.len();
        self.rescan(buffer);
    }

    /// The decorations that intersect or touch `range`, ordered by start, enclosing ones first.
    pub fn decorations(&mut self, buffer: &Buffer, range: Range<ByteOffset>) -> Vec<Decoration> {
        self.sync(buffer);
        let mut found = Vec::new();
        let first = self.region_index(range.start.0);
        for index in first..self.regions.len() {
            let start = self.regions[index].start;
            if start > range.end.0 {
                break;
            }
            let touches = |d: &&Decoration| {
                d.range.start.0 + start <= range.end.0 && range.start.0 <= d.range.end.0 + start
            };
            found.extend(
                self.region_decorations(buffer, index)
                    .iter()
                    .filter(touches)
                    .map(|d| d.shifted(start)),
            );
        }
        found
    }

    /// The destination of the link whose text or markers contain the char at `offset`, for opening it on
    /// Ctrl+click.
    pub fn link_at(&mut self, buffer: &Buffer, offset: ByteOffset) -> Option<String> {
        self.decorations(buffer, offset..offset)
            .into_iter()
            .find_map(|d| match d.kind {
                DecorationKind::Link { destination } if offset < d.range.end => Some(destination),
                _ => None,
            })
    }

    fn rescan(&mut self, buffer: &Buffer) {
        let mut scanner = blocks::Scanner::default();
        let mut start = 0;
        self.regions.clear();
        self.regions.push(Region {
            start: 0,
            decorations: None,
        });
        for line in buffer.lines_from(0) {
            let len = line.len_bytes();
            let text: std::borrow::Cow<'_, str> = line.into();
            if scanner.starts_region(text.strip_suffix('\n').unwrap_or(&text)) {
                self.regions.push(Region {
                    start,
                    decorations: None,
                });
            }
            start += len;
        }
    }

    /// The index of the region containing `offset`.
    fn region_index(&self, offset: usize) -> usize {
        self.regions
            .partition_point(|r| r.start <= offset)
            .saturating_sub(1)
    }

    fn region_end(&self, index: usize) -> usize {
        self.regions.get(index + 1).map_or(self.len, |r| r.start)
    }

    fn region_decorations(&mut self, buffer: &Buffer, index: usize) -> &[Decoration] {
        let range = ByteOffset(self.regions[index].start)..ByteOffset(self.region_end(index));
        self.regions[index]
            .decorations
            .get_or_insert_with(|| parse::parse(&buffer.text_for_range(range)))
    }
}
