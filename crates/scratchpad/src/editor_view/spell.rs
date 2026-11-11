//! Spell checking in the open note (ADR 0170): the misspelled words around the screen, kept up
//! to date with the text, the corrections at the top of the context menu (ADR 0175), and
//! autocorrect as the user types.
//!
//! Only prose is checked: front matter, code blocks, inline code, link addresses and other
//! Markdown markers, and web addresses are left out. Plain text files (logs, settings files)
//! are not checked.

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use scratchpad_editor::markdown::{DecorationKind, MarkdownState, front_matter};
use scratchpad_editor::{Bias, Buffer, ByteOffset, TextChange};
use scratchpad_spell::{Action, Checked, Segment};

use super::EditorView;
use crate::spelling;

/// How long the text must stay unchanged before it is checked again, so a word being typed is
/// not checked at every letter.
const CHECK_DEBOUNCE: Duration = Duration::from_millis(300);
/// Lines checked above and below the ones on screen, so short scrolls show underlines at once.
const MARGIN_LINES: usize = 100;
/// At most this much text is checked at a time, around the screen of a note with long lines,
/// even when a single line on screen is longer.
const MAX_CHECK_BYTES: usize = 256 * 1024;
/// Runs of prose longer than this go to the checker in pieces, cut at a space at least
/// [`MIN_SEGMENT_BYTES`] in, so that one call never keeps the checker (and another window's
/// check, or a word for autocorrect) waiting long.
const MAX_SEGMENT_BYTES: usize = 16 * 1024;
const MIN_SEGMENT_BYTES: usize = 4 * 1024;

/// A misspelled or repeated word in the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    pub range: Range<ByteOffset>,
    pub action: Action,
}

pub(super) struct Spell {
    /// The user's choices (View > Spell check, View > Autocorrect), both on by default.
    enabled: bool,
    autocorrect: bool,
    /// Sorted, non-overlapping, at buffer version `version`. Between checks they follow edits,
    /// and words an edit touched are dropped until they are checked again.
    pub(super) found: Vec<Found>,
    version: u64,
    /// The lines checked, at `version`.
    checked: Range<usize>,
    /// The pending check; replacing it cancels the previous one.
    task: Option<Task<()>>,
    /// Waits for the checker's verdict on the word just ended, to correct it.
    autocorrect_task: Option<Task<()>>,
    /// The cursor after the last typed character. The word there is still being typed and is
    /// not underlined while the cursor stays there.
    pub(super) typed_at: Option<ByteOffset>,
}

impl Default for Spell {
    fn default() -> Self {
        Self {
            enabled: true,
            autocorrect: true,
            found: Vec::new(),
            version: 0,
            checked: 0..0,
            task: None,
            autocorrect_task: None,
            typed_at: None,
        }
    }
}

/// The corrections offered for a word: the top of the context menu opened on it.
pub(super) struct SpellingSection {
    found: Found,
    word: String,
    /// The buffer version `found` is for: an edit while the menu is open closes it.
    version: u64,
    /// `None` until the checker has answered.
    suggestions: Option<Vec<String>>,
    _suggest: Option<Task<()>>,
}

impl SpellingSection {
    /// A line shown above the corrections while there are none to offer.
    pub(super) fn note(&self) -> Option<&'static str> {
        match &self.suggestions {
            None => Some("Looking for suggestions\u{2026}"),
            Some(suggestions) if suggestions.is_empty() && self.found.action == Action::Suggest => {
                Some("No suggestions")
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Replace(String),
    DeleteRepeated,
    Add,
    Ignore,
}

impl Choice {
    /// A change to the text, drawn in semibold above what to do with the word.
    pub(super) fn is_correction(&self) -> bool {
        matches!(self, Choice::Replace(_) | Choice::DeleteRepeated)
    }
}

impl EditorView {
    /// Whether misspelled words are underlined.
    pub fn spell_check(&self) -> bool {
        self.spell.enabled
    }

    pub fn set_spell_check(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.spell.enabled {
            return;
        }
        self.spell.enabled = enabled;
        self.spell.found.clear();
        self.spell.checked = 0..0;
        self.spell.task = None;
        self.close_context_menu(cx);
        if enabled {
            self.schedule_spell_check(false, cx);
        }
        cx.notify();
    }

    /// Whether typos on the system's autocorrect list are corrected after a space or
    /// punctuation is typed.
    pub fn autocorrect(&self) -> bool {
        self.spell.autocorrect
    }

    pub fn set_autocorrect(&mut self, autocorrect: bool, cx: &mut Context<Self>) {
        if autocorrect != self.spell.autocorrect {
            self.spell.autocorrect = autocorrect;
            cx.notify();
        }
    }

    /// The words underlined as misspelled, for tests.
    #[doc(hidden)]
    pub fn misspelled_words(&self) -> Vec<String> {
        let buffer = self.editor.buffer();
        self.visible_misspellings()
            .map(|found| buffer.text_for_range(found.range.clone()).into_owned())
            .collect()
    }

    /// The spelling items at the top of the context menu while it is open on a misspelled
    /// word, for tests.
    #[doc(hidden)]
    pub fn spelling_menu_items(&self) -> Option<Vec<String>> {
        let section = self.context_menu.as_ref()?.spelling.as_ref()?;
        Some(
            menu_choices(section, self.read_only)
                .into_iter()
                .map(|(_, label)| label)
                .collect(),
        )
    }

    /// Spell checking applies: on, in Markdown, with a checker (or one starting).
    fn spell_active(&self, cx: &Context<Self>) -> bool {
        self.spell.enabled && !self.plain && spelling::is_available(cx)
    }

    /// Called after every change to the text: moves the misspellings along with it and checks
    /// again once typing pauses. A `new_document` has nothing in common with the old ones.
    pub(super) fn spell_text_changed(&mut self, new_document: bool, cx: &mut Context<Self>) {
        // The word its corrections are for, the selection it cuts and the history it undoes may
        // have changed.
        self.close_context_menu(cx);
        let spell = &mut self.spell;
        if new_document {
            spell.typed_at = None;
        }
        let buffer = self.editor.buffer();
        match buffer
            .changes_since(spell.version)
            .filter(|_| !new_document)
        {
            Some(changes) => {
                for change in changes {
                    adjust(&mut spell.found, change);
                }
            }
            None => {
                spell.found.clear();
                spell.checked = 0..0;
            }
        }
        spell.version = buffer.version();
        self.schedule_spell_check(!new_document, cx);
    }

    /// Called with each frame: checks the lines on screen if they have not been, e.g. after
    /// scrolling, opening a note or turning Markdown on.
    pub(super) fn spell_check_visible(&mut self, cx: &mut Context<Self>) {
        let visible = &self.visible_lines;
        let spell = &self.spell;
        let covered = spell.checked.start <= visible.start && visible.end <= spell.checked.end;
        if spell.task.is_none() && !covered && self.spell_active(cx) {
            self.schedule_spell_check(false, cx);
        }
    }

    fn schedule_spell_check(&mut self, debounce: bool, cx: &mut Context<Self>) {
        if !self.spell_active(cx) {
            self.spell.task = None;
            if !self.spell.found.is_empty() {
                self.spell.found.clear();
                cx.notify();
            }
            return;
        }
        let requester = cx.entity_id().as_u64();
        self.spell.task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(CHECK_DEBOUNCE).await;
            }
            let request = this.update(cx, |this, cx| {
                let speller = spelling::speller(cx)?;
                let buffer = this.editor.buffer();
                let lines = lines_to_check(buffer, this.visible_lines.clone());
                let segments = prose_segments(buffer, &mut this.markdown, lines.clone());
                Some((speller.check(requester, segments), buffer.version(), lines))
            });
            let Ok(Some((reply, version, lines))) = request else {
                return;
            };
            let checked = reply.await;
            this.update(cx, |this, cx| {
                this.spell.task = None;
                let found = match checked {
                    Checked::Done(found) => found,
                    // A newer check of this view's took its place, in a task of its own.
                    Checked::Superseded => return,
                    // The checker stopped: there is nothing to check with any more.
                    Checked::Unavailable => {
                        this.spell.found.clear();
                        this.spell.checked = 0..0;
                        cx.notify();
                        return;
                    }
                };
                // An edit since then has scheduled another check.
                if this.editor.buffer().version() != version || !this.spell_active(cx) {
                    return;
                }
                this.spell.found = found
                    .into_iter()
                    .map(|m| Found {
                        range: ByteOffset(m.range.start)..ByteOffset(m.range.end),
                        action: m.action,
                    })
                    .collect();
                this.spell.version = version;
                this.spell.checked = lines;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The misspellings to underline: all but the word being typed at the cursor.
    pub(super) fn visible_misspellings(&self) -> impl Iterator<Item = &Found> {
        let head = self.editor.selection().head;
        let typing = self
            .spell
            .typed_at
            .filter(|&typed| typed == head && self.editor.selection().is_empty());
        self.spell.found.iter().filter(move |found| {
            typing.is_none_or(|at| at < found.range.start || at > found.range.end)
        })
    }

    // --- Autocorrect ---

    /// After a keystroke inserted `text`: remembers where typing is, and if it ended a word with
    /// a space or punctuation, asks the checker about that word. Typing goes on meanwhile; the
    /// word is corrected when the answer comes, if the checker is sure of the correction and
    /// nothing changed since.
    pub(super) fn after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let head = self.editor.selection().head;
        self.spell.typed_at = Some(head);
        self.spell.autocorrect_task = None;
        let mut chars = text.chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else {
            return;
        };
        let ends_word = typed.is_whitespace() || ".,;:!?".contains(typed);
        if !ends_word || !self.spell.autocorrect || !self.spell_active(cx) {
            return;
        }
        // Not while the checker is starting: the answer would come too late to be of use.
        let Some(speller) = spelling::speller(cx).filter(|s| s.available() == Some(true)) else {
            return;
        };
        let Some((range, word)) = self.word_before(head, typed) else {
            return;
        };
        let version = self.editor.buffer().version();
        let verdict = speller.check_word(&word);
        self.spell.autocorrect_task = Some(cx.spawn(async move |this, cx| {
            let Some(Action::Replace(correction)) = verdict.await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.autocorrect_word(range, &word, &correction, version, cx)
            })
            .ok();
        }));
    }

    /// Replaces `word` at `range` with `correction` if the text is still as it was at
    /// `version`, as its own undo step, so Ctrl+Z brings back what was typed.
    fn autocorrect_word(
        &mut self,
        range: Range<ByteOffset>,
        word: &str,
        correction: &str,
        version: u64,
        cx: &mut Context<Self>,
    ) {
        self.spell.autocorrect_task = None;
        let buffer = self.editor.buffer();
        let unchanged = buffer.version() == version && buffer.text_for_range(range.clone()) == word;
        if !unchanged || !self.spell.autocorrect || correction == word || correction.is_empty() {
            return;
        }
        // The cursor (and where typing is) stay after the word, or before it.
        let moved = |offset: ByteOffset| {
            if offset >= range.end {
                ByteOffset(offset.0 - range.end.0 + range.start.0 + correction.len())
            } else {
                offset.min(range.start)
            }
        };
        let selection = self.editor.selection();
        self.edit(cx, |editor| {
            editor.break_undo_group();
            editor.transact(|editor| {
                editor.replace_range(range.clone(), correction);
                editor.set_selection(scratchpad_editor::Selection::new(
                    moved(selection.anchor),
                    moved(selection.head),
                ));
            });
        });
        self.spell.typed_at = self.spell.typed_at.map(moved);
    }

    /// The prose word that ends right before `typed`, which was just typed before `head`.
    fn word_before(
        &mut self,
        head: ByteOffset,
        typed: char,
    ) -> Option<(Range<ByteOffset>, String)> {
        let buffer = self.editor.buffer();
        let boundary = head.0.checked_sub(typed.len_utf8())?;
        if boundary == 0 || buffer.text_for_range(ByteOffset(boundary)..head) != typed.to_string() {
            return None;
        }
        let inside = buffer.clip_offset(ByteOffset(boundary - 1), Bias::Left);
        let range = self.editor.word_range_at(inside);
        if range.end.0 != boundary {
            return None;
        }
        let word = buffer.text_for_range(range.clone()).into_owned();
        if !word.chars().any(char::is_alphabetic) {
            return None;
        }
        let line = buffer.line_of(range.start);
        let in_prose = prose_segments(buffer, &mut self.markdown, line..line + 1)
            .iter()
            .any(|segment| {
                segment.offset <= range.start.0
                    && range.end.0 <= segment.offset + segment.text.len()
            });
        in_prose.then_some((range, word))
    }

    // --- The context menu's corrections ---

    /// The corrections for the misspelled word at `offset`, if there is one there, for the top of
    /// the context menu. Suggestions are asked for meanwhile and fill in when they come.
    pub(super) fn spelling_section_at(
        &mut self,
        offset: ByteOffset,
        cx: &mut Context<Self>,
    ) -> Option<SpellingSection> {
        let found = self
            .visible_misspellings()
            .find(|found| found.range.start <= offset && offset <= found.range.end)
            .cloned()?;
        let speller = spelling::speller(cx)?;
        let word = self
            .editor
            .buffer()
            .text_for_range(found.range.clone())
            .into_owned();
        let (suggestions, task) = match &found.action {
            Action::Suggest => {
                let reply = speller.suggest(&word);
                let task = cx.spawn(async move |this, cx| {
                    let suggestions = reply.await;
                    this.update(cx, |this, cx| {
                        let menu = this.context_menu.as_mut();
                        if let Some(section) = menu.and_then(|menu| menu.spelling.as_mut()) {
                            section.suggestions = Some(suggestions);
                            cx.notify();
                        }
                    })
                    .ok();
                });
                (None, Some(task))
            }
            Action::Replace(correction) => (Some(vec![correction.clone()]), None),
            Action::Delete => (Some(Vec::new()), None),
        };
        Some(SpellingSection {
            found,
            word,
            version: self.editor.buffer().version(),
            suggestions,
            _suggest: task,
        })
    }

    /// Applies `choice` from the context menu, which has closed, to the word of `section`.
    pub(super) fn choose_spelling(
        &mut self,
        section: SpellingSection,
        choice: Choice,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.editor.buffer();
        let current = section.version == buffer.version();
        let range = section.found.range.clone();
        match choice {
            Choice::Replace(replacement) if current => {
                self.edit(cx, |editor| editor.replace_range(range, &replacement));
            }
            Choice::DeleteRepeated if current => {
                // With the space before it, which separated it from the first.
                let line_start = buffer.line_start(buffer.line_of(range.start));
                let before = buffer.text_for_range(line_start..range.start);
                let spaces = before.len() - before.trim_end_matches([' ', '\t']).len();
                let start = ByteOffset(range.start.0 - spaces);
                self.edit(cx, |editor| editor.replace_range(start..range.end, ""));
            }
            Choice::Add | Choice::Ignore => {
                if let Some(speller) = spelling::speller(cx) {
                    if choice == Choice::Add {
                        speller.add(&section.word);
                    } else {
                        speller.ignore(&section.word);
                    }
                }
                // Every other place the word is underlined goes at once; the check confirms it.
                let buffer = self.editor.buffer();
                self.spell
                    .found
                    .retain(|found| buffer.text_for_range(found.range.clone()) != section.word);
                self.spell.checked = 0..0;
                self.schedule_spell_check(false, cx);
            }
            Choice::Replace(_) | Choice::DeleteRepeated => {}
        }
    }
}

/// The section's items and their labels, in order: the corrections, then what to do with the
/// word. A read-only note offers no corrections.
pub(super) fn menu_choices(menu: &SpellingSection, read_only: bool) -> Vec<(Choice, String)> {
    let mut choices = Vec::new();
    if !read_only {
        match &menu.found.action {
            Action::Delete => {
                choices.push((Choice::DeleteRepeated, "Delete repeated word".to_owned()))
            }
            _ => choices.extend(
                menu.suggestions
                    .iter()
                    .flatten()
                    .map(|suggestion| (Choice::Replace(suggestion.clone()), suggestion.clone())),
            ),
        }
    }
    if menu.found.action != Action::Delete {
        choices.push((Choice::Add, "Add to dictionary".to_owned()));
    }
    choices.push((Choice::Ignore, "Ignore".to_owned()));
    choices
}

/// The lines to check for a screen showing `visible`: those and [`MARGIN_LINES`] on each side,
/// fewer if they hold more than [`MAX_CHECK_BYTES`].
fn lines_to_check(buffer: &Buffer, visible: Range<usize>) -> Range<usize> {
    let count = buffer.line_count();
    let visible = visible.start.min(count)..visible.end.min(count);
    let mut lines =
        visible.start.saturating_sub(MARGIN_LINES)..(visible.end + MARGIN_LINES).min(count);
    let bytes = |lines: &Range<usize>| {
        buffer.line_end(lines.end.max(lines.start + 1) - 1).0 - buffer.line_start(lines.start).0
    };
    while lines.len() > visible.len().max(1) && bytes(&lines) > MAX_CHECK_BYTES {
        if lines.start < visible.start {
            lines.start = (lines.start + visible.start).div_ceil(2);
        }
        if lines.end > visible.end {
            lines.end = (lines.end + visible.end) / 2;
        }
    }
    lines
}

/// The prose of `lines`: their text without front matter, code, Markdown markers, link
/// addresses and web addresses, as one segment per run (or per piece of a long run). At most
/// [`MAX_CHECK_BYTES`] of text is read, from the start of `lines`.
fn prose_segments(
    buffer: &Buffer,
    markdown: &mut MarkdownState,
    lines: Range<usize>,
) -> Vec<Segment> {
    // Only closed front matter: an open one may be a note being written under a `---`.
    let front_matter = front_matter(buffer, usize::MAX).filter(|front_matter| front_matter.closed);
    let lines = front_matter.map_or(lines.clone(), |f| lines.start.max(f.end)..lines.end);
    if lines.is_empty() {
        return Vec::new();
    }
    let start = buffer.line_start(lines.start);
    let end = buffer
        .line_end(lines.end - 1)
        .min(buffer.clip_offset(ByteOffset(start.0 + MAX_CHECK_BYTES), Bias::Left));
    let mut excluded: Vec<Range<usize>> = Vec::new();
    for decoration in markdown.decorations(buffer, start..end) {
        let range = decoration.range.start.0..decoration.range.end.0;
        match &decoration.kind {
            DecorationKind::CodeBlock { .. } | DecorationKind::InlineCode => excluded.push(range),
            DecorationKind::Link { destination } => {
                // An autolink, `<https://...>`, shows its address as its text.
                let content = decoration.content.clone();
                if buffer.text_for_range(content.clone()) == destination.as_str() {
                    excluded.push(range);
                } else {
                    excluded.extend(decoration.markers.iter().map(|m| m.start.0..m.end.0));
                }
            }
            _ => excluded.extend(decoration.markers.iter().map(|m| m.start.0..m.end.0)),
        }
    }
    excluded.sort_by_key(|range| range.start);
    let mut segments = Vec::new();
    for line in lines {
        let line_start = buffer.line_start(line).0;
        let line_end = buffer.line_end(line).0;
        if line_start > end.0 {
            break;
        }
        let text = buffer.text_for_range(ByteOffset(line_start)..ByteOffset(line_end.min(end.0)));
        // A line cut short ends at its last space, not inside a word.
        let text = if line_end > end.0 {
            &text[..text.rfind(char::is_whitespace).unwrap_or(0)]
        } else {
            &text[..]
        };
        let mut cuts: Vec<Range<usize>> = excluded
            .iter()
            .filter(|range| range.start < line_start + text.len() && range.end > line_start)
            .map(|range| {
                range.start.max(line_start) - line_start
                    ..range.end.min(line_start + text.len()) - line_start
            })
            .collect();
        cuts.extend(web_addresses(text));
        cuts.sort_by_key(|range| range.start);
        let mut from = 0;
        for cut in cuts
            .into_iter()
            .chain(std::iter::once(text.len()..text.len()))
        {
            if cut.start > from {
                for piece in pieces(&text[from..cut.start]) {
                    let piece_text = &text[from + piece.start..from + piece.end];
                    if piece_text.chars().any(char::is_alphabetic) {
                        segments.push(Segment {
                            offset: line_start + from + piece.start,
                            text: piece_text.to_owned(),
                        });
                    }
                }
            }
            from = from.max(cut.end);
        }
    }
    segments
}

/// `text` in pieces of at most [`MAX_SEGMENT_BYTES`], each ending before a space where there is
/// one far enough in: byte ranges, together all of `text`.
fn pieces(text: &str) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut from = 0;
    while text.len() - from > MAX_SEGMENT_BYTES {
        let mut to = from + MAX_SEGMENT_BYTES;
        while !text.is_char_boundary(to) {
            to -= 1;
        }
        if let Some(space) = text[from..to]
            .rfind(char::is_whitespace)
            .filter(|&space| space >= MIN_SEGMENT_BYTES)
        {
            to = from + space;
        }
        pieces.push(from..to);
        from = to;
    }
    pieces.push(from..text.len());
    pieces
}

/// Bare web addresses in a line (`https://...`, `www....`): whitespace-separated runs that look
/// like one.
fn web_addresses(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(index),
            (true, Some(from)) => {
                let word = &text[from..index];
                // From the scheme or `www.` on: what comes before (`docs](`) is not the address.
                let address = word
                    .find("://")
                    .map(|scheme_end| {
                        word[..scheme_end]
                            .rfind(|c: char| !c.is_ascii_alphanumeric() && !"+-.".contains(c))
                            .map_or(0, |before| before + 1)
                    })
                    .or_else(|| {
                        word.match_indices("www.")
                            .map(|(at, _)| at)
                            .find(|&at| !word[..at].ends_with(|c: char| c.is_alphanumeric()))
                    });
                if let Some(address) = address {
                    found.push(from + address..index);
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// Moves misspellings along with `change`: those after it shift, those it touched are dropped
/// until the text is checked again (see `search::adjust_matches`).
fn adjust(found: &mut Vec<Found>, change: &TextChange) {
    let before = found.partition_point(|f| f.range.end < change.start);
    let touched = found[before..].partition_point(|f| f.range.start <= change.old_end);
    found.drain(before..before + touched);
    let removed = change.old_end.0 - change.start.0;
    let inserted = change.new_end.0 - change.start.0;
    for f in &mut found[before..] {
        f.range.start = ByteOffset(f.range.start.0 - removed + inserted);
        f.range.end = ByteOffset(f.range.end.0 - removed + inserted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments(text: &str) -> Vec<(usize, String)> {
        let buffer = Buffer::from_text(text);
        let mut markdown = MarkdownState::new(&buffer);
        prose_segments(&buffer, &mut markdown, 0..buffer.line_count())
            .into_iter()
            .map(|segment| (segment.offset, segment.text))
            .collect()
    }

    #[test]
    fn code_markers_and_addresses_are_left_out() {
        let text = "# Title\n\nSee `cofe` and [the docs](https://exmaple.com) or https://x.io/a.\n\n```\nfn mian() {}\n```\n<https://auto.link>";
        let found: Vec<String> = segments(text).into_iter().map(|(_, text)| text).collect();
        assert_eq!(found, ["Title", "See ", " and ", "the docs", " or "]);
    }

    #[test]
    fn web_addresses_start_at_their_scheme() {
        let text = "see (https://a.b/c) docs](http://x.y) www.z.org awww.q";
        let found: Vec<&str> = web_addresses(text)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(found, ["https://a.b/c)", "http://x.y)", "www.z.org"]);
    }

    #[test]
    fn segments_have_document_offsets() {
        assert_eq!(
            segments("one\n**two** three"),
            [
                (0, "one".to_owned()),
                (6, "two".to_owned()),
                (11, " three".to_owned())
            ]
        );
    }

    #[test]
    fn edits_drop_the_words_they_touch_and_move_the_rest() {
        let found = |start: usize, end: usize| Found {
            range: ByteOffset(start)..ByteOffset(end),
            action: Action::Suggest,
        };
        let mut editor = scratchpad_editor::Editor::from_text("aa bb cc dd");
        let mut list = vec![found(0, 2), found(3, 5), found(9, 11)];
        // Typing at the end of "bb" changes that word.
        editor.replace_range(ByteOffset(5)..ByteOffset(5), "x");
        let change = editor.buffer().changes_since(0).unwrap().next().unwrap();
        adjust(&mut list, change);
        assert_eq!(list, [found(0, 2), found(10, 12)]);
    }

    #[test]
    fn long_runs_are_cut_at_spaces() {
        let text = "word ".repeat(10_000);
        let found = pieces(&text);
        assert!(found.len() > 1);
        assert_eq!(found.first().unwrap().start, 0);
        assert_eq!(found.last().unwrap().end, text.len());
        for (piece, next) in found.iter().zip(&found[1..]) {
            assert_eq!(piece.end, next.start);
            assert!(piece.len() <= MAX_SEGMENT_BYTES, "{piece:?}");
            assert_eq!(&text[next.start..next.start + 1], " ", "{next:?}");
        }
        // Without spaces, at a character boundary.
        let text = "é".repeat(10_000);
        assert!(
            pieces(&text)
                .iter()
                .all(|piece| text.is_char_boundary(piece.end))
        );
        assert_eq!(pieces("short"), [Range { start: 0, end: 5 }]);
    }

    #[test]
    fn long_lines_limit_what_is_checked() {
        let line = "word ".repeat(20_000);
        let buffer = Buffer::from_text(&vec![line.as_str(); 50].join("\n"));
        let lines = lines_to_check(&buffer, 10..12);
        assert!(lines.start <= 10 && 12 <= lines.end, "{lines:?}");
        assert!(lines.len() < 10, "{lines:?}");
        assert_eq!(lines_to_check(&Buffer::from_text("a\nb"), 0..2), 0..2);
    }
}
