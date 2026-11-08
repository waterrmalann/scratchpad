//! Spell checking in the open note (ADR 0170): the misspelled words around the screen, kept up
//! to date with the text, the menu of corrections on right-click, and autocorrect as the user
//! types.
//!
//! Only prose is checked: code blocks, inline code, link addresses and other Markdown markers,
//! and web addresses are left out. Plain text files (logs, settings files) are not checked.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    ClickEvent, Context, FocusHandle, KeyDownEvent, MouseDownEvent, Pixels, Point, Subscription,
    Task, Window, anchored, deferred, div, prelude::*, px,
};
use scratchpad_editor::markdown::{DecorationKind, MarkdownState};
use scratchpad_editor::{Bias, Buffer, ByteOffset, TextChange};
use scratchpad_spell::{Action, Segment};

use super::EditorView;
use crate::spelling;
use crate::theme::ActiveTheme;

/// How long the text must stay unchanged before it is checked again, so a word being typed is
/// not checked at every letter.
const CHECK_DEBOUNCE: Duration = Duration::from_millis(300);
/// Lines checked above and below the ones on screen, so short scrolls show underlines at once.
const MARGIN_LINES: usize = 100;
/// At most this much text is checked at a time, around the screen of a note with long lines.
const MAX_CHECK_BYTES: usize = 256 * 1024;
/// How long a typed space waits for the checker's verdict on the word before it. The checker
/// answers single words in well under a millisecond; a late answer only skips the correction.
const AUTOCORRECT_WAIT: Duration = Duration::from_millis(40);

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
    /// The cursor after the last typed character. The word there is still being typed and is
    /// not underlined while the cursor stays there.
    pub(super) typed_at: Option<ByteOffset>,
    menu: Option<SpellMenu>,
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
            typed_at: None,
            menu: None,
        }
    }
}

/// The corrections offered for a word, open after a right-click on it.
pub(super) struct SpellMenu {
    found: Found,
    word: String,
    /// The buffer version `found` is for: an edit while the menu is open closes it.
    version: u64,
    /// `None` until the checker has answered.
    suggestions: Option<Vec<String>>,
    position: Point<Pixels>,
    focus: FocusHandle,
    _blur: Subscription,
    _suggest: Option<Task<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Choice {
    Replace(String),
    DeleteRepeated,
    Add,
    Ignore,
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
        self.spell.menu = None;
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
        self.spell
            .found
            .iter()
            .map(|found| buffer.text_for_range(found.range.clone()).into_owned())
            .collect()
    }

    /// The items of the spelling menu while it is open, for tests.
    #[doc(hidden)]
    pub fn spelling_menu_items(&self) -> Option<Vec<String>> {
        let menu = self.spell.menu.as_ref()?;
        Some(
            menu_choices(menu, self.read_only)
                .into_iter()
                .map(|(_, label)| label)
                .collect(),
        )
    }

    /// Picks the spelling menu item labelled `label`, for tests.
    #[doc(hidden)]
    pub fn choose_spelling_menu_item(
        &mut self,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = self.spell.menu.as_ref().and_then(|menu| {
            menu_choices(menu, self.read_only)
                .into_iter()
                .find(|(_, item)| item == label)
                .map(|(choice, _)| choice)
        });
        if let Some(choice) = choice {
            self.choose_spelling(choice, window, cx);
        }
    }

    /// Spell checking applies: on, in Markdown, with a checker.
    fn spell_active(&self, cx: &Context<Self>) -> bool {
        self.spell.enabled && !self.plain && spelling::speller(cx).is_some()
    }

    /// Called after every change to the text: moves the misspellings along with it and checks
    /// again once typing pauses. A `new_document` has nothing in common with the old ones.
    pub(super) fn spell_text_changed(&mut self, new_document: bool, cx: &mut Context<Self>) {
        let spell = &mut self.spell;
        if new_document {
            spell.typed_at = None;
        }
        // Its word may have changed or moved.
        spell.menu = None;
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
        self.spell.task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor().timer(CHECK_DEBOUNCE).await;
            }
            let request = this.update(cx, |this, cx| {
                let speller = spelling::speller(cx)?;
                let buffer = this.editor.buffer();
                let lines = lines_to_check(buffer, this.visible_lines.clone());
                let segments = prose_segments(buffer, &mut this.markdown, lines.clone());
                Some((speller.check(segments), buffer.version(), lines))
            });
            let Ok(Some((reply, version, lines))) = request else {
                return;
            };
            // No reply: a newer check of another window's note took its place (this view's
            // own newer checks drop this task). Try again once that one is done.
            let Ok(found) = reply.recv_async().await else {
                this.update(cx, |this, cx| this.schedule_spell_check(true, cx))
                    .ok();
                return;
            };
            this.update(cx, |this, cx| {
                this.spell.task = None;
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
    /// a space or punctuation, corrects that word when the checker is sure of the correction.
    /// The correction is its own undo step, so Ctrl+Z brings back what was typed.
    pub(super) fn after_typing(&mut self, text: &str, cx: &mut Context<Self>) {
        let head = self.editor.selection().head;
        self.spell.typed_at = Some(head);
        let mut chars = text.chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else {
            return;
        };
        let ends_word = typed.is_whitespace() || ".,;:!?".contains(typed);
        if !ends_word || !self.spell.autocorrect || !self.spell_active(cx) {
            return;
        }
        let Some(speller) = spelling::speller(cx) else {
            return;
        };
        let Some((range, word)) = self.word_before(head, typed) else {
            return;
        };
        let Some(Action::Replace(correction)) = speller.check_word(&word, AUTOCORRECT_WAIT) else {
            return;
        };
        if correction == word || correction.is_empty() {
            return;
        }
        let shift = correction.len() as isize - word.len() as isize;
        let selection = self.editor.selection();
        self.edit(cx, |editor| {
            editor.break_undo_group();
            editor.transact(|editor| {
                editor.replace_range(range, &correction);
                let shifted =
                    |offset: ByteOffset| ByteOffset(offset.0.saturating_add_signed(shift));
                editor.set_selection(scratchpad_editor::Selection::new(
                    shifted(selection.anchor),
                    shifted(selection.head),
                ));
            });
        });
        self.spell.typed_at = Some(self.editor.selection().head);
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

    // --- The menu ---

    /// A right-click on a misspelled word opens its menu of corrections; elsewhere it does
    /// nothing.
    pub(super) fn spelling_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let offset = self.offset_at(event.position, window);
        let Some(found) = self
            .visible_misspellings()
            .find(|found| found.range.start <= offset && offset <= found.range.end)
            .cloned()
        else {
            return false;
        };
        let Some(speller) = spelling::speller(cx) else {
            return false;
        };
        let word = self
            .editor
            .buffer()
            .text_for_range(found.range.clone())
            .into_owned();
        let (suggestions, task) = match &found.action {
            Action::Suggest => {
                let reply = speller.suggest(&word);
                let task = cx.spawn(async move |this, cx| {
                    let suggestions = reply.recv_async().await.unwrap_or_default();
                    this.update(cx, |this, cx| {
                        if let Some(menu) = &mut this.spell.menu {
                            menu.suggestions = Some(suggestions);
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
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.spell.menu = Some(SpellMenu {
            found,
            word,
            version: self.editor.buffer().version(),
            suggestions,
            position: event.position,
            _blur: cx.on_blur(&focus, window, |this, window, cx| {
                this.close_spelling_menu(window, cx)
            }),
            focus,
            _suggest: task,
        });
        cx.notify();
        true
    }

    fn close_spelling_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.spell.menu.take() {
            if menu.focus.is_focused(window) {
                window.focus(&self.focus_handle);
            }
            cx.notify();
        }
    }

    fn choose_spelling(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.spell.menu.take() else {
            return;
        };
        window.focus(&self.focus_handle);
        cx.notify();
        let buffer = self.editor.buffer();
        let current = menu.version == buffer.version();
        let range = menu.found.range.clone();
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
                        speller.add(&menu.word);
                    } else {
                        speller.ignore(&menu.word);
                    }
                }
                // Every other place the word is underlined goes at once; the check confirms it.
                let buffer = self.editor.buffer();
                self.spell
                    .found
                    .retain(|found| buffer.text_for_range(found.range.clone()) != menu.word);
                self.spell.checked = 0..0;
                self.schedule_spell_check(false, cx);
            }
            Choice::Replace(_) | Choice::DeleteRepeated => {}
        }
    }

    pub(super) fn render_spelling_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.spell.menu.as_ref()?;
        let theme = cx.theme().clone();
        let choices = menu_choices(menu, self.read_only);
        let note = match &menu.suggestions {
            None => Some("Looking for suggestions\u{2026}"),
            Some(suggestions) if suggestions.is_empty() && menu.found.action == Action::Suggest => {
                Some("No suggestions")
            }
            _ => None,
        };
        let mut element = div()
            .id("spelling-menu")
            .debug_selector(|| "spelling-menu".into())
            .track_focus(&menu.focus)
            .occlude()
            .min_w(px(170.))
            .p_1()
            .flex()
            .flex_col()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .text_color(theme.foreground)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.close_spelling_menu(window, cx)
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_spelling_menu(window, cx);
                    cx.stop_propagation();
                }
            }))
            .children(note.map(|note| div().px_3().py_1().text_color(theme.muted).child(note)));
        let mut separated = false;
        for (index, (choice, label)) in choices.into_iter().enumerate() {
            let suggestion = matches!(choice, Choice::Replace(_) | Choice::DeleteRepeated);
            if !suggestion && !separated {
                separated = true;
                if index > 0 || note.is_some() {
                    element = element.child(div().my_1().h(px(1.)).bg(theme.border));
                }
            }
            let selection = theme.selection;
            element = element.child(
                div()
                    .id(index)
                    .debug_selector({
                        let label = label.clone();
                        move || format!("spelling:{label}")
                    })
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(suggestion, |item| {
                        item.font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    .hover(move |style| style.bg(selection))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.choose_spelling(choice.clone(), window, cx)
                    }))
                    .child(label),
            );
        }
        Some(
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(element),
            )
            .with_priority(1),
        )
    }
}

/// The menu's items and their labels, in order: the corrections, then what to do with the word.
/// A read-only note offers no corrections.
fn menu_choices(menu: &SpellMenu, read_only: bool) -> Vec<(Choice, String)> {
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

/// The prose of `lines`: their text without code, Markdown markers, link addresses and web
/// addresses, as one segment per run.
fn prose_segments(
    buffer: &Buffer,
    markdown: &mut MarkdownState,
    lines: Range<usize>,
) -> Vec<Segment> {
    if lines.is_empty() {
        return Vec::new();
    }
    let start = buffer.line_start(lines.start);
    let end = buffer.line_end(lines.end - 1);
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
    let mut bytes = 0;
    for line in lines {
        let line_start = buffer.line_start(line).0;
        let text = buffer.line_text(line);
        let mut cuts: Vec<Range<usize>> = excluded
            .iter()
            .filter(|range| range.start < line_start + text.len() && range.end > line_start)
            .map(|range| {
                range.start.max(line_start) - line_start
                    ..range.end.min(line_start + text.len()) - line_start
            })
            .collect();
        cuts.extend(web_addresses(&text));
        cuts.sort_by_key(|range| range.start);
        let mut from = 0;
        for cut in cuts
            .into_iter()
            .chain(std::iter::once(text.len()..text.len()))
        {
            if cut.start > from {
                let piece = &text[from..cut.start];
                if piece.chars().any(char::is_alphabetic) {
                    bytes += piece.len();
                    segments.push(Segment {
                        offset: line_start + from,
                        text: piece.to_owned(),
                    });
                }
            }
            from = from.max(cut.end);
        }
        if bytes > MAX_CHECK_BYTES {
            break;
        }
    }
    segments
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
    fn long_lines_limit_what_is_checked() {
        let line = "word ".repeat(20_000);
        let buffer = Buffer::from_text(&vec![line.as_str(); 50].join("\n"));
        let lines = lines_to_check(&buffer, 10..12);
        assert!(lines.start <= 10 && 12 <= lines.end, "{lines:?}");
        assert!(lines.len() < 10, "{lines:?}");
        assert_eq!(lines_to_check(&Buffer::from_text("a\nb"), 0..2), 0..2);
    }
}
