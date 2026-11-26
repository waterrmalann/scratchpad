//! The status bar under the note, as in Notepad: where the caret is and how many characters
//! the note has on the left; the zoom level, line endings and encoding on the right. See
//! ADR 0137. Clicking the count shows words, then reading time, then characters again.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    Context, Entity, Focusable, IntoElement, Pixels, Subscription, Task, Window, div, prelude::*,
    px,
};
use scratchpad_editor::{ByteOffset, LineEnding};

use crate::editor_view::{EditorView, MAX_ZOOM, MIN_ZOOM};
use crate::session::{Notice, Session};
use crate::theme::{ActiveTheme, Theme};

const HEIGHT: Pixels = px(26.);

/// Words read in a minute, for the reading time: a usual figure for prose read silently.
const WORDS_PER_MINUTE: usize = 230;

/// Notes longer than this, in bytes, have their words counted again once typing pauses rather
/// than on every keystroke, so typing stays as fast in a note of any length (PLAN §37).
const COUNT_NOW_BYTES: usize = 1 << 20;
const COUNT_DELAY: Duration = Duration::from_millis(300);

/// What the count on the left shows; a click moves to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Count {
    #[default]
    Characters,
    Words,
    ReadingTime,
}

impl Count {
    fn next(self) -> Self {
        match self {
            Count::Characters => Count::Words,
            Count::Words => Count::ReadingTime,
            Count::ReadingTime => Count::Characters,
        }
    }
}

/// Words in the note and in the selection, as of a buffer version and selection.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Words {
    version: u64,
    selection: Range<ByteOffset>,
    total: usize,
    selected: usize,
}

impl Words {
    fn of(editor: &EditorView) -> Self {
        let editor = editor.editor();
        let buffer = editor.buffer();
        let selection = editor.selection().range();
        Self {
            version: buffer.version(),
            total: buffer.word_count_in(ByteOffset(0)..buffer.end()),
            selected: if selection.is_empty() {
                0
            } else {
                buffer.word_count_in(selection.clone())
            },
            selection,
        }
    }

    fn is_current(&self, editor: &EditorView) -> bool {
        let editor = editor.editor();
        self.version == editor.buffer().version() && self.selection == editor.selection().range()
    }
}

/// What the status bar says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// `Ln 44, Col 10`: the caret's line and column, from 1. The column counts characters
    /// (Unicode scalar values) from the start of the line.
    pub position: String,
    /// `2,421 characters`, or `12 of 2,421 characters` while text is selected. A line break
    /// counts as one character. After a click, words (`380 words`, `12 of 380 words`), and
    /// after another the time they take to read (`2 min read`, `< 1 of 2 min read`).
    pub characters: String,
    /// The note's text size, `100%` when not zoomed (ADR 0131); at the smallest and largest
    /// sizes, with an ant and a telescope.
    pub zoom: String,
    /// The line breaks the note is saved with: `Windows (CRLF)` or `Unix (LF)`.
    pub line_ending: &'static str,
    /// `UTF-8`, what notes are saved in, or `Not UTF-8` while a note that is not valid UTF-8 is
    /// shown read-only (ADR 0066).
    pub encoding: &'static str,
}

impl Status {
    /// Counts with the rope's indexes, so a keystroke costs the same in a note of any length.
    /// Words are counted by the caller, who knows when.
    fn of(editor: &EditorView, not_utf8: bool, count: Count, words: Option<&Words>) -> Self {
        let zoom = editor.zoom_percent();
        let editor = editor.editor();
        let buffer = editor.buffer();
        let head = editor.selection().head;
        let line = buffer.line_of(head);
        let column = buffer.char_count_in(buffer.line_start(line)..head);
        let characters = match (count, words) {
            (Count::Words, Some(words)) => {
                let noun = if words.total == 1 { "word" } else { "words" };
                of_total(words.selected, words.total, &with_separators, noun)
            }
            (Count::ReadingTime, Some(words)) => {
                of_total(words.selected, words.total, &reading_minutes, "min read")
            }
            _ => {
                let total = buffer.char_count();
                let selected = buffer.char_count_in(editor.selection().range());
                let noun = if total == 1 {
                    "character"
                } else {
                    "characters"
                };
                of_total(selected, total, &with_separators, noun)
            }
        };
        Self {
            position: format!(
                "Ln {}, Col {}",
                with_separators(line + 1),
                with_separators(column + 1)
            ),
            characters,
            zoom: match zoom {
                MIN_ZOOM => format!("{zoom}% 🐜"),
                MAX_ZOOM => format!("{zoom}% 🔭"),
                _ => format!("{zoom}%"),
            },
            line_ending: match buffer.line_ending() {
                LineEnding::Crlf => "Windows (CRLF)",
                LineEnding::Lf => "Unix (LF)",
            },
            encoding: if not_utf8 { "Not UTF-8" } else { "UTF-8" },
        }
    }
}

/// `2,421 characters`, or `12 of 2,421 characters` with something selected.
fn of_total(selected: usize, total: usize, number: &dyn Fn(usize) -> String, noun: &str) -> String {
    if selected == 0 {
        format!("{} {noun}", number(total))
    } else {
        format!("{} of {} {noun}", number(selected), number(total))
    }
}

/// How many minutes `words` take to read, to the nearest minute: `< 1` under half a minute.
fn reading_minutes(words: usize) -> String {
    match (words + WORDS_PER_MINUTE / 2) / WORDS_PER_MINUTE {
        0 => "< 1".to_owned(),
        minutes => with_separators(minutes),
    }
}

/// `1234567` as `1,234,567`.
fn with_separators(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Shows the [`Status`] of the open note.
pub struct StatusBar {
    editor: Entity<EditorView>,
    not_utf8: bool,
    count: Count,
    /// Counted while the count shows words or reading time; out of date while `recount` waits
    /// for typing in a long note to pause.
    words: Option<Words>,
    recount: Option<Task<()>>,
    status: Status,
    _subscriptions: [Subscription; 2],
}

impl StatusBar {
    pub fn new(
        editor: Entity<EditorView>,
        session: Entity<Session>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = [
            // The editor notifies on every edit and caret move (and blink); the status is only
            // redrawn when it reads differently.
            cx.observe(&editor, |this, _, cx| this.refresh(cx)),
            cx.observe(&session, |this, session, cx| {
                this.not_utf8 = session
                    .read(cx)
                    .notices()
                    .iter()
                    .any(|notice| matches!(notice, Notice::NotUtf8));
                this.refresh(cx);
            }),
        ];
        let status = Status::of(editor.read(cx), false, Count::Characters, None);
        Self {
            editor,
            not_utf8: false,
            count: Count::Characters,
            words: None,
            recount: None,
            status,
            _subscriptions: subscriptions,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Shows the next [`Count`]: characters, words, reading time, and round again.
    pub fn next_count(&mut self, cx: &mut Context<Self>) {
        self.count = self.count.next();
        if self.count == Count::Characters {
            self.words = None;
            self.recount = None;
        }
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.count != Count::Characters {
            self.count_words(cx);
        }
        let status = Status::of(
            self.editor.read(cx),
            self.not_utf8,
            self.count,
            self.words.as_ref(),
        );
        if status != self.status {
            self.status = status;
            cx.notify();
        }
    }

    /// Counts the words again if the text or selection changed: at once in a short note (or when
    /// none were counted yet), once typing pauses in a long one.
    fn count_words(&mut self, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if self
            .words
            .as_ref()
            .is_some_and(|words| words.is_current(editor))
        {
            return;
        }
        if self.words.is_none() || editor.editor().buffer().len() <= COUNT_NOW_BYTES {
            self.words = Some(Words::of(editor));
            self.recount = None;
            return;
        }
        self.recount = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COUNT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.words = Some(Words::of(this.editor.read(cx)));
                this.recount = None;
                this.refresh(cx);
            });
        }));
    }
}

/// A thin line between two parts of the bar.
fn divider(theme: &Theme) -> impl IntoElement {
    div().mx_3().w(px(1.)).h(px(12.)).bg(theme.border)
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = &self.status;
        div()
            .debug_selector(|| "status-bar".into())
            .flex_none()
            .h(HEIGHT)
            .px_4()
            .flex()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .text_xs()
            .text_color(theme.muted)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(status.position.clone())
            .child(divider(theme))
            .child(
                div()
                    .id("status-count")
                    .debug_selector(|| "status-count".into())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        // The click is no reason to stop typing in the note.
                        window.focus(&this.editor.focus_handle(cx));
                        this.next_count(cx);
                    }))
                    .child(status.characters.clone()),
            )
            .child(div().flex_1())
            .child(status.zoom.clone())
            .child(divider(theme))
            .child(status.line_ending)
            .child(divider(theme))
            .child(status.encoding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_separated_by_commas() {
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(1_000), "1,000");
        assert_eq!(with_separators(2_421), "2,421");
        assert_eq!(with_separators(100_000), "100,000");
        assert_eq!(with_separators(1_234_567), "1,234,567");
    }

    #[test]
    fn reading_time_is_to_the_nearest_minute() {
        assert_eq!(reading_minutes(0), "< 1");
        assert_eq!(reading_minutes(114), "< 1");
        assert_eq!(reading_minutes(115), "1");
        assert_eq!(reading_minutes(460), "2");
        assert_eq!(reading_minutes(230 * 1_500), "1,500");
    }
}
