//! Conservative bracket and quote pairing for typed characters (PLAN §50, ADR 0044).

use crate::coords::ByteOffset;
use crate::editor::Editor;
use crate::motion::Motion;
use crate::selection::Selection;

const PAIRS: [(char, char); 6] = [
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

fn closer_of(open: char) -> Option<char> {
    PAIRS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

impl Editor {
    /// Inserts text the user typed on the keyboard, pairing brackets and quotes:
    /// - an opener gets its closer when the next character is whitespace, a closing bracket or punctuation, or
    ///   the line end, and (for quotes) the previous one is neither a letter or digit nor the same quote, so
    ///   apostrophes and code fences type normally;
    /// - a closer typed right before the same closer moves over it, if that keeps the line's pairs balanced;
    /// - an opener or `*` typed over a selection wraps it, keeping it selected.
    ///
    /// Paste, IME commits and programmatic text use [`Editor::insert_text`].
    pub fn insert_typed(&mut self, text: &str) {
        let mut chars = text.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return self.insert_text(text);
        };
        let selection = self.selection();
        if self.marked_range().is_some() {
            return self.insert_text(text);
        }
        if !selection.is_empty() {
            match closer_of(c).or((c == '*').then_some('*')) {
                Some(close) => self.wrap_selection(c, close),
                None => self.insert_text(text),
            }
            return;
        }

        let cursor = selection.head;
        let (before, after) = self.chars_around(cursor);
        if after == Some(c) && PAIRS.iter().any(|(_, close)| *close == c) && self.line_balanced(c) {
            self.move_cursor(Motion::Right, false);
            return;
        }
        let next_allows = after.is_none_or(|n| n.is_whitespace() || ")]}.,;:!?".contains(n));
        let previous_allows = !is_quote(c) || before.is_none_or(|p| !p.is_alphanumeric() && p != c);
        match closer_of(c) {
            Some(close) if next_allows && previous_allows => self.transact(|editor| {
                editor.replace_range(cursor..cursor, &format!("{c}{close}"));
                editor.set_selection(Selection::cursor(ByteOffset(cursor.0 + c.len_utf8())));
            }),
            _ => self.insert_text(text),
        }
    }

    /// Backspace that also deletes the closer of an empty pair the cursor is in, such as `(|)`.
    pub fn backspace_typed(&mut self) {
        let selection = self.selection();
        if selection.is_empty()
            && let (Some(open), Some(close)) = self.chars_around(selection.head)
            && closer_of(open) == Some(close)
        {
            let start = ByteOffset(selection.head.0 - open.len_utf8());
            let end = ByteOffset(selection.head.0 + close.len_utf8());
            return self.replace_range(start..end, "");
        }
        self.backspace();
    }

    fn wrap_selection(&mut self, open: char, close: char) {
        let selection = self.selection();
        let (start, end) = (selection.start(), selection.end());
        let shift = |offset: ByteOffset| ByteOffset(offset.0 + open.len_utf8());
        self.transact(|editor| {
            editor.replace_range(end..end, close.encode_utf8(&mut [0; 4]));
            editor.replace_range(start..start, open.encode_utf8(&mut [0; 4]));
            editor.set_selection(Selection::new(
                shift(selection.anchor),
                shift(selection.head),
            ));
        });
    }

    /// The chars just before and after `offset`.
    fn chars_around(&self, offset: ByteOffset) -> (Option<char>, Option<char>) {
        let buffer = self.buffer();
        let before = buffer.text_for_range(ByteOffset(offset.0.saturating_sub(4))..offset);
        let after = buffer.text_for_range(offset..ByteOffset(offset.0 + 4));
        (before.chars().next_back(), after.chars().next())
    }

    /// Whether the cursor's line has as many closers `close` as openers (an even number for quotes), so that
    /// typing another `close` would leave one unmatched.
    fn line_balanced(&self, close: char) -> bool {
        let buffer = self.buffer();
        let line = buffer.line_text(buffer.line_of(self.selection().head));
        let open = PAIRS
            .iter()
            .find(|(_, c)| *c == close)
            .map_or(close, |(o, _)| *o);
        if open == close {
            return line.matches(close).count().is_multiple_of(2);
        }
        line.matches(open).count() == line.matches(close).count()
    }
}
