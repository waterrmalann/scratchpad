//! Markdown decorations (PLAN §17–21): styling information that refers to byte ranges of the buffer and never
//! replaces its text.
//!
//! [`MarkdownState`] splits the document into block regions (ADR 0041) and parses a region with pulldown-cmark
//! (ADR 0040) only when something asks for its decorations. A renderer asks for [`StyledLine`]s of the visible
//! lines, which also say which syntax markers live preview hides around the selection (ADR 0042).

mod blocks;
mod parse;
mod style;

pub use style::{MarkerKind, SpanStyle, StyledLine, StyledSpan};

use std::borrow::Cow;
use std::ops::Range;

use crate::buffer::{Buffer, TextChange};
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
        state.rescan_all(buffer);
        state
    }

    /// Catches up with the edits made to `buffer` since the last call, rescanning only the regions around them
    /// and dropping only their parse results (ADR 0041). Every query calls this first.
    fn sync(&mut self, buffer: &Buffer) {
        if self.version == buffer.version() && self.len == buffer.len() {
            return;
        }
        let edit = buffer.changes_since(self.version).and_then(combine);
        match edit {
            // The length check catches a buffer other than the one this state was made for.
            Some(edit)
                if (self.len + edit.new_end).checked_sub(edit.old_end) == Some(buffer.len()) =>
            {
                self.rescan_edit(buffer, edit)
            }
            _ => self.rescan_all(buffer),
        }
        self.version = buffer.version();
        self.len = buffer.len();
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

    fn rescan_all(&mut self, buffer: &Buffer) {
        self.regions = std::iter::once(0)
            .chain(region_starts(buffer, 0))
            .map(Region::unparsed)
            .collect();
    }

    /// Rescans from the region before the edit until a region start that existed before the edit is found again
    /// after it. Region starts depend only on the text from the previous region start, so every region from
    /// there on is unchanged except for its position.
    fn rescan_edit(&mut self, buffer: &Buffer, edit: Edit) {
        let edited = self.region_index(edit.start);
        // An edit at the start of a region can join it to the previous one, e.g. by indenting its first line.
        let first = edited.saturating_sub(1);
        let scan_start = self.regions[first].start;
        let mut rescanned = vec![Region::unparsed(scan_start)];
        let mut kept_from = self.regions.len();
        let mut old = edited + 1;
        for start in region_starts(buffer, scan_start) {
            if start >= edit.new_end {
                let old_start = start - edit.new_end + edit.old_end;
                while old < self.regions.len() && self.regions[old].start < old_start {
                    old += 1;
                }
                if old < self.regions.len() && self.regions[old].start == old_start {
                    kept_from = old;
                    break;
                }
            }
            rescanned.push(Region::unparsed(start));
        }

        // The region before the edited one keeps its parse if it still ends where it did.
        let first_end = rescanned.get(1).map(|r| r.start).or_else(|| {
            let kept = self.regions.get(kept_from)?;
            Some(kept.start + edit.new_end - edit.old_end)
        });
        if first < edited && first_end == Some(self.regions[edited].start) {
            rescanned[0].decorations = self.regions[first].decorations.take();
        }

        for region in &mut self.regions[kept_from..] {
            region.start = region.start + edit.new_end - edit.old_end;
        }
        self.regions.splice(first..kept_from, rescanned);
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

impl Region {
    fn unparsed(start: usize) -> Self {
        Self {
            start,
            decorations: None,
        }
    }
}

/// The starts of the regions after the one starting at `from`, which must be a region start.
fn region_starts(buffer: &Buffer, from: usize) -> impl Iterator<Item = usize> {
    let mut scanner = blocks::Scanner::default();
    let mut offset = from;
    buffer
        .lines_from(buffer.line_of(ByteOffset(from)))
        .filter_map(move |line| {
            let start = offset;
            offset += line.len_bytes();
            let text: Cow<'_, str> = line.into();
            scanner
                .starts_region(text.strip_suffix('\n').unwrap_or(&text))
                .then_some(start)
        })
}

/// Consecutive buffer changes merged into one replacement: `start..old_end` of the old text became
/// `start..new_end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edit {
    start: usize,
    old_end: usize,
    new_end: usize,
}

/// Merges changes given oldest first; `None` if there are none.
fn combine<'a>(changes: impl Iterator<Item = &'a TextChange>) -> Option<Edit> {
    changes.fold(None, |merged, change| {
        let (start, old_end, new_end) = (change.start.0, change.old_end.0, change.new_end.0);
        let Some(merged) = merged else {
            return Some(Edit {
                start,
                old_end,
                new_end,
            });
        };
        // `change` is in the coordinates of the text after `merged`, which past `merged.new_end` differ from the
        // old text's by `merged`'s change in length.
        let combined_old_end = if old_end > merged.new_end {
            merged.old_end + (old_end - merged.new_end)
        } else {
            merged.old_end
        };
        Some(Edit {
            start: start.min(merged.start),
            old_end: combined_old_end,
            new_end: combined_old_end + merged.new_end + new_end - merged.old_end - old_end,
        })
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::Editor;

    /// Fragments that open, close and separate regions, mixed with inline syntax and text.
    const ATOMS: &[&str] = &[
        "```", "~~~", "\n", "\n\n", "**", "# ", "- ", "1. ", "2. ", "> ", "  ", "    ", "\t",
        "<!--", "-->", "<p>", "a", "word ", "`", "*", "é",
    ];

    #[derive(Debug, Clone)]
    enum Op {
        Replace(usize, usize, String),
        /// Several replacements, recorded as separate buffer changes before the next query.
        Transaction(Vec<(usize, usize, String)>),
        Undo,
        Redo,
        /// Queries the decorations near an offset, which syncs and parses the regions there.
        Query(usize),
    }

    fn atoms(max: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(ATOMS), 0..max).prop_map(|atoms| atoms.concat())
    }

    fn replacement() -> impl Strategy<Value = (usize, usize, String)> {
        (0..200usize, 0..20usize, atoms(4)).prop_map(|(at, len, text)| (at, at + len, text))
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => replacement().prop_map(|(a, b, text)| Op::Replace(a, b, text)),
            1 => prop::collection::vec(replacement(), 1..4).prop_map(Op::Transaction),
            1 => Just(Op::Undo),
            1 => Just(Op::Redo),
            3 => (0..200usize).prop_map(Op::Query),
        ]
    }

    fn region_starts_of(state: &MarkdownState) -> Vec<usize> {
        state.regions.iter().map(|r| r.start).collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2048))]

        /// Incremental updates, with parse results kept across edits, give exactly the regions and decorations
        /// of parsing the edited document from scratch.
        #[test]
        fn incremental_updates_equal_a_full_reparse(
            text in atoms(40),
            ops in prop::collection::vec(op(), 0..20),
        ) {
            let mut editor = Editor::from_text(&text);
            let mut markdown = MarkdownState::new(editor.buffer());
            markdown.decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end());
            for op in &ops {
                match op {
                    Op::Replace(a, b, text) => editor.replace_range(ByteOffset(*a)..ByteOffset(*b), text),
                    Op::Transaction(edits) => editor.transact(|editor| {
                        for (a, b, text) in edits {
                            editor.replace_range(ByteOffset(*a)..ByteOffset(*b), text);
                        }
                    }),
                    Op::Undo => {
                        editor.undo();
                    }
                    Op::Redo => {
                        editor.redo();
                    }
                    Op::Query(at) => {
                        markdown.decorations(editor.buffer(), ByteOffset(*at)..ByteOffset(at + 30));
                    }
                }
            }
            let buffer = editor.buffer();
            let all = ByteOffset(0)..buffer.end();
            let incremental = markdown.decorations(buffer, all.clone());
            let mut fresh = MarkdownState::new(buffer);
            prop_assert_eq!(region_starts_of(&markdown), region_starts_of(&fresh));
            prop_assert_eq!(incremental, fresh.decorations(buffer, all));
        }
    }

    /// How lines may start: containers, block openers and their lookalikes, at various indentations.
    const LINE_STARTS: &[&str] = &[
        "", "", "", "  ", "    ", "\t", "# ", "## ", "- ", "* ", "+ ", "1. ", "2. ", "1) ", "3) ",
        "> ", "> > ", "- [ ] ", "```", "~~~", "````", "   ```", "<!--", "<div>", "===", "---",
        "- - -", "-", "1.", "> - ", "- > ", "  - ", "   1. ", "-     ",
    ];
    /// What follows on the line: text, inline syntax and fragments that open or close blocks.
    const LINE_REST: &[&str] = &[
        "a", "word", "**", "*", "_", "`", "~~", "-->", "```", "é", "[x](y)", "<b>", " ", "#",
        "1. ", "- ", "> ",
    ];

    fn line() -> impl Strategy<Value = String> {
        (
            prop::sample::select(LINE_STARTS),
            prop::collection::vec(prop::sample::select(LINE_REST), 0..4),
        )
            .prop_map(|(start, rest)| format!("{start}{}", rest.concat()))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]

        /// Region boundaries never change the meaning of the document: parsing region by region gives the
        /// decorations of parsing the whole text at once.
        #[test]
        fn regions_parse_like_the_whole_document(lines in prop::collection::vec(line(), 0..14)) {
            let text = lines.join("\n");
            let buffer = Buffer::from_text(&text);
            let mut markdown = MarkdownState::new(&buffer);
            let by_region = markdown.decorations(&buffer, ByteOffset(0)..buffer.end());
            prop_assert_eq!(by_region, parse::parse(&text), "{:?} split at {:?}", text, region_starts_of(&markdown));
        }
    }

    #[test]
    fn combined_changes_cover_every_change() {
        let mut buffer = Buffer::from_text("0123456789");
        buffer.replace(5..7, "abcd"); // 01234abcd789
        buffer.replace(1..2, ""); // 0234abcd789
        buffer.replace(10..11, "XY"); // 0234abcd78XY
        let edit = combine(buffer.changes_since(0).unwrap());
        assert_eq!(
            edit,
            Some(Edit {
                start: 1,
                old_end: 10,
                new_end: 12
            })
        );
        assert_eq!(combine(buffer.changes_since(3).unwrap()), None);
    }

    #[test]
    fn typing_inside_a_region_reparses_only_that_region() {
        let mut editor = Editor::from_text("# A\n\n*b*\n\n`c`\n\nd");
        let mut markdown = MarkdownState::new(editor.buffer());
        markdown.decorations(editor.buffer(), ByteOffset(0)..editor.buffer().end());
        editor.move_to(ByteOffset(7), false);
        editor.insert_text("x");
        markdown.sync(editor.buffer());
        let parsed: Vec<bool> = markdown
            .regions
            .iter()
            .map(|r| r.decorations.is_some())
            .collect();
        // The edited region and the one before it are rescanned; the one before keeps its parse because it
        // still ends where it did, and the regions after are kept.
        assert_eq!(parsed, [true, false, true, true]);
    }

    /// Cuts in long runs of lines depend on the distance from the previous region start, which edits that add
    /// or remove lines shift for every later cut.
    #[test]
    fn incremental_updates_move_the_cuts_in_long_runs() {
        let mut editor = Editor::from_text(&vec!["t *x*"; 1000].join("\n"));
        let mut markdown = MarkdownState::new(editor.buffer());
        for (at, len, text) in [
            (60, 0, "\n"),
            (60, 6, ""),
            (1500, 0, "```"),
            (3000, 0, "a"),
            (1500, 3, ""),
            (2000, 12, ""),
        ] {
            editor.replace_range(ByteOffset(at)..ByteOffset(at + len), text);
            let buffer = editor.buffer();
            let incremental = markdown.decorations(buffer, ByteOffset(0)..buffer.end());
            let mut fresh = MarkdownState::new(buffer);
            assert_eq!(region_starts_of(&markdown), region_starts_of(&fresh));
            assert!(region_starts_of(&markdown).len() > 3);
            assert_eq!(
                incremental,
                fresh.decorations(buffer, ByteOffset(0)..buffer.end())
            );
        }
    }
}
