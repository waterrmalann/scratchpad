//! YAML front matter at the top of a note: a `---` first line, `key: value` lines, and a `---` or `...` line that
//! closes it. Spell checking leaves closed front matter out (ADR 0170); typed replacements also leave alone
//! front matter still being typed (ADR 0180).

use crate::buffer::Buffer;
use crate::coords::ByteOffset;

/// Front matter is looked for this far down the note at most.
pub const MAX_FRONT_MATTER_LINES: usize = 1000;

/// And this much of it at most.
pub const MAX_FRONT_MATTER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontMatter {
    /// The line after it: after its closing line, or, while it is open, after its last line that looks like
    /// front matter.
    pub end: usize,
    /// Whether a `---` or `...` line closes it.
    pub closed: bool,
}

/// The note's front matter, if it has some, read up to line `until` (exclusive), at most
/// [`MAX_FRONT_MATTER_LINES`] and [`MAX_FRONT_MATTER_BYTES`]: the first line is `---` and the second is a `key:`
/// line or a closing line. A `---` followed by anything else is a thematic break, not front matter. Without a
/// closing line in the lines read, it is open and lasts while its lines look like front matter (`key:` lines,
/// indented lines, `- ` items and `#` comments), up to the first that does not, such as a blank line.
pub fn front_matter(buffer: &Buffer, until: usize) -> Option<FrontMatter> {
    let lines = buffer.line_count().min(MAX_FRONT_MATTER_LINES).min(until);
    // A long first line is no fence, and need not be copied to find out.
    let first_end = buffer.line_end(0);
    if lines < 2
        || first_end.0 > 8
        || !is_fence(&buffer.text_for_range(ByteOffset(0)..first_end), &["---"])
    {
        return None;
    }
    // Read at once: a line at a time costs a rope lookup or two for each.
    let start = buffer.line_start(1);
    let end = buffer
        .line_end(lines - 1)
        .min(ByteOffset(start.0 + MAX_FRONT_MATTER_BYTES));
    let text = buffer.text_for_range(start..end);
    let mut read = text.split('\n');
    let second = read.clone().next().unwrap_or_default();
    if !is_fence(second, &["---", "..."]) && !is_key(second) {
        return None;
    }
    // One pass: the closing line may come after lines that do not look like front matter (blank lines).
    let mut open_end = None;
    let mut line = 1;
    for text in read.by_ref() {
        if is_fence(text, &["---", "..."]) {
            return Some(FrontMatter {
                end: line + 1,
                closed: true,
            });
        }
        if open_end.is_none() && !looks_like_front_matter(text) {
            open_end = Some(line);
        }
        line += 1;
    }
    Some(FrontMatter {
        end: open_end.unwrap_or(line),
        closed: false,
    })
}

/// Whether `line` is one of `fences`, trailing whitespace aside.
fn is_fence(line: &str, fences: &[&str]) -> bool {
    line.len() <= 8 && fences.contains(&line.trim_end())
}

/// `key: value` or `key:`, the key starting with a letter, digit, `_` or a quote.
fn is_key(line: &str) -> bool {
    line.starts_with(|c: char| c.is_alphanumeric() || "_\"'".contains(c))
        && line
            .split_once(':')
            .is_some_and(|(_, rest)| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

fn looks_like_front_matter(line: &str) -> bool {
    let continued = line.starts_with(char::is_whitespace) && !line.trim().is_empty();
    is_key(line) || continued || line.starts_with("- ") || line.starts_with('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Option<(usize, bool)> {
        front_matter(&Buffer::from_text(text), usize::MAX).map(|f| (f.end, f.closed))
    }

    #[test]
    fn closed_front_matter_ends_after_its_closing_line() {
        assert_eq!(found("---\ntitle: a\n---\ntext"), Some((3, true)));
        assert_eq!(
            found("---\ntitle: a\n\ntags: [x]\n...\ntext"),
            Some((5, true))
        );
        assert_eq!(found("---\n---\ntext"), Some((2, true)));
    }

    #[test]
    fn open_front_matter_lasts_while_its_lines_look_like_it() {
        assert_eq!(found("---\ntitle: a\n"), Some((2, false)));
        assert_eq!(
            found("---\ntitle: a\ntags:\n  - x\n- y\n# note\n\ntext"),
            Some((6, false))
        );
        assert_eq!(found("---\ntitle: a\nIntro paragraph"), Some((2, false)));
    }

    #[test]
    fn only_the_lines_before_until_are_read() {
        let text = "---
title: a
b: c
---
text";
        assert_eq!(
            front_matter(&Buffer::from_text(text), 3).map(|f| (f.end, f.closed)),
            Some((3, false))
        );
        assert_eq!(
            front_matter(&Buffer::from_text(text), 4).map(|f| (f.end, f.closed)),
            Some((4, true))
        );
    }

    #[test]
    fn a_rule_followed_by_prose_is_no_front_matter() {
        assert_eq!(found("---\nIntro paragraph\n\nmore\n---\n"), None);
        assert_eq!(found("---\n\ntitle: a\n---"), None);
        assert_eq!(found("---\nhttp://x.com\n---"), None);
        assert_eq!(found("text\n---\ntitle: a\n---"), None);
        assert_eq!(found("---"), None);
        assert_eq!(
            found(
                "---     x
title: a
---"
            ),
            None
        );
    }
}
