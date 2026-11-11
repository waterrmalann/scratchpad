//! Conservative bracket and quote pairing for typed characters (PLAN §50, ADR 0044).

use crate::coords::ByteOffset;
use crate::editor::Editor;
use crate::markdown::MarkdownState;
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

/// `_` runs around a selection are only counted this far; longer ones are not emphasis anyone types.
const MAX_UNDERSCORES: usize = 16;

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
    /// - with no selection, a sequence such as `->` is replaced by its symbol when its last char is typed (see
    ///   [`crate::replacements`]), except in code;
    /// - an opener, `*` or `~` typed over a selection wraps it, keeping it selected, so typing `*` or `~`
    ///   twice makes it bold or struck through; `_` wraps only where that makes emphasis (not in code, not
    ///   inside a word, nor over an identifier like `foo_bar` or a space) and otherwise replaces the selection.
    ///
    /// `markdown` says whether the selection is in code ([`MarkdownState::in_code`]), for the rules that do not
    /// apply there. Paste, IME commits, programmatic text and plain text files use [`Editor::insert_text`].
    pub fn insert_typed(&mut self, text: &str, markdown: &mut MarkdownState) {
        let mut chars = text.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return self.insert_text(text);
        };
        let selection = self.selection();
        if self.marked_range().is_some() {
            return self.insert_text(text);
        }
        if !selection.is_empty() {
            let wraps = closer_of(c).is_some()
                || matches!(c, '*' | '~')
                || (c == '_' && self.underscore_wraps(selection, markdown));
            if wraps {
                self.wrap_selection(c, closer_of(c).unwrap_or(c));
            } else {
                self.insert_text(text);
            }
            return;
        }

        self.before_typed_replacement(c);
        let cursor = self.selection().head;
        if c == '`' && self.opens_code_fence(cursor) {
            let buffer = self.buffer();
            let line_start = buffer.line_start(buffer.line_of(cursor));
            let line = buffer.text_for_range(line_start..cursor);
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            self.transact(|editor| {
                editor.replace_range(cursor..cursor, &format!("`\n{indent}\n{indent}```"));
                // On the empty line between the fences.
                let inside = cursor.0 + 1 + 1 + indent.len();
                editor.set_selection(Selection::cursor(ByteOffset(inside)));
            });
            return;
        }
        let (before, after) = self.chars_around(cursor);
        if after == Some(c) && PAIRS.iter().any(|(_, close)| *close == c) && self.line_balanced(c) {
            self.move_cursor(Motion::Right, false);
            return self.after_typed_replacement(c, markdown);
        }
        let next_allows = after.is_none_or(|n| n.is_whitespace() || ")]}.,;:!?".contains(n));
        let previous_allows = !is_quote(c) || before.is_none_or(|p| !p.is_alphanumeric() && p != c);
        match closer_of(c) {
            Some(close) if next_allows && previous_allows => self.transact(|editor| {
                editor.replace_range(cursor..cursor, &format!("{c}{close}"));
                editor.set_selection(Selection::cursor(ByteOffset(cursor.0 + c.len_utf8())));
            }),
            _ => {
                self.insert_text(text);
                self.after_typed_replacement(c, markdown);
            }
        }
    }

    /// Backspace that takes back a typed replacement right before the cursor (`→` becomes `->` again), or else
    /// also deletes the closer of an empty pair the cursor is in, such as `(|)`.
    pub fn backspace_typed(&mut self) {
        if self.backspace_replacement() {
            return;
        }
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

    /// Whether a backtick typed at `cursor` completes "```" alone on its line (indentation aside), and so
    /// opens a code block.
    fn opens_code_fence(&self, cursor: ByteOffset) -> bool {
        let buffer = self.buffer();
        let line = buffer.line_of(cursor);
        let start = buffer.line_start(line);
        buffer.text_for_range(start..cursor).trim_start() == "``" && cursor == buffer.line_end(line)
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

    /// Whether `_` typed over `selection` wraps it rather than replacing it, i.e. whether the result would be
    /// emphasis. The selection must hold a letter or digit, no `_` and no whitespace at either end, and must not
    /// be in code. Outside it, past any `_` already wrapped around it (as many on each side, so typing `_` again
    /// makes `__strong__`), there must be no letter or digit: inside a word `_` is not emphasis (CommonMark).
    /// Selecting a space, a `-`, part of a word or an identifier such as `foo_bar`, and typing `_`, means to
    /// replace it, and in code `_` is a name more often than a marker.
    fn underscore_wraps(&self, selection: Selection, markdown: &mut MarkdownState) -> bool {
        let (start, end) = (selection.start(), selection.end());
        let selected = self.buffer().text_for_range(start..end);
        if selected.contains('_')
            || !selected.chars().any(char::is_alphanumeric)
            || selected.starts_with(char::is_whitespace)
            || selected.ends_with(char::is_whitespace)
        {
            return false;
        }
        let runs = (self.underscores_before(start), self.underscores_after(end));
        if runs.0 != runs.1 || markdown.in_code(self.buffer(), start..end) {
            return false;
        }
        let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        !word(self.base_before(ByteOffset(start.0 - runs.0)))
            && !word(self.chars_around(ByteOffset(end.0 + runs.1)).1)
    }

    /// The number of `_` just before `offset` (up to [`MAX_UNDERSCORES`]).
    fn underscores_before(&self, offset: ByteOffset) -> usize {
        let start = ByteOffset(offset.0.saturating_sub(MAX_UNDERSCORES));
        let text = self.buffer().text_for_range(start..offset);
        text.bytes().rev().take_while(|&b| b == b'_').count()
    }

    /// The number of `_` just after `offset` (up to [`MAX_UNDERSCORES`]).
    fn underscores_after(&self, offset: ByteOffset) -> usize {
        let end = ByteOffset((offset.0 + MAX_UNDERSCORES).min(self.buffer().len()));
        let text = self.buffer().text_for_range(offset..end);
        text.bytes().take_while(|&b| b == b'_').count()
    }

    /// The first char of the grapheme that ends at `offset`, so that a letter followed by combining marks (`é`
    /// written as `e` and U+0301) counts as the letter.
    fn base_before(&self, offset: ByteOffset) -> Option<char> {
        let buffer = self.buffer();
        let start = buffer.prev_grapheme_boundary(offset);
        buffer.text_for_range(start..offset).chars().next()
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
