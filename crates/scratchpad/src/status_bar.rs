//! The status bar under the note, as in Notepad: where the caret is and how many characters
//! the note has on the left, its line endings and encoding on the right. See ADR 0137.

use gpui::{Context, Entity, IntoElement, Pixels, Subscription, Window, div, prelude::*, px};
use scratchpad_editor::{Editor, LineEnding};

use crate::editor_view::EditorView;
use crate::session::{Notice, Session};
use crate::theme::{ActiveTheme, Theme};

const HEIGHT: Pixels = px(26.);

/// What the status bar says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// `Ln 44, Col 10`: the caret's line and column, from 1. The column counts characters
    /// (Unicode scalar values) from the start of the line.
    pub position: String,
    /// `2,421 characters`, or `12 of 2,421 characters` while text is selected. A line break
    /// counts as one character.
    pub characters: String,
    /// The line breaks the note is saved with: `Windows (CRLF)` or `Unix (LF)`.
    pub line_ending: &'static str,
    /// `UTF-8`, what notes are saved in, or `Not UTF-8` while a note that is not valid UTF-8 is
    /// shown read-only (ADR 0066).
    pub encoding: &'static str,
}

impl Status {
    /// Counts with the rope's indexes, so a keystroke costs the same in a note of any length.
    fn of(editor: &Editor, not_utf8: bool) -> Self {
        let buffer = editor.buffer();
        let head = editor.selection().head;
        let line = buffer.line_of(head);
        let column = buffer.char_count_in(buffer.line_start(line)..head);
        let total = buffer.char_count();
        let selected = buffer.char_count_in(editor.selection().range());
        let noun = if total == 1 {
            "character"
        } else {
            "characters"
        };
        Self {
            position: format!(
                "Ln {}, Col {}",
                with_separators(line + 1),
                with_separators(column + 1)
            ),
            characters: if selected == 0 {
                format!("{} {noun}", with_separators(total))
            } else {
                format!(
                    "{} of {} {noun}",
                    with_separators(selected),
                    with_separators(total)
                )
            },
            line_ending: match buffer.line_ending() {
                LineEnding::Crlf => "Windows (CRLF)",
                LineEnding::Lf => "Unix (LF)",
            },
            encoding: if not_utf8 { "Not UTF-8" } else { "UTF-8" },
        }
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
        let status = Status::of(editor.read(cx).editor(), false);
        Self {
            editor,
            not_utf8: false,
            status,
            _subscriptions: subscriptions,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let status = Status::of(self.editor.read(cx).editor(), self.not_utf8);
        if status != self.status {
            self.status = status;
            cx.notify();
        }
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
            .child(status.characters.clone())
            .child(div().flex_1())
            // The zoom level (e.g. "100%") goes here, before the line endings, as in Notepad.
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
}
