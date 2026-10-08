//! Markdown-aware Enter (PLAN §49, ADR 0043): continuing list items and block quotes; and Tab and
//! Shift+Tab nesting list items (ADR 0126).

use super::{DecorationKind, MarkdownState};
use crate::buffer::Buffer;
use crate::coords::ByteOffset;
use crate::editor::Editor;
use crate::selection::Selection;

impl Editor {
    /// Enter in a Markdown document. On a list item or quote line it continues the item on the new line
    /// (`- `, `3. ` after `2. `, `- [ ] `, `> `, with the same indentation and enclosing quotes), renumbering the
    /// items after it that would repeat a number. On an item with no text it moves the item out to its parent
    /// list, or at the top level removes the marker, ending the list with a blank line. In a code block it keeps
    /// the line's indentation and the quotes around the block; before the end of the marker it inserts a plain
    /// line break. A selection is replaced first. One undo step.
    pub fn insert_markdown_newline(&mut self, markdown: &mut MarkdownState) {
        self.transact(|editor| {
            if !editor.selection().is_empty() {
                editor.replace_range(editor.selection().range(), "");
            }
            let cursor = editor.selection().head;
            let buffer = editor.buffer();
            let line = buffer.line_of(cursor);
            let line_start = buffer.line_start(line);
            let line_end = buffer.line_end(line);
            let text = buffer.line_text(line).into_owned();
            let column = cursor.0 - line_start.0;
            let decorations = markdown.decorations(buffer, cursor..cursor);
            if decorations
                .iter()
                .any(|d| matches!(d.kind, DecorationKind::CodeBlock { .. }))
            {
                let quotes = decorations
                    .iter()
                    .filter(|d| d.kind == DecorationKind::BlockQuote)
                    .count();
                let keep = code_line_prefix(&text, quotes).min(column);
                return editor.replace_range(cursor..cursor, &format!("\n{}", &text[..keep]));
            }

            let Some(prefix) = continuation(&text).filter(|prefix| column >= prefix.len) else {
                return editor.replace_range(cursor..cursor, "\n");
            };
            if !text[prefix.len..].trim().is_empty() {
                editor.replace_range(cursor..cursor, &format!("\n{}", prefix.next));
                renumber_after(editor, line + 1, &prefix.next);
                return;
            }
            match parent_item(buffer, line, &text, &prefix) {
                Some(parent) => {
                    editor.replace_range(line_start..line_end, &parent.next);
                    renumber_after(editor, line, &parent.next);
                }
                None => editor.replace_range(line_start..line_end, "\n"),
            }
        });
    }
}

impl Editor {
    /// Tab on list items: if every line the selection touches is a list item, nests each one level
    /// deeper (indented by the width of its marker, so it belongs to the item above), with the
    /// lines below it that are indented further (its children), and returns true. Otherwise, and
    /// in code blocks, it does nothing and returns false. One undo step; the selection keeps
    /// covering the same text.
    pub fn indent_list_items(&mut self, markdown: &mut MarkdownState) -> bool {
        self.shift_list_items(markdown, true)
    }

    /// Shift+Tab on list items: like [`Editor::indent_list_items`], but moves each item out to its
    /// parent item's indentation, or at the top level removes its indentation.
    pub fn outdent_list_items(&mut self, markdown: &mut MarkdownState) -> bool {
        self.shift_list_items(markdown, false)
    }

    fn shift_list_items(&mut self, markdown: &mut MarkdownState, deeper: bool) -> bool {
        let buffer = self.buffer();
        let range = self.selection().range();
        let first = buffer.line_of(range.start);
        let mut last = buffer.line_of(range.end);
        // A selection of whole lines ends at the start of the line after them.
        if last > first && buffer.line_start(last) == range.end {
            last -= 1;
        }
        let lines = buffer.line_start(first)..buffer.line_end(last);
        if markdown
            .decorations(buffer, lines)
            .iter()
            .any(|d| matches!(d.kind, DecorationKind::CodeBlock { .. }))
        {
            return false;
        }
        // Where each changed line's indentation starts, and the spaces added (or removed if negative).
        let mut edits: Vec<(ByteOffset, isize)> = Vec::new();
        // Shift+Tab: the parent indentation of the item handled last, with its indentation and quotes. The
        // next item at the same level has the same parent, since only its children are between them;
        // looking it up again for every item would be quadratic in a long list.
        let mut last_parent: Option<(usize, String, usize)> = None;
        let mut line = first;
        while line <= last {
            let text = buffer.line_text(line);
            if text.trim().is_empty() {
                line += 1;
                continue;
            }
            let Some(item) = continuation(&text).filter(|item| item.item_indent.is_some()) else {
                return false;
            };
            let quotes = quotes_end(&text);
            let indent = item.item_indent.unwrap_or(quotes);
            let shift = if deeper {
                marker_width(&text, indent) as isize
            } else if indent == quotes {
                last_parent = None;
                0
            } else {
                let parent = match last_parent.take() {
                    Some((at, prefix, parent)) if at == indent && prefix == text[..quotes] => {
                        parent
                    }
                    _ => parent_item(buffer, line, &text, &item)
                        .and_then(|parent| parent.item_indent)
                        .unwrap_or(quotes),
                };
                last_parent = Some((indent, text[..quotes].to_string(), parent));
                parent as isize - indent as isize
            };
            edits.push((ByteOffset(buffer.line_start(line).0 + quotes), shift));
            // Its children move with it, including selected ones. After a blank line, only lines indented
            // as far as its text still belong to it.
            let text_column = indent + marker_width(&text, indent);
            let mut after_blank = false;
            line += 1;
            while line < buffer.line_count() {
                let child = buffer.line_text(line);
                if child.trim().is_empty() {
                    after_blank = true;
                    line += 1;
                    continue;
                }
                let child_quotes = quotes_end(&child);
                let child_indent = blanks(child.as_bytes(), child_quotes);
                if child_indent <= indent || (after_blank && child_indent < text_column) {
                    break;
                }
                let available = (child_indent - child_quotes) as isize;
                let start = ByteOffset(buffer.line_start(line).0 + child_quotes);
                edits.push((start, shift.max(-available)));
                line += 1;
            }
        }
        if edits.is_empty() {
            return false;
        }
        let map = |offset: ByteOffset| {
            let mut mapped = offset.0 as isize;
            for &(at, shift) in &edits {
                if offset >= at {
                    mapped += shift.max(at.0 as isize - offset.0 as isize);
                }
            }
            ByteOffset(mapped as usize)
        };
        let selection = self.selection();
        let after = Selection::new(map(selection.anchor), map(selection.head));
        self.transact(|editor| {
            // Back to front, so that earlier offsets stay valid.
            for &(at, shift) in edits.iter().rev() {
                if shift > 0 {
                    editor.replace_range(at..at, &" ".repeat(shift as usize));
                } else if shift < 0 {
                    editor.replace_range(at..ByteOffset(at.0 + shift.unsigned_abs()), "");
                }
            }
            editor.set_selection(after);
        });
        true
    }
}

/// The width of the list marker starting at `start` with the spaces after it, e.g. 2 for `- ` and
/// 3 for `1. `.
fn marker_width(line: &str, start: usize) -> usize {
    let bytes = line.as_bytes();
    let digits = count(&bytes[start..], |b| b.is_ascii_digit());
    blanks(bytes, start + digits + 1) - start
}

/// The container markers a line starts with, and what the next line should start with.
#[derive(Debug, PartialEq, Eq)]
struct Continuation {
    /// Length of the markers (and their indentation and spacing) in the line.
    len: usize,
    next: String,
    /// Where the list marker starts, after the quote markers and the indentation; `None` without a list marker.
    item_indent: Option<usize>,
}

/// The list item that contains the item on `line`, judged by indentation, for moving an empty nested item out
/// to it.
fn parent_item(
    buffer: &Buffer,
    line: usize,
    text: &str,
    item: &Continuation,
) -> Option<Continuation> {
    let indent = item.item_indent?;
    let quotes = &text[..quotes_end(text)];
    for above in (0..line).rev() {
        let above = buffer.line_text(above);
        if above.trim().is_empty() {
            continue;
        }
        if !above.starts_with(quotes) {
            return None;
        }
        let above_indent = blanks(above.as_bytes(), quotes_end(&above));
        if above_indent < indent {
            return continuation(&above).filter(|parent| parent.item_indent == Some(above_indent));
        }
    }
    None
}

/// After `line` got a new ordered item numbered as in `prefix`, increments the numbers of the following items
/// of the same list while they repeat the number before them, e.g. `2.` after a new `2.`. Lists numbered `1.`
/// throughout are left alone. The selection, which is before these lines, stays where it is.
fn renumber_after(editor: &mut Editor, mut line: usize, prefix: &str) {
    let Some((mut number, delimiter)) = ordered_number(prefix) else {
        return;
    };
    let selection = editor.selection();
    let indent = blanks(prefix.as_bytes(), quotes_end(prefix));
    let head = &prefix[..indent];
    while line + 1 < editor.buffer().line_count() {
        line += 1;
        let text = editor.buffer().line_text(line).into_owned();
        if text.trim().is_empty() {
            continue;
        }
        let text_indent = blanks(text.as_bytes(), quotes_end(&text));
        if !text.starts_with(head) || text_indent < indent {
            return;
        }
        if text_indent > indent {
            // The text of an item, or a nested list.
            continue;
        }
        if ordered_number(&text) != Some((number, delimiter)) {
            return;
        }
        let start = editor.buffer().line_start(line).0 + indent;
        let digits = number.to_string().len();
        number += 1;
        editor.replace_range(
            ByteOffset(start)..ByteOffset(start + digits),
            &number.to_string(),
        );
        editor.set_selection(selection);
    }
}

/// The number and the delimiter (`.` or `)`) of the ordered list item `line` starts with.
fn ordered_number(line: &str) -> Option<(u64, u8)> {
    let start = blanks(line.as_bytes(), quotes_end(line));
    let digits = count(&line.as_bytes()[start..], |b| b.is_ascii_digit());
    let item = continuation(line).is_some_and(|c| c.item_indent == Some(start));
    if !item || digits == 0 {
        return None;
    }
    let number = line[start..start + digits].parse().ok()?;
    Some((number, line.as_bytes()[start + digits]))
}

/// How much of a code block line to repeat on the next line: the markers of the `quotes` block quotes around
/// the block and the indentation after them.
fn code_line_prefix(line: &str, quotes: usize) -> usize {
    let bytes = line.as_bytes();
    let mut end = 0;
    for _ in 0..quotes {
        let gt = blanks(bytes, end);
        if bytes.get(gt) != Some(&b'>') {
            break;
        }
        end = gt + 1 + usize::from(bytes.get(gt + 1) == Some(&b' '));
    }
    blanks(bytes, end)
}

/// The end of the quote markers (`> `, repeated) a line starts with.
fn quotes_end(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut end = 0;
    while bytes.get(blanks(bytes, end)) == Some(&b'>') {
        end = blanks(bytes, end) + 1;
        if bytes.get(end) == Some(&b' ') {
            end += 1;
        }
    }
    end
}

/// Recognises quote markers (`> `, repeated) followed by an optional list marker (`-`, `*`, `+`, `1.` or `1)`
/// and its spacing) and an optional task box. `None` for lines with neither, and for thematic breaks such as
/// `* * *`, which look like list items.
fn continuation(line: &str) -> Option<Continuation> {
    let bytes = line.as_bytes();
    let quotes_end = quotes_end(line);
    let quotes_only = (quotes_end > 0).then(|| Continuation {
        len: quotes_end,
        next: line[..quotes_end].to_string(),
        item_indent: None,
    });
    if is_thematic_break(&line[quotes_end..]) {
        return quotes_only;
    }

    let marker_start = blanks(bytes, quotes_end);
    let digits = count(&bytes[marker_start..], |b| b.is_ascii_digit());
    let (marker_end, next_number) = match bytes.get(marker_start + digits) {
        Some(b'-' | b'*' | b'+') if digits == 0 => (marker_start + 1, None),
        Some(b'.' | b')') if (1..=9).contains(&digits) => {
            let number: u64 = line[marker_start..marker_start + digits].parse().ok()?;
            (marker_start + digits + 1, Some(number + 1))
        }
        _ => return quotes_only,
    };
    let spacing_end = blanks(bytes, marker_end);
    if spacing_end == marker_end {
        // `-x`, `1.5` or a lone `-` is text, not a list item.
        return quotes_only;
    }

    let mut next = line[..marker_start].to_string();
    match next_number {
        Some(number) => {
            next.push_str(&number.to_string());
            next.push_str(&line[marker_start + digits..spacing_end]);
        }
        None => next.push_str(&line[marker_start..spacing_end]),
    }
    let mut len = spacing_end;
    let task = &line[spacing_end..];
    if ["[ ]", "[x]", "[X]"].iter().any(|b| task.starts_with(b))
        && matches!(task.as_bytes().get(3), None | Some(b' '))
    {
        len = (spacing_end + 4).min(line.len());
        next.push_str("[ ] ");
    }
    Some(Continuation {
        len,
        next,
        item_indent: Some(marker_start),
    })
}

/// `---`, `* * *`, `___` and the like: three or more of one of `-*_`, optionally separated by spaces.
fn is_thematic_break(line: &str) -> bool {
    let mut chars = line.chars().filter(|c| !matches!(c, ' ' | '\t'));
    let Some(first) = chars.next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    let mut n = 1;
    for c in chars {
        if c != first {
            return false;
        }
        n += 1;
    }
    n >= 3
}

/// The end of the spaces and tabs from `from`.
fn blanks(bytes: &[u8], from: usize) -> usize {
    from + count(&bytes[from..], |b| b == b' ' || b == b'\t')
}

fn count(bytes: &[u8], matches: impl Fn(u8) -> bool) -> usize {
    bytes.iter().take_while(|&&b| matches(b)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(line: &str) -> Option<(usize, String)> {
        continuation(line).map(|c| (c.len, c.next))
    }

    #[test]
    fn recognises_list_quote_and_task_prefixes() {
        assert_eq!(next("- a"), Some((2, "- ".into())));
        assert_eq!(next("  *   a"), Some((6, "  *   ".into())));
        assert_eq!(next("9) a"), Some((3, "10) ".into())));
        assert_eq!(next("> > 1. a"), Some((7, "> > 2. ".into())));
        assert_eq!(next(">quote"), Some((1, ">".into())));
        assert_eq!(next("- [x] a"), Some((6, "- [ ] ".into())));
        assert_eq!(next("- [ ]"), Some((5, "- [ ] ".into())));
    }

    #[test]
    fn rejects_text_that_only_resembles_a_marker() {
        for line in [
            "-x",
            "-",
            "1.",
            "1.5 million",
            "plain",
            "  indented",
            "* * *",
            "---",
            "1234567890. a",
        ] {
            assert_eq!(next(line), None, "{line:?}");
        }
        assert_eq!(next("> -x"), Some((2, "> ".into())));
    }
}
