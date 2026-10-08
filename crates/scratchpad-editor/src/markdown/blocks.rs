//! Splitting a document into block regions that can be parsed independently (ADR 0041).
//!
//! A region starts at an unindented line that begins a new top-level block whatever came before it: a non-blank
//! line after a blank line, or an ATX heading or list item, which interrupt paragraphs and are never lazy
//! continuation lines. Not inside a fenced code block or an HTML comment, which continue across such lines, and
//! (for headings and list items) not while an HTML block may be open, which swallows them up to a blank line.
//! Parsing regions separately then gives the same decorations as parsing the whole document (property-tested),
//! except for rarer constructs that ADR 0041 lists and for cuts in very long runs of lines
//! ([`MAX_REGION_LINES`]). Where the nesting of a construct cannot be told from the lines alone, the scan stops
//! starting regions rather than guess.

/// Past this many lines without a region start, any unindented line starts one (outside fences, comments and
/// HTML blocks). Text that runs on that long without a blank line, heading or list item is not prose anyone
/// formats (logs, pasted data), and cutting it keeps keystrokes from reparsing megabytes; the cost is that an
/// emphasis or link spanning the cut, a lazy quote line or a setext heading text right there loses its style.
const MAX_REGION_LINES: usize = 256;

/// Decides line by line where regions start. Lines are fed without their line break.
#[derive(Debug, Default)]
pub(super) struct Scanner {
    open: Option<Open>,
    /// Lines fed since the start of the current region, which is the first line fed.
    lines: usize,
    previous_blank: bool,
    /// A line since the last blank line started with `<`, so an HTML block may be open.
    html: bool,
    /// The delimiter (`.` or `)`) of the top-level ordered list item on the previous line, which the next item
    /// continues even if its number is not 1.
    ordered: Option<u8>,
    /// A list item started since the region start, so an indented line may be inside it.
    in_list: bool,
    /// Where constructs end could not be told without parsing (see [`Open`]), so no more regions start. Only
    /// documents with unusual nesting get here; they are parsed in larger pieces.
    uncertain: bool,
}

/// A construct that continues across blank lines.
#[derive(Debug, Clone, Copy)]
struct Open {
    kind: OpenKind,
    /// The column where the text of the list items around the construct starts, 0 at the top level. A
    /// non-blank line indented less ends the items and with them the construct.
    container: usize,
    /// For a construct opened by an indented line in a list, the indentation of that line: it is inside a list
    /// item or at the top level, depending on the item's indentation. Either way it continues while lines stay
    /// indented that far, but a line indented less ends it only if it is inside an item, which makes the scan
    /// uncertain.
    ambiguous_below: usize,
    /// Opened while an HTML block may be open, in which case the opening line is HTML and the construct ends at
    /// the next blank line instead.
    in_html: bool,
}

#[derive(Debug, Clone, Copy)]
enum OpenKind {
    /// A code fence of `len` `byte`s.
    Fence {
        byte: u8,
        len: usize,
    },
    Comment,
}

impl Scanner {
    /// Feeds the next line and returns whether a region starts at it. The first line fed never starts one: it is
    /// the start of the region being scanned.
    pub fn starts_region(&mut self, line: &str) -> bool {
        let first = self.lines == 0;
        self.lines += 1;
        let (indent, column) = indentation(line);
        let blank = indent == line.len();
        if let Some(open) = self.open {
            self.uncertain |= if blank {
                open.in_html
            } else {
                column < open.ambiguous_below
            };
            if !blank && column < open.container {
                self.open = None;
            }
        }
        let starts = match self.open {
            Some(open) => {
                if open.closes(line) {
                    self.open = None;
                }
                false
            }
            None => {
                let ordered = ordered_item(line);
                let begins_block = is_heading(line)
                    || is_bullet_item(line)
                    || ordered
                        .is_some_and(|(one, delimiter)| one || self.ordered == Some(delimiter));
                let long = self.lines > MAX_REGION_LINES;
                let starts = column == 0
                    && !blank
                    && (self.previous_blank || (!self.html && (begins_block || long)));
                let at_region_start = starts || first;
                if starts {
                    self.lines = 1;
                }
                let in_html = self.html && !at_region_start;
                self.ordered = ordered
                    .filter(|_| at_region_start || (begins_block && !self.html))
                    .map(|(_, delimiter)| delimiter);
                self.html = !blank && (in_html || line[indent..].starts_with('<') && column <= 3);
                self.in_list =
                    starts_item(line.as_bytes(), indent) || self.in_list && !at_region_start;
                // After a paragraph line, an item numbered other than 1 is the paragraph's text unless it
                // continues a list, which is only tracked at the top level.
                let maybe_text = !(first || self.previous_blank || column == 0 && begins_block)
                    && ordered_marker(line, indent).is_some_and(|(one, _)| !one);
                self.open = opens(line).map(|open| Open {
                    ambiguous_below: if open.container == 0 && self.in_list {
                        column
                    } else {
                        0
                    },
                    in_html,
                    ..open
                });
                self.uncertain |= maybe_text && self.open.is_some();
                starts
            }
        };
        self.previous_blank = blank;
        starts && !first && !self.uncertain
    }
}

impl Open {
    fn closes(self, line: &str) -> bool {
        match self.kind {
            // At least as many fence bytes as the opening fence, at most three columns into the container, and
            // nothing but whitespace after them.
            OpenKind::Fence { byte, len } => {
                let (indent, column) = indentation(line);
                let run = count(line.as_bytes(), indent, byte);
                column <= self.container + 3
                    && run >= len
                    && line[indent + run..]
                        .bytes()
                        .all(|b| b == b' ' || b == b'\t')
            }
            OpenKind::Comment => line.contains("-->"),
        }
    }
}

/// Whether `line` opens a fence or an HTML comment that does not end on the same line: after at most three
/// columns of indentation and any list markers, which put it inside list items.
fn opens(line: &str) -> Option<Open> {
    let bytes = line.as_bytes();
    let (indent, column) = indentation(line);
    if column > 3 {
        return None;
    }
    let mut i = indent;
    let mut container = 0;
    while let Some(marker_end) = list_marker_end(bytes, i) {
        // Five or more spaces after a list marker make indented code, not a fence.
        let spaces = count(bytes, marker_end, b' ');
        if spaces > 4 {
            return None;
        }
        i = marker_end + spaces;
        container = column + i - indent;
    }
    let rest = &line[i..];
    let kind = if rest.starts_with("<!--") {
        // `<!-->` and `<!--->` are complete comments.
        (!rest[2..].contains("-->")).then_some(OpenKind::Comment)?
    } else {
        let byte = *bytes.get(i).filter(|b| matches!(b, b'`' | b'~'))?;
        let len = count(bytes, i, byte);
        let info = &rest[len..];
        (len >= 3 && !(byte == b'`' && info.contains('`')))
            .then_some(OpenKind::Fence { byte, len })?
    };
    Some(Open {
        kind,
        container,
        ambiguous_below: 0,
        in_html: false,
    })
}

/// `#` to `######` at the line start, followed by a space, a tab or the line end.
fn is_heading(line: &str) -> bool {
    let hashes = count(line.as_bytes(), 0, b'#');
    (1..=6).contains(&hashes) && matches!(line.as_bytes().get(hashes), None | Some(b' ' | b'\t'))
}

/// A `-`, `*` or `+` item with text at the line start. Without text it could not interrupt a paragraph.
fn is_bullet_item(line: &str) -> bool {
    matches!(line.as_bytes().first(), Some(b'-' | b'*' | b'+'))
        && list_marker_end(line.as_bytes(), 0).is_some_and(|end| !line[end..].trim().is_empty())
}

/// An ordered item with text at the line start: whether its number is 1, and its delimiter.
fn ordered_item(line: &str) -> Option<(bool, u8)> {
    let has_text =
        list_marker_end(line.as_bytes(), 0).is_some_and(|end| !line[end..].trim().is_empty());
    ordered_marker(line, 0).filter(|_| has_text)
}

/// An ordered list marker at `i`: whether its number is 1, which lets it interrupt a paragraph, and its
/// delimiter.
fn ordered_marker(line: &str, i: usize) -> Option<(bool, u8)> {
    let bytes = line.as_bytes();
    let end = list_marker_end(bytes, i).filter(|_| bytes[i].is_ascii_digit())?;
    Some((&line[i..end - 1] == "1", bytes[end - 1]))
}

/// The end of a list marker (`-`, `*`, `+`, `1.` or `1)`) at `i` that is followed by a space.
fn list_marker_end(bytes: &[u8], i: usize) -> Option<usize> {
    list_marker(bytes, i).filter(|&end| bytes.get(end) == Some(&b' '))
}

/// Whether a list item starts at `i`: a list marker followed by whitespace or the line end.
fn starts_item(bytes: &[u8], i: usize) -> bool {
    list_marker(bytes, i).is_some_and(|end| matches!(bytes.get(end), None | Some(b' ' | b'\t')))
}

fn list_marker(bytes: &[u8], i: usize) -> Option<usize> {
    let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    match bytes.get(i + digits)? {
        b'-' | b'*' | b'+' if digits == 0 => Some(i + 1),
        b'.' | b')' if (1..=9).contains(&digits) => Some(i + digits + 1),
        _ => None,
    }
}

/// The byte length and the column width (tabs advance to the next multiple of 4) of the leading whitespace.
fn indentation(line: &str) -> (usize, usize) {
    let mut column = 0;
    let bytes = line
        .bytes()
        .take_while(|&b| match b {
            b' ' => {
                column += 1;
                true
            }
            b'\t' => {
                column += 4 - column % 4;
                true
            }
            _ => false,
        })
        .count();
    (bytes, column)
}

fn count(bytes: &[u8], from: usize, byte: u8) -> usize {
    bytes[from..].iter().take_while(|b| **b == byte).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The indices of the lines of `text` that start regions.
    fn starts(text: &str) -> Vec<usize> {
        let mut scanner = Scanner::default();
        text.split('\n')
            .enumerate()
            .filter(|(_, line)| scanner.starts_region(line))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn regions_start_after_blank_lines_at_unindented_lines() {
        assert_eq!(starts("text\nmore\n\npara\n \nx\n\n  indented\n\n"), [3, 5]);
    }

    #[test]
    fn headings_and_list_items_start_regions_without_blank_lines() {
        assert_eq!(starts("# a\ntext\n## b\n- c\n- d\n* e\nf"), [2, 3, 4, 5]);
        assert_eq!(starts("1. a\n2. b\n1) c\n2) d"), [1, 2, 3]);
        // Not where they would continue a paragraph or are not list items: an ordered item that is not
        // numbered 1 only continues a list of the same delimiter.
        assert_eq!(starts("text\n2. b\n-\n#x\n  - c\n-x\n####### d"), []);
        assert_eq!(starts("- a\n2. b\n1) c\n2. d"), [2]);
    }

    #[test]
    fn html_blocks_swallow_headings_and_list_items_up_to_a_blank_line() {
        assert_eq!(starts("<div>\n# a\n- b\n\n# c\n- d"), [4, 5]);
    }

    #[test]
    fn fences_and_comments_continue_across_blank_lines() {
        assert_eq!(starts("```\na\n\n# b\n```\n\nc"), [6]);
        assert_eq!(starts("~~~~\n\n```\n\nx\n~~~~\n\ny"), [7]);
        assert_eq!(starts("- ```\n\n  x\n  ```\n\ny"), [5]);
        assert_eq!(starts("<!--\n\n# hidden\n-->\n\nshown"), [5]);
        assert_eq!(starts("<!-- one line -->\n\nshown"), [2]);
        assert_eq!(starts("<!-->\n\nshown"), [2]);
    }

    #[test]
    fn fence_lookalikes_do_not_open_fences() {
        // Too short, info string with a backtick, indented code, quoted, no space after the bullet, indented
        // code inside a list item.
        for opener in [
            "``",
            "``` a`b",
            "    ```",
            "\t```",
            "> ```",
            "-```",
            "-     ```",
        ] {
            assert_eq!(starts(&format!("{opener}\n\nx")), [2], "{opener:?}");
        }
    }

    #[test]
    fn a_fence_closes_only_with_a_long_enough_run_of_its_byte() {
        assert_eq!(starts("````\n```\n\nx\n````\n\ny"), [6]);
        assert_eq!(starts("```\n~~~\n\nx\n``` x\n\ny\n```\n\nz"), [9]);
        assert_eq!(starts("```\n    ```\n\nx\n   ```\n\ny"), [6]);
        // Four columns in is fence content, even when the opening fence was indented.
        assert_eq!(starts("   ```\n    ```\n\nx\n```\n\ny"), [6]);
    }

    #[test]
    fn a_fence_in_a_list_item_ends_with_the_item() {
        // The unindented fence line ends the item, so it opens a new fence rather than closing the item's.
        assert_eq!(starts("- ```\n```\n\nx\n```\n\ny"), [6]);
        assert_eq!(starts("- ```\n  a\nb\n\nc"), [4]);
        assert_eq!(starts("1.  <!--\n\n    x\n\ny -->"), [4]);
    }

    #[test]
    fn long_runs_of_lines_are_cut_into_regions() {
        let text = vec!["text"; 600].join("\n");
        assert_eq!(starts(&text), [256, 512]);
        // Never inside a fence.
        let fenced = format!("```\n{text}\n```\nafter");
        assert_eq!(starts(&fenced), [602]);
    }

    #[test]
    fn regions_stop_where_nesting_cannot_be_told_from_lines() {
        // An indented fence in a list closed by an unindented one: in an item, that line opens a new fence.
        assert_eq!(starts("- a\n\n  ```\n  x\n  ```\n\nb"), [6]);
        assert_eq!(starts("- a\n\n  ```\n```\n\nb\n\nc"), []);
        // A fence line in an HTML block is HTML, which ends at a blank line; outside one it opens a fence.
        assert_eq!(starts("<div>\n```\n```\n\nb"), [4]);
        assert_eq!(starts("<div>\n```\n\n```\n\nb\n\nc"), []);
    }
}
