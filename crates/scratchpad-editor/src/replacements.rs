//! Typed replacements (#4, ADR 0180): `->` becomes →, `--` becomes —, `(c)` becomes ©, and so on, on the last
//! character typed. [`RULES`] is the table; this module holds the text-only decisions (which rule matches the text
//! before the cursor, and whether the text around it allows it), and [`Editor`] applies them on the keystroke
//! path, remembers the last replacement so Backspace can take it back, and asks the document's Markdown structure
//! whether the text is code.

use std::ops::Range;

use crate::coords::ByteOffset;
use crate::editor::Editor;
use crate::markdown::{MarkdownState, front_matter};

/// One replacement: `from`, typed, becomes `to` when its last char is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub from: &'static str,
    pub to: &'static str,
    pub guard: Guard,
}

/// What a rule needs of the text around it, beyond what every rule needs (see [`find`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    None,
    /// `--`. Held off while the line up to the cursor is only dashes, `|`, `:` and whitespace: it can still
    /// become a thematic break or front matter fence (`---`), a setext underline or a table's delimiter row. A
    /// `--` that starts a word (`--flag`) is taken back when a letter or digit is typed right after it.
    Dash,
    /// `(c)`, `(r)`, `(tm)`: only at the start of a word (after nothing, whitespace or an opening bracket or
    /// quote), so a call `f(c)` or a link `[x](c)` stays. With `Some(previous)`, also not when `previous` appears
    /// before it on its line or the line before, so enumerations `(a) … (b) … (c)` stay.
    Symbol(Option<&'static str>),
    /// Not right after this char, so `C++-style` keeps its `+-`.
    NotAfter(char),
}

const fn rule(from: &'static str, to: &'static str) -> Rule {
    Rule {
        from,
        to,
        guard: Guard::None,
    }
}

/// The replacements, any order: the longest match wins. `<=>` is not in the issue's table; it is here so that `<=`
/// then `>` does not leave `≤>`, as `<->` keeps `<-` then `>` from leaving `←>`.
pub const RULES: &[Rule] = &[
    rule("->", "→"),
    rule("<-", "←"),
    rule("<->", "↔"),
    rule("=>", "⇒"),
    rule("<=", "≤"),
    rule("<=>", "⇔"),
    rule(">=", "≥"),
    rule("!=", "≠"),
    Rule {
        from: "--",
        to: "—",
        guard: Guard::Dash,
    },
    rule("...", "…"),
    Rule {
        from: "(c)",
        to: "©",
        guard: Guard::Symbol(Some("(b)")),
    },
    Rule {
        from: "(r)",
        to: "®",
        guard: Guard::Symbol(Some("(q)")),
    },
    Rule {
        from: "(tm)",
        to: "™",
        guard: Guard::Symbol(None),
    },
    Rule {
        from: "+-",
        to: "±",
        guard: Guard::NotAfter('+'),
    },
];

/// How far back from the cursor the text is read to decide on a rule: the line before and the cursor's line, cut
/// to this many bytes.
const CONTEXT_BYTES: usize = 512;

/// How far back an opening `<!--` is looked for.
const COMMENT_BYTES: usize = 4096;

/// How much of the line before a match is read to tell whether it is in a code span being typed.
const LINE_BYTES: usize = 16 * 1024;

/// Whether typing `c` can complete a rule. Checked first, so that most keystrokes cost nothing more.
pub fn triggers(c: char) -> bool {
    RULES.iter().any(|rule| rule.from.ends_with(c))
}

/// The rule to apply to `before`, the text before the cursor (just after the typed char), of which only the
/// cursor's line and the line before are read: the longest rule `before` ends with whose guards allow it. Every
/// rule also needs not to be in a web or e-mail address, a path, a link destination, or an autolink or HTML tag
/// being typed: the run of non-whitespace it ends must not hold `/`, `\` or `@` (so neither `://`) or a `](`
/// not closed by a `)`, nor start with `www.` or with `<` and a letter. Whether it is in code is for the caller
/// to ask ([`in_code_span`] and the Markdown structure).
pub fn find(before: &str) -> Option<&'static Rule> {
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[line_start..];
    // The line before and this one.
    let previous_start = before[..line_start.saturating_sub(1)]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let before = &before[previous_start..];
    let run_start = line
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let run = &line[run_start..];
    if looks_like_address(run) {
        return None;
    }
    RULES
        .iter()
        .filter(|rule| line.ends_with(rule.from))
        .filter(|rule| {
            let start = line.len() - rule.from.len();
            let line_before = &line[..start];
            guard_allows(
                rule.guard,
                &before[..before.len() - rule.from.len()],
                line_before,
            )
        })
        .max_by_key(|rule| rule.from.len())
}

/// Whether `guard` allows a rule whose match starts after `before` (which ends with `line_before`).
fn guard_allows(guard: Guard, before: &str, line_before: &str) -> bool {
    match guard {
        Guard::None => true,
        Guard::Dash => !line_before
            .chars()
            .all(|c| matches!(c, '-' | '|' | ':') || c.is_whitespace()),
        Guard::Symbol(previous) => {
            starts_word(line_before.chars().next_back())
                && previous.is_none_or(|previous| !before.contains(previous))
        }
        Guard::NotAfter(not) => !line_before.ends_with(not),
    }
}

/// Whether text after `before` (a char or nothing) starts a word: `before` is nothing, whitespace or an opening
/// bracket or quote.
fn starts_word(before: Option<char>) -> bool {
    before.is_none_or(|c| c.is_whitespace() || "([{\"'“‘".contains(c))
}

/// Whether `run`, the non-whitespace text the match ends, is an address, a path or an autolink or tag.
fn looks_like_address(run: &str) -> bool {
    let mut chars = run.chars();
    let tag = chars.next() == Some('<') && chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let destination = run.rfind("](").is_some_and(|i| !run[i + 2..].contains(')'));
    tag || destination || run.starts_with("www.") || run.contains(['/', '\\', '@'])
}

/// Whether the end of `line` (the line before a match) is in an inline code span, closed or being typed: a run
/// of backticks opened it and no run as long closed it. Backslash-escaped backticks outside a span open nothing.
pub fn in_code_span(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut open = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if open.is_none() => i += 2,
            b'`' => {
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                open = match open {
                    None => Some(run),
                    Some(n) if n == run => None,
                    other => other,
                };
                i += run;
            }
            _ => i += 1,
        }
    }
    open.is_some()
}

/// After `c` was typed at the end of `line` (the line before it): if `line` is only dashes and whitespace and
/// ends with a `--` run and whitespace, held off as the line could still become `---`, and `c` is neither a dash
/// nor whitespace, the line can no longer be a thematic break, and that `--` (at the offset returned) becomes a
/// dash: `- -- item`, `-- signed`. A `--` run right before `c` is a flag (`--x`) and stays.
pub fn held_dash(line: &str, c: char) -> Option<usize> {
    if c == '-' || c.is_whitespace() || !line.chars().all(|c| c == '-' || c.is_whitespace()) {
        return None;
    }
    let dashes = line.trim_end();
    if dashes.len() == line.len() || !dashes.ends_with("--") {
        return None;
    }
    let start = dashes.len() - 2;
    let starts_run = dashes[..start]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    starts_run.then_some(start)
}

/// Whether a match, the last `matched` bytes of `before`, is in an HTML comment: the text before it has a `<!--`
/// after its last `-->`, or the match ends a `<!--` (so its second dash stays a dash, as do those of `-->`).
pub fn in_html_comment(before: &str, matched: usize) -> bool {
    let preceding = &before[..before.len() - matched];
    before.ends_with("<!--")
        || preceding
            .rfind("<!--")
            .is_some_and(|open| !preceding[open + 4..].contains("-->"))
}

/// What the last replacement changed, so Backspace can take it back and a char typed after it can extend it
/// (`<-` then `>`) or take it back (`--` then a letter).
#[derive(Debug, Clone)]
pub(crate) struct Last {
    /// Where the replacement is in the buffer.
    range: Range<ByteOffset>,
    rule: &'static Rule,
    /// The buffer's version right after it.
    version: u64,
    /// Made outside an explicit transaction, so it is the last undo step while `version` is current.
    undoable: bool,
}

/// The editor's typed-replacement state: whether they are on, and what the last one did.
#[derive(Debug, Clone, Default)]
pub(crate) struct Replacements {
    off: bool,
    last: Option<Last>,
    /// The buffer version and cursor right after Backspace took a replacement back. The next typed char then
    /// replaces nothing, so `--`, taken back, then `-` stays `---`.
    reverted: Option<(u64, ByteOffset)>,
}

impl Replacements {
    /// Forgets the last replacement: the cursor moved, so Backspace deletes as usual again.
    pub(crate) fn forget(&mut self) {
        self.last = None;
        self.reverted = None;
    }
}

impl Editor {
    /// Whether typed sequences such as `->` are replaced (on by default).
    pub fn typed_replacements(&self) -> bool {
        !self.replacements().off
    }

    /// Turns typed replacements on or off. Backspace right after a replacement then deletes as usual; a char typed
    /// right after Backspace took one back still replaces nothing.
    pub fn set_typed_replacements(&mut self, on: bool) {
        let replacements = self.replacements_mut();
        replacements.off = !on;
        replacements.last = None;
    }

    /// The last replacement, while the cursor is right after it and nothing changed since.
    fn live_replacement(&self) -> Option<Last> {
        let last = self.replacements().last.clone()?;
        let selection = self.selection();
        (last.version == self.buffer().version()
            && selection.is_empty()
            && selection.head == last.range.end)
            .then_some(last)
    }

    /// Called by [`Editor::insert_typed`] before it inserts `c` with no selection: if `c` extends the last
    /// replacement into a longer rule (`←` then `>` is `<->`), or `c` is a letter or digit after a `--` that started a
    /// word (`--flag`), the last replacement is taken back first, as undo does, so the original text gets `c`.
    pub(crate) fn before_typed_replacement(&mut self, c: char) {
        let suppressed = self.replacements().reverted.is_some_and(|(version, at)| {
            version == self.buffer().version() && self.selection().head == at
        });
        if !suppressed {
            self.replacements_mut().reverted = None;
        }
        if self.replacements().off || suppressed {
            return;
        }
        let Some(last) = self.live_replacement() else {
            return;
        };
        let start = last.range.start;
        let context = self.context_before(start);
        let extended = format!("{context}{}{c}", last.rule.from);
        // A rule that covers all of what the last replacement replaced, and `c`.
        let longer =
            find(&extended).is_some_and(|rule| extended.len() - rule.from.len() <= context.len());
        let flag = last.rule.guard == Guard::Dash
            && c.is_alphanumeric()
            && starts_word(context.chars().next_back());
        if longer || flag {
            self.take_back(&last);
        }
    }

    /// Called by [`Editor::insert_typed`] after it typed `c` with no selection (inserted, or stepped over the same
    /// closer): replaces the rule the text before the cursor now ends with, if any applies here, as its own undo
    /// step.
    pub(crate) fn after_typed_replacement(&mut self, c: char, markdown: &mut MarkdownState) {
        let replacements = self.replacements();
        let cursor = self.selection().head;
        let suppressed = replacements
            .reverted
            .is_some_and(|(_, at)| at.0 + c.len_utf8() == cursor.0);
        self.replacements_mut().reverted = None;
        if self.replacements().off || suppressed || !self.selection().is_empty() {
            return;
        }
        self.replace_held_dash(c, markdown);
        if !triggers(c) {
            return;
        }
        let cursor = self.selection().head;
        let Some(rule) = find(&self.context_before(cursor)) else {
            return;
        };
        let start = ByteOffset(cursor.0 - rule.from.len());
        if self.excluded(start..cursor, markdown) {
            return;
        }
        let undoable = !self.in_transaction();
        self.break_undo_group();
        self.replace_range(start..cursor, rule.to);
        let end = ByteOffset(start.0 + rule.to.len());
        self.replacements_mut().last = Some(Last {
            range: start..end,
            rule,
            version: self.buffer().version(),
            undoable,
        });
    }

    /// A `--` held off on a line of dashes becomes a dash once `c`, typed after it, means the line is no thematic
    /// break ([`held_dash`]), as its own undo step; the cursor stays after `c`.
    fn replace_held_dash(&mut self, c: char, markdown: &mut MarkdownState) {
        let buffer = self.buffer();
        let cursor = self.selection().head;
        let typed_at = ByteOffset(cursor.0 - c.len_utf8());
        let line_start = buffer.line_start(buffer.line_of(cursor));
        // A line break typed starts the line. Lines of dashes are short; a long line is not read.
        if typed_at < line_start || typed_at.0 - line_start.0 > 64 {
            return;
        }
        let line = buffer.text_for_range(line_start..typed_at);
        let Some(offset) = held_dash(&line, c) else {
            return;
        };
        let range = ByteOffset(line_start.0 + offset)..ByteOffset(line_start.0 + offset + 2);
        if self.excluded(range.clone(), markdown) {
            return;
        }
        let after = ByteOffset(cursor.0 - 2 + "—".len());
        self.break_undo_group();
        self.transact(|editor| {
            editor.replace_range(range, "—");
            editor.set_selection(crate::selection::Selection::cursor(after));
        });
    }

    /// Whether the text at `range`, matched by a rule, is where nothing is replaced: in code (a code block or a
    /// span, closed or being typed), front matter or an HTML comment.
    fn excluded(&mut self, range: Range<ByteOffset>, markdown: &mut MarkdownState) -> bool {
        let buffer = self.buffer();
        let line_start = buffer.line_start(buffer.line_of(range.start));
        let line_bytes = (range.start.0 - line_start.0).min(LINE_BYTES);
        // Cheapest first: the Markdown structure may have to parse the edited region.
        in_code_span(&self.text_before(range.start, line_bytes))
            || in_html_comment(
                &self.text_before(range.end, COMMENT_BYTES),
                range.end.0 - range.start.0,
            )
            || self.in_front_matter(range.end)
            || markdown.in_code(self.buffer(), range.clone())
    }

    /// Backspace right after a replacement: brings back the text it replaced, as its own undo step. Returns
    /// false, doing nothing, if the cursor is not right after one.
    pub(crate) fn backspace_replacement(&mut self) -> bool {
        let Some(last) = self.live_replacement() else {
            return false;
        };
        self.replacements_mut().last = None;
        self.replace_range(last.range.clone(), last.rule.from);
        let at = self.selection().head;
        self.replacements_mut().reverted = Some((self.buffer().version(), at));
        true
    }

    /// Puts back the text `last` replaced, with the cursor after it: by undoing it when it is the last undo step,
    /// so undo history shows no trace of it.
    fn take_back(&mut self, last: &Last) {
        self.replacements_mut().last = None;
        if last.undoable && !self.in_transaction() {
            self.undo();
        } else {
            self.replace_range(last.range.clone(), last.rule.from);
        }
    }

    /// The text before `offset`, from the start of the line before its line, at most [`CONTEXT_BYTES`].
    fn context_before(&self, offset: ByteOffset) -> String {
        let buffer = self.buffer();
        let line_start = buffer.line_start(buffer.line_of(offset).saturating_sub(1));
        let bytes = (offset.0 - line_start.0).min(CONTEXT_BYTES);
        self.text_before(offset, bytes)
    }

    /// At most `bytes` of text before `offset` (fewer if that cuts a char).
    fn text_before(&self, offset: ByteOffset, bytes: usize) -> String {
        let start = ByteOffset(offset.0.saturating_sub(bytes));
        let text = self.buffer().text_for_range(start..offset);
        // `text_for_range` rounds a start inside a char down; that char is not wanted.
        let extra = text.len().saturating_sub(bytes);
        text[text.ceil_char_boundary(extra)..].to_owned()
    }

    /// Whether `offset` is in the note's front matter, closed or still being typed ([`front_matter`]).
    fn in_front_matter(&self, offset: ByteOffset) -> bool {
        let line = self.buffer().line_of(offset);
        // Read up to `offset`'s line: lines past it do not change whether it is in front matter.
        line > 0
            && front_matter(self.buffer(), line + 1)
                .is_some_and(|front_matter| line < front_matter.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to(before: &str) -> Option<&'static str> {
        find(before).map(|rule| rule.to)
    }

    #[test]
    fn every_rule_matches_the_text_it_ends() {
        for rule in RULES {
            assert_eq!(
                to(&format!("a {}", rule.from)),
                Some(rule.to),
                "{}",
                rule.from
            );
        }
    }

    #[test]
    fn the_longest_match_wins() {
        assert_eq!(to("a <->"), Some("↔"));
        assert_eq!(to("a <=>"), Some("⇔"));
        assert_eq!(to("a ->"), Some("→"));
        assert_eq!(to("x"), None);
    }

    #[test]
    fn triggers_are_the_last_chars_of_rules() {
        for c in ['>', '-', '=', '.', ')'] {
            assert!(triggers(c), "{c}");
        }
        for c in ['a', '<', '(', ' ', '!', '+'] {
            assert!(!triggers(c), "{c}");
        }
    }

    #[test]
    fn dashes_wait_while_the_line_could_become_a_break_or_a_table_row() {
        assert_eq!(to("--"), None);
        assert_eq!(to("  --"), None);
        assert_eq!(to("- --"), None);
        assert_eq!(to("| --"), None);
        assert_eq!(to("|:--"), None);
        assert_eq!(to("intro\n--"), None);
        assert_eq!(to("a --"), Some("—"));
        assert_eq!(to("a--"), Some("—"));
    }

    #[test]
    fn symbols_start_a_word_and_are_not_enumerations() {
        assert_eq!(to("Copyright (c)"), Some("©"));
        assert_eq!(to("(c)"), Some("©"));
        assert_eq!(to("\"(tm)"), Some("™"));
        assert_eq!(to("f(c)"), None);
        assert_eq!(to("[link](c)"), None);
        assert_eq!(to("x_(r)"), None);
        assert_eq!(to("(a) one (b) two (c)"), None);
        assert_eq!(to("(b) two\n(c)"), None);
        assert_eq!(to("(p) one (q) two (r)"), None);
        assert_eq!(to("(b) far\nline\n(c)"), Some("©"));
    }

    #[test]
    fn addresses_paths_and_tags_are_left_alone() {
        assert_eq!(to("see https://example.com/a..."), None);
        assert_eq!(to("www.example..."), None);
        assert_eq!(to("src/..."), None);
        assert_eq!(to("C:\\dir\\..."), None);
        assert_eq!(to("me@example->"), None);
        assert_eq!(to("<https:->"), None);
        assert_eq!(to("<a->"), None);
        assert_eq!(to("a <->"), Some("↔"));
        assert_eq!(to("wait..."), Some("…"));
        // After wide whitespace (an em space) the run starts past it.
        assert_eq!(to("a\u{2003}->"), Some("→"));
        assert_eq!(to("a\u{2003}src/->"), None);
    }

    #[test]
    fn code_spans_open_until_a_run_of_backticks_as_long_closes_them() {
        assert!(in_code_span("`a"));
        assert!(!in_code_span("`a` "));
        assert!(!in_code_span("a "));
        // Escaped backticks open nothing, outside a span.
        assert!(!in_code_span(r"a \` b "));
        assert!(in_code_span(r"a \` b `c"));
        // Inside a span a backslash is just a backslash.
        assert!(!in_code_span(r"`a\` b "));
        // Runs pair by length.
        assert!(!in_code_span("``a ` b`` "));
        assert!(in_code_span("``a ` b"));
        assert!(in_code_span("``a ` b``` "));
    }

    #[test]
    fn link_destinations_are_left_alone() {
        assert_eq!(to("[draft](notes--"), None);
        assert_eq!(to("[x](a->"), None);
        assert_eq!(to("[a b](c..."), None);
        // After the link, prose again.
        assert_eq!(to("[x](a) ->"), Some("→"));
        assert_eq!(to("[x](a)->"), Some("→"));
        // Brackets that are not a link.
        assert_eq!(to("[x] (a ->"), Some("→"));
    }

    #[test]
    fn plus_minus_is_not_after_another_plus() {
        assert_eq!(to("a +-"), Some("±"));
        assert_eq!(to("5+-"), Some("±"));
        assert_eq!(to("C++-"), None);
    }

    #[test]
    fn a_held_dash_becomes_a_dash_once_the_line_cannot_be_a_rule() {
        assert_eq!(held_dash("- -- ", 'i'), Some(2));
        assert_eq!(held_dash("-- ", 'J'), Some(0));
        assert_eq!(held_dash("  --  ", '('), Some(2));
        // Still a possible rule, or not held.
        assert_eq!(held_dash("- -- ", '-'), None);
        assert_eq!(held_dash("- -- ", ' '), None);
        assert_eq!(held_dash("--- ", 'x'), None);
        assert_eq!(held_dash("- - ", 'x'), None);
        // A flag, a table row, prose.
        assert_eq!(held_dash("- --", 'x'), None);
        assert_eq!(held_dash("| -- ", 'x'), None);
        assert_eq!(held_dash("a -- ", 'x'), None);
    }

    #[test]
    fn html_comments_are_open_from_their_start_to_their_end() {
        assert!(in_html_comment("<!--", 2));
        assert!(in_html_comment("x <!-- note ->", 2));
        assert!(in_html_comment("x <!-- note -->", 2));
        assert!(in_html_comment("<!-- a --> <!-- b ->", 2));
        assert!(in_html_comment(
            "<!--
line
->",
            2
        ));
        assert!(!in_html_comment("<!-- a --> ->", 2));
        assert!(!in_html_comment("a <-", 2));
        assert!(!in_html_comment("->", 2));
    }
}
