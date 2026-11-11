//! The walk over a region's parse that the text scanners share: bare addresses (ADR 0187), highlights and emoji
//! shortcodes (ADR 0190). [`TextRuns`] turns pulldown-cmark's events into [`Step`]s: runs of consecutive text
//! (merged, since entities, escapes and unmatched delimiters split text into several events) with what they are
//! in, and the inline elements and blocks around them. Every scanner sees the same runs at the same moments, so
//! none can get out of step with another.
//!
//! Text in code blocks is no run. Reference links (`[text][ref]`, `[ref]`) stay text (ADR 0040) and resolve
//! against definitions that may be in another region, so they are seen through: their brackets are text and
//! they are no element. What is found around them is then the same whether or not a definition exists.
//!
//! Nothing is allocated per event; the stack of open links grows only with links nested in links.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Tag, TagEnd};

/// What the walk reports, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    /// A run of text, ended by the event after it.
    Run {
        range: Range<usize>,
        context: RunContext,
    },
    /// An inline element starts (emphasis, strong, strikethrough, super- or subscript, a link or an image).
    Enter,
    /// The inline element entered last ends.
    Leave,
    /// A block starts or ends: nothing inline continues across it.
    Block,
}

/// Where a text run is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RunContext {
    /// In the text of a link or an image (not a reference link).
    pub in_link: bool,
    /// In an autolink, an e-mail autolink or a wikilink, whose text is an address or a page name.
    pub in_address_or_page: bool,
    /// The run starts a text node of its own in GFM, as at the start of a line: a table cell's text, and (an
    /// Obsidian addition) text right after a wikilink.
    pub node_start: bool,
}

/// Whether links of `link_type` resolve against reference definitions (see the module documentation).
fn is_reference(link_type: LinkType) -> bool {
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
    /// An inline link or an image.
    Plain,
    /// An autolink, e-mail autolink or wikilink.
    AddressOrPage { wikilink: bool },
    /// A reference link, with where the events inside it end so far.
    Reference(usize),
}

/// The walk's state; see the module documentation.
#[derive(Debug, Default)]
pub(super) struct TextRuns {
    /// The run of text seen last, not yet reported.
    run: Option<Range<usize>>,
    /// Where text starts that is a text node of its own ([`RunContext::node_start`]).
    node_start: Option<usize>,
    /// How many code blocks the next event is in.
    code_blocks: usize,
    /// The links and images the next event is in, innermost last.
    links: Vec<OpenLink>,
    /// How many of them are plain links or images, and autolinks or wikilinks.
    plain: usize,
    address_or_page: usize,
}

impl TextRuns {
    /// Takes the next event of the parse of `text`, with its range, and reports what it ends and starts to
    /// `step`. Text from `text_end` on is not prose (a heading's attribute block) and is no run.
    pub(super) fn event(
        &mut self,
        event: &Event<'_>,
        range: &Range<usize>,
        text_end: Option<usize>,
        mut step: impl FnMut(Step),
    ) {
        let range = range.start..range.end.min(text_end.unwrap_or(usize::MAX));
        let link_end = matches!(event, Event::End(TagEnd::Link | TagEnd::Image));
        if let Some(OpenLink::Reference(end)) = self.links.last_mut()
            && !link_end
        {
            *end = (*end).max(range.end);
        }
        match event {
            Event::Text(_) if self.code_blocks == 0 => return self.text(range, &mut step),
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
                return self.text(opener, &mut step);
            }
            Event::End(TagEnd::Link | TagEnd::Image)
                if matches!(self.links.last(), Some(OpenLink::Reference(_))) =>
            {
                // Its `]`, `][ref]` or `][]` is text too.
                if let Some(OpenLink::Reference(inner_end)) = self.links.pop() {
                    self.text(inner_end.min(range.end)..range.end, &mut step);
                }
                return;
            }
            _ => {}
        }
        self.flush(&mut step);
        match event {
            Event::Start(Tag::Link { link_type, .. }) => {
                let open = match link_type {
                    LinkType::Autolink | LinkType::Email => {
                        OpenLink::AddressOrPage { wikilink: false }
                    }
                    LinkType::WikiLink { .. } => OpenLink::AddressOrPage { wikilink: true },
                    _ => OpenLink::Plain,
                };
                self.open(open);
                step(Step::Enter);
            }
            Event::Start(Tag::Image { .. }) => {
                self.open(OpenLink::Plain);
                step(Step::Enter);
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                match self.links.pop() {
                    Some(OpenLink::Plain) => self.plain = self.plain.saturating_sub(1),
                    Some(OpenLink::AddressOrPage { wikilink }) => {
                        self.address_or_page = self.address_or_page.saturating_sub(1);
                        if wikilink {
                            self.node_start = Some(range.end);
                        }
                    }
                    _ => {}
                }
                step(Step::Leave);
            }
            Event::Start(
                Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript,
            ) => step(Step::Enter),
            Event::End(
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript,
            ) => step(Step::Leave),
            Event::Start(tag) => {
                match tag {
                    Tag::CodeBlock(_) => self.code_blocks += 1,
                    Tag::TableCell => self.node_start = Some(range.start),
                    _ => {}
                }
                self.close_links();
                step(Step::Block);
            }
            Event::End(tag) => {
                if let TagEnd::CodeBlock = tag {
                    self.code_blocks = self.code_blocks.saturating_sub(1);
                }
                self.close_links();
                step(Step::Block);
            }
            // Code spans, math, inline HTML, line breaks, task boxes: no text, no element.
            _ => {}
        }
    }

    /// Reports the last run.
    pub(super) fn finish(&mut self, mut step: impl FnMut(Step)) {
        self.flush(&mut step);
    }

    fn open(&mut self, link: OpenLink) {
        match link {
            OpenLink::Plain => self.plain += 1,
            OpenLink::AddressOrPage { .. } => self.address_or_page += 1,
            OpenLink::Reference(_) => {}
        }
        self.links.push(link);
    }

    /// Links never span blocks; forgets any left open by malformed events.
    fn close_links(&mut self) {
        self.links.clear();
        self.plain = 0;
        self.address_or_page = 0;
    }

    /// Adds `piece` to the run it follows, or reports that run and starts a new one.
    fn text(&mut self, piece: Range<usize>, step: &mut impl FnMut(Step)) {
        if piece.is_empty() {
            return;
        }
        match &mut self.run {
            Some(run) if run.end == piece.start => run.end = piece.end,
            _ => {
                self.flush(step);
                self.run = Some(piece);
            }
        }
    }

    fn flush(&mut self, step: &mut impl FnMut(Step)) {
        if let Some(range) = self.run.take() {
            let context = RunContext {
                in_link: self.plain + self.address_or_page > 0,
                in_address_or_page: self.address_or_page > 0,
                node_start: self.node_start == Some(range.start),
            };
            step(Step::Run { range, context });
        }
    }
}
