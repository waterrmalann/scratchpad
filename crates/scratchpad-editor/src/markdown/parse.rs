//! Turns the text of one block region into [`Decoration`]s (ADR 0040).
//!
//! pulldown-cmark reports the range of every element; the syntax marker ranges inside them (`**`, `# `, `](url)`,
//! fence lines, ...) are derived here from the text. Every marker consists of ASCII bytes found by scanning
//! inside an element range, so all derived offsets are char boundaries. Offsets are relative to the region text.

// Marker lists are vectors of ranges that often start with a single marker, which is what this lint flags.
#![allow(clippy::single_range_in_vec_init)]

use std::ops::Range;

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd,
};

use super::table::{self, TableState};
use super::{CalloutKind, Decoration, DecorationKind, MarkdownOptions};
use crate::coords::ByteOffset;

/// Parses `text` with `options` and returns its decorations ordered by start, enclosing decorations before enclosed
/// ones.
pub(super) fn parse(text: &str, options: MarkdownOptions) -> Vec<Decoration> {
    let mut builder = Builder {
        text,
        options,
        decorations: Vec::new(),
        quotes: Vec::new(),
        items: Vec::new(),
        links: Vec::new(),
        table: TableState::default(),
    };
    for (event, range) in Parser::new_ext(text, pulldown_options(options)).into_offset_iter() {
        builder.event(event, range);
    }
    // Events come in document order except a task box, which follows the start of the item's first block, e.g.
    // a setext heading.
    let mut decorations = builder.decorations;
    decorations.sort_by_key(|d| (d.range.start, std::cmp::Reverse(d.range.end)));
    decorations
}

/// The pulldown-cmark options for `options`. In pulldown-cmark 0.13, `ENABLE_GFM` only adds the alert tags of
/// block quotes; check its documentation when upgrading, since anything else it turns on would change the meaning
/// of notes with callouts on (ADR 0185).
fn pulldown_options(options: MarkdownOptions) -> Options {
    let mut pulldown = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    pulldown.set(Options::ENABLE_GFM, options.callouts);
    pulldown.set(Options::ENABLE_SUPERSCRIPT, options.superscript);
    pulldown.set(Options::ENABLE_SUBSCRIPT, options.subscript);
    pulldown.set(Options::ENABLE_HEADING_ATTRIBUTES, options.heading_ids);
    // GFM tables are always on (ADR 0186).
    pulldown |= Options::ENABLE_TABLES;
    pulldown
}

struct Builder<'a> {
    text: &'a str,
    options: MarkdownOptions,
    decorations: Vec<Decoration>,
    /// Indices of the decorations of the open block quotes, innermost last.
    quotes: Vec<usize>,
    /// Ranges of the open list items, innermost last.
    items: Vec<Range<usize>>,
    /// Open links and images: the index of their decoration (`None` for unsupported link types) and the furthest
    /// end of the events inside them so far, which is where their text ends.
    links: Vec<(Option<usize>, usize)>,
    /// The open table's row and cells.
    table: TableState,
}

impl Builder<'_> {
    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        let link_end = matches!(event, Event::End(TagEnd::Link | TagEnd::Image));
        if let Some((_, text_end)) = self.links.last_mut()
            && !link_end
        {
            *text_end = (*text_end).max(range.end);
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(TagEnd::BlockQuote(_)) => {
                self.quotes.pop();
            }
            Event::End(TagEnd::Item) => {
                self.items.pop();
            }
            _ if link_end => self.end_link(),
            Event::Code(_) => {
                let ticks = self.run_len(range.start, b'`');
                self.delimited(DecorationKind::InlineCode, range, ticks);
            }
            Event::TaskListMarker(checked) => self.task(checked, range),
            Event::Rule => {
                let range = self.trim(range);
                let end = range.end;
                self.push(
                    DecorationKind::ThematicBreak,
                    range.clone(),
                    end..end,
                    vec![range],
                );
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                self.table.end_row(self.text, &mut self.decorations);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Heading { level, .. } => self.heading(level as u8, range),
            Tag::BlockQuote(kind) => self.block_quote(range, kind),
            Tag::CodeBlock(kind) => self.code_block(kind, range),
            Tag::Item => self.item(range),
            Tag::Emphasis => self.delimited(DecorationKind::Emphasis, range, 1),
            Tag::Strong => self.delimited(DecorationKind::Strong, range, 2),
            Tag::Strikethrough => {
                // GFM accepts `~x~` as well as `~~x~~`. A run of tildes closes only a run as long, so the run is
                // the marker; with subscript on, a single `~` is subscript instead and the run is always `~~`.
                let tildes = self.run_len(range.start, b'~').min(2);
                self.delimited(DecorationKind::Strikethrough, range, tildes);
            }
            Tag::Superscript => {
                // `^^x^^` is superscript too, and `^^^x^^^` a `^` around a `^^`: of a run of carets, inner
                // elements take two each and the outermost one takes the odd one left.
                let carets = match self.run_len(range.start, b'^') {
                    0 => 0,
                    run => 2 - run % 2,
                };
                self.delimited(DecorationKind::Superscript, range, carets);
            }
            Tag::Subscript => {
                let tildes = self.run_len(range.start, b'~').min(1);
                self.delimited(DecorationKind::Subscript, range, tildes);
            }
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => self.link(link_type, dest_url, range, 1),
            // Styled and opened like a link; nothing is loaded (ADR 0185).
            Tag::Image {
                link_type,
                dest_url,
                ..
            } => self.link(link_type, dest_url, range, 2),
            Tag::Table(alignments) => self.table_start(range, table::Start::Table(&alignments)),
            Tag::TableHead => self.table_start(range, table::Start::Row { header: true }),
            Tag::TableRow => self.table_start(range, table::Start::Row { header: false }),
            Tag::TableCell => self.table_start(range, table::Start::Cell),
            _ => {}
        }
    }

    /// A link, or an image with `opener` 2 for its `![`.
    fn link(
        &mut self,
        link_type: LinkType,
        destination: CowStr<'_>,
        range: Range<usize>,
        opener: usize,
    ) {
        // Reference links resolve against definitions anywhere in the document, which a region parsed on its own
        // cannot see, so they stay plain text everywhere rather than only sometimes (ADR 0040).
        let supported = matches!(
            link_type,
            LinkType::Inline | LinkType::Autolink | LinkType::Email
        );
        let index = supported.then(|| {
            let kind = DecorationKind::Link {
                destination: destination.to_string(),
            };
            self.push(kind, range.clone(), range.clone(), Vec::new())
        });
        self.links.push((index, range.start + opener));
    }

    fn push(
        &mut self,
        kind: DecorationKind,
        range: Range<usize>,
        content: Range<usize>,
        markers: Vec<Range<usize>>,
    ) -> usize {
        self.decorations.push(Decoration {
            kind,
            range: offsets(range),
            content: offsets(content),
            markers: markers.into_iter().map(offsets).collect(),
        });
        self.decorations.len() - 1
    }

    /// Emphasis, strikethrough and code spans: `len` marker bytes at each end.
    fn delimited(&mut self, kind: DecorationKind, range: Range<usize>, len: usize) {
        if len == 0 || range.len() < 2 * len {
            return;
        }
        let content = range.start + len..range.end - len;
        let markers = vec![range.start..content.start, content.end..range.end];
        self.push(kind, range, content, markers);
    }

    fn heading(&mut self, level: u8, range: Range<usize>) {
        let range = self.trim(range);
        let kind = DecorationKind::Heading { level };
        let hashes_start = self.skip(range.start, range.end, b" ");
        if self.byte(hashes_start) != Some(b'#') {
            // A setext heading: the text, then an underline of `=` or `-` (after any container prefix).
            let Some(newline) = self.text[range.clone()].rfind('\n') else {
                self.push(kind, range.clone(), range, Vec::new());
                return;
            };
            let last_line = range.start + newline + 1;
            let underline = self.text[last_line..range.end]
                .find(['=', '-'])
                .map_or(range.end, |i| last_line + i);
            let mut content = range.start..last_line - 1;
            let mut markers = vec![underline..range.end];
            if let Some(attributes) = self.attributes(content.clone()) {
                markers.insert(0, attributes.start..content.end);
                content.end = attributes.start;
            }
            self.push(kind, range.clone(), content, markers);
            return;
        }

        let hashes_end = self.skip(hashes_start, range.end, b"#");
        let open_end = self.skip(hashes_end, range.end, b" \t");
        let mut markers = vec![range.start..open_end];
        // An attribute block comes last, even after a closing sequence (`# Title ## {#id}`).
        let attributes = self.attributes(open_end..range.end);
        let end = attributes.as_ref().map_or(range.end, |a| a.start);
        // An optional closing sequence: spaces, then `#`s, then only spaces (`# Title ##`).
        let trimmed_end = self.skip_back(end, open_end, b" \t");
        let closing_start = self.skip_back(trimmed_end, open_end, b"#");
        let content_end = if closing_start < trimmed_end
            && (closing_start == open_end
                || matches!(self.byte(closing_start - 1), Some(b' ' | b'\t')))
        {
            let content_end = self.skip_back(closing_start, open_end, b" \t");
            markers.push(content_end..end);
            content_end
        } else {
            trimmed_end
        };
        markers.extend(attributes.map(|a| a.start..range.end));
        self.push(kind, range, open_end..content_end, markers);
    }

    /// With [`MarkdownOptions::heading_ids`], the trailing attribute block (`{#id .class}`) of the heading text
    /// `text`, from the whitespace before it to its `}`. The rule is pulldown-cmark's: the last non-whitespace
    /// byte is `}` and the nearest `{`, `}`, `<`, `>`, `\` or line break before it is a `{`, even for `{}`.
    fn attributes(&self, text: Range<usize>) -> Option<Range<usize>> {
        if !self.options.heading_ids {
            return None;
        }
        let end = self.skip_back(text.end, text.start, b" \t\r\n");
        if end == text.start || self.byte(end - 1) != Some(b'}') {
            return None;
        }
        let open = self.text.as_bytes()[text.start..end - 1]
            .iter()
            .rposition(|b| matches!(b, b'{' | b'}' | b'<' | b'>' | b'\\' | b'\n' | b'\r'))
            .map(|i| text.start + i)
            .filter(|&i| self.byte(i) == Some(b'{'))?;
        Some(self.skip_back(open, text.start, b" \t")..end)
    }

    fn block_quote(&mut self, range: Range<usize>, kind: Option<BlockQuoteKind>) {
        let range = self.trim(range);
        let parent = self.quotes.last().copied();
        let mut markers = Vec::new();
        let mut line_start = range.start;
        loop {
            let line_end = self.text[line_start..range.end]
                .find('\n')
                .map_or(range.end, |i| line_start + i);
            // On later lines this quote's `>` follows the enclosing quote's marker; a line where the enclosing
            // quote has no marker is a lazy continuation line, which has none either.
            let search_from = if line_start == range.start {
                Some(range.start)
            } else if let Some(parent) = parent {
                let parent_markers = &self.decorations[parent].markers;
                let i = parent_markers.partition_point(|m| m.start.0 < line_start);
                parent_markers
                    .get(i)
                    .filter(|m| m.start.0 < line_end)
                    .map(|m| m.end.0)
            } else {
                Some(line_start)
            };
            if let Some(from) = search_from {
                // Whitespace before `>` is indentation or the continuation indent of an enclosing list item.
                let gt = self.skip(from, line_end, b" \t");
                if gt < line_end && self.byte(gt) == Some(b'>') {
                    let end = if self.byte(gt + 1) == Some(b' ') && gt + 1 < line_end {
                        gt + 2
                    } else {
                        gt + 1
                    };
                    markers.push(gt..end);
                }
            }
            if line_end >= range.end {
                break;
            }
            line_start = line_end + 1;
        }
        let content_start = markers.first().map_or(range.start, |m| m.end);
        let content = content_start..range.end;
        let index = self.push(DecorationKind::BlockQuote, range.clone(), content, markers);
        self.quotes.push(index);
        if let Some(kind) = kind {
            self.callout(kind, range, content_start);
        }
    }

    /// The alert of the quote `range` whose first line's text, from `tag_start`, is the alert's tag
    /// (pulldown-cmark only reports a kind where it is). Pushed after the quote, with the same range.
    fn callout(&mut self, kind: BlockQuoteKind, range: Range<usize>, tag_start: usize) {
        let line_end = self.text[tag_start..range.end]
            .find('\n')
            .map_or(range.end, |i| tag_start + i);
        if !self.text[tag_start..line_end].starts_with("[!") {
            return;
        }
        let kind = DecorationKind::Callout {
            kind: match kind {
                BlockQuoteKind::Note => CalloutKind::Note,
                BlockQuoteKind::Tip => CalloutKind::Tip,
                BlockQuoteKind::Important => CalloutKind::Important,
                BlockQuoteKind::Warning => CalloutKind::Warning,
                BlockQuoteKind::Caution => CalloutKind::Caution,
            },
        };
        let content = (line_end + 1).min(range.end)..range.end;
        self.push(kind, range, content, vec![tag_start..line_end]);
    }

    fn code_block(&mut self, kind: CodeBlockKind<'_>, range: Range<usize>) {
        let range = self.trim(range);
        let CodeBlockKind::Fenced(info) = kind else {
            let kind = DecorationKind::CodeBlock {
                info: String::new(),
            };
            self.push(kind, range.clone(), range, Vec::new());
            return;
        };
        let kind = DecorationKind::CodeBlock {
            info: info.to_string(),
        };
        let fence_start = self.skip(range.start, range.end, b" ");
        let Some(fence) = self.byte(fence_start).filter(|b| matches!(b, b'`' | b'~')) else {
            self.push(kind, range.clone(), range, Vec::new());
            return;
        };
        let fence_len = self.run_len(fence_start, fence);
        let first_line_end = self.text[range.clone()]
            .find('\n')
            .map_or(range.end, |i| range.start + i);
        let mut markers = vec![range.start..first_line_end];
        let content_start = (first_line_end + 1).min(range.end);
        let mut content_end = range.end;
        if first_line_end < range.end {
            let last_line = self.text[..range.end].rfind('\n').map_or(0, |i| i + 1);
            if let Some(closing) = self.closing_fence(last_line..range.end, fence, fence_len) {
                markers.push(closing);
                content_end = (last_line - 1).max(content_start);
            }
        }
        self.push(kind, range, content_start..content_end, markers);
    }

    /// The fence run of `line` if it closes a fence of `len` `fence` bytes. Only whitespace and the `>` of
    /// enclosing quotes may precede it.
    fn closing_fence(&self, line: Range<usize>, fence: u8, len: usize) -> Option<Range<usize>> {
        let trimmed_end = self.skip_back(line.end, line.start, b" \t");
        let run_start = self.skip_back(trimmed_end, line.start, &[fence]);
        let prefix_is_container = self.text.as_bytes()[line.start..run_start]
            .iter()
            .all(|b| matches!(b, b' ' | b'\t' | b'>'));
        (trimmed_end - run_start >= len && prefix_is_container).then_some(run_start..line.end)
    }

    fn item(&mut self, range: Range<usize>) {
        let range = self.trim(range);
        // The first item of a list indented by up to three spaces starts at the indentation.
        let range = self.skip(range.start, range.end, b" \t")..range.end;
        let ordered = !matches!(self.byte(range.start), Some(b'-' | b'*' | b'+'));
        let marker_end = if ordered {
            // Digits, then `.` or `)`.
            (self.skip(range.start, range.end, b"0123456789") + 1).min(range.end)
        } else {
            (range.start + 1).min(range.end)
        };
        // The spaces after the bullet belong to it, except that content indented by five or more is an indented
        // code block whose indentation starts after the first space.
        let spaces_end = self.skip(marker_end, range.end, b" \t");
        let at_line_end = matches!(self.byte(spaces_end), None | Some(b'\n'));
        let marker_end = if spaces_end - marker_end > 4 && !at_line_end {
            marker_end + 1
        } else {
            spaces_end
        };
        let kind = DecorationKind::ListItem { ordered };
        self.push(
            kind,
            range.clone(),
            marker_end..range.end,
            vec![range.start..marker_end],
        );
        self.items.push(range);
    }

    fn task(&mut self, checked: bool, range: Range<usize>) {
        let item_end = self.items.last().map_or(range.end, |item| item.end);
        let marker_end = if self.byte(range.end) == Some(b' ') && range.end < item_end {
            range.end + 1
        } else {
            range.end
        };
        let item_end = item_end.max(marker_end);
        self.push(
            DecorationKind::Task { checked },
            range.start..item_end,
            marker_end..item_end,
            vec![range.start..marker_end],
        );
    }

    fn end_link(&mut self) {
        let Some((Some(index), text_end)) = self.links.pop() else {
            return;
        };
        let range = self.decorations[index].range.start.0..self.decorations[index].range.end.0;
        // Images open with `![`.
        let opener = if self.byte(range.start) == Some(b'!') {
            2
        } else {
            1
        };
        if range.len() <= opener {
            return;
        }
        // Autolinks are `<url>`; inline links are `[text](destination "title")`, images `![alt](source)`.
        let text_end = if self.byte(range.start) == Some(b'<') {
            range.end - 1
        } else {
            text_end.clamp(range.start + opener, range.end)
        };
        let decoration = &mut self.decorations[index];
        decoration.content = offsets(range.start + opener..text_end);
        decoration.markers = vec![
            offsets(range.start..range.start + opener),
            offsets(text_end..range.end),
        ];
    }

    /// Drops the line breaks and blank lines pulldown-cmark includes at the end of some block ranges (which
    /// depends on what follows the block), so that a block ends at the end of its last non-blank line.
    fn trim(&self, range: Range<usize>) -> Range<usize> {
        let last_text = self.skip_back(range.end, range.start, b" \t\n");
        let line_end = self.text[last_text..range.end]
            .find('\n')
            .map_or(range.end, |i| last_text + i);
        range.start..line_end
    }

    fn byte(&self, i: usize) -> Option<u8> {
        self.text.as_bytes().get(i).copied()
    }

    /// How many `byte`s start at `i`.
    fn run_len(&self, i: usize, byte: u8) -> usize {
        self.skip(i, self.text.len(), &[byte]) - i
    }

    /// The first position in `from..to` whose byte is not in `set`, or `to`.
    fn skip(&self, from: usize, to: usize, set: &[u8]) -> usize {
        let bytes = &self.text.as_bytes()[from.min(to)..to];
        from + bytes.iter().take_while(|b| set.contains(b)).count()
    }

    /// Moves back from `from` over bytes in `set`, stopping at `limit`.
    fn skip_back(&self, from: usize, limit: usize, set: &[u8]) -> usize {
        let bytes = &self.text.as_bytes()[limit.min(from)..from];
        from - bytes.iter().rev().take_while(|b| set.contains(b)).count()
    }

    fn table_start(&mut self, range: Range<usize>, start: table::Start) {
        self.table
            .start(self.text, &mut self.decorations, range, start);
    }
}

fn offsets(range: Range<usize>) -> Range<ByteOffset> {
    ByteOffset(range.start)..ByteOffset(range.end)
}
