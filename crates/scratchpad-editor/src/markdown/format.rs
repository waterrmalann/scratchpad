//! Markdown formatting commands (PLAN §36, ADR 0043): toggling emphasis, strong, strikethrough and inline code
//! by editing delimiters in the text.

use std::ops::Range;

use super::{Decoration, DecorationKind, MarkdownState};
use crate::buffer::Buffer;
use crate::coords::ByteOffset;
use crate::editor::Editor;
use crate::motion;
use crate::selection::Selection;

/// Inline formatting written as delimiters at both ends of the text.
#[derive(Clone, Copy)]
struct Format {
    /// What toggling on inserts at each end.
    marker: &'static str,
    /// Delimiter bytes recognised in unparsed runs, e.g. `*` and `_` for emphasis.
    bytes: &'static [u8],
    kind: fn(&DecorationKind) -> bool,
}

const BOLD: Format = Format {
    marker: "**",
    bytes: b"*_",
    kind: |kind| matches!(kind, DecorationKind::Strong),
};
const ITALIC: Format = Format {
    marker: "*",
    bytes: b"*_",
    kind: |kind| matches!(kind, DecorationKind::Emphasis),
};
const STRIKETHROUGH: Format = Format {
    marker: "~~",
    bytes: b"~",
    kind: |kind| matches!(kind, DecorationKind::Strikethrough),
};
const CODE: Format = Format {
    marker: "`",
    bytes: b"`",
    kind: |kind| matches!(kind, DecorationKind::InlineCode),
};

impl Format {
    /// Whether runs of `len` delimiters at both ends already apply this formatting. Runs are shared: `***x***` is
    /// both strong (2) and emphasis (1). Counting modulo twice the marker length makes toggling its own inverse:
    /// adding a marker to a run where the formatting is absent always makes it present, and the reverse.
    fn applies(self, len: usize) -> bool {
        let marker = self.marker.len();
        len % (2 * marker) >= marker
    }
}

/// Run lengths are only inspected this far; longer runs are not formatting anyone types.
const MAX_RUN: usize = 16;

/// A replacement of `range` of the buffer. A selection end at an insertion point moves past the inserted text if
/// `after` is set.
#[derive(Debug)]
struct Change {
    range: Range<usize>,
    text: &'static str,
    after: bool,
}

impl Change {
    fn delete(range: Range<ByteOffset>) -> Self {
        Self {
            range: range.start.0..range.end.0,
            text: "",
            after: false,
        }
    }

    fn insert(at: usize, text: &'static str, after: bool) -> Self {
        Self {
            range: at..at,
            text,
            after,
        }
    }
}

impl Editor {
    /// Ctrl+B: makes the selection bold, or not bold if all its text already is (like a word processor). With
    /// a cursor at the end of a bold span's text it steps over the closing `**`, so that typing goes on
    /// unformatted (Ctrl+B, type, Ctrl+B, type); elsewhere in or right after a bold span it removes the span,
    /// else formats the word around the cursor, else inserts `****` with the cursor in the middle. Bold spans
    /// the selection overlaps are merged into one. One undo step; the selection keeps covering the same text.
    pub fn toggle_bold(&mut self, markdown: &mut MarkdownState) {
        self.toggle_format(markdown, BOLD);
    }

    /// Ctrl+I: like [`Editor::toggle_bold`] with `*`.
    pub fn toggle_italic(&mut self, markdown: &mut MarkdownState) {
        self.toggle_format(markdown, ITALIC);
    }

    /// Like [`Editor::toggle_bold`] with `~~`.
    pub fn toggle_strikethrough(&mut self, markdown: &mut MarkdownState) {
        self.toggle_format(markdown, STRIKETHROUGH);
    }

    /// Like [`Editor::toggle_bold`] with a backtick.
    pub fn toggle_inline_code(&mut self, markdown: &mut MarkdownState) {
        self.toggle_format(markdown, CODE);
    }

    /// Ctrl+K: replaces the selection with `[selection]()` and puts the cursor between the parentheses, or
    /// between the brackets of `[]()` if nothing is selected. One undo step.
    pub fn insert_link(&mut self) {
        let range = self.selection().range();
        let text = self.buffer().text_for_range(range.clone()).into_owned();
        let cursor = if text.is_empty() {
            range.start.0 + 1
        } else {
            range.start.0 + text.len() + 3
        };
        self.transact(|editor| {
            editor.replace_range(range, &format!("[{text}]()"));
            editor.set_selection(Selection::cursor(ByteOffset(cursor)));
        });
    }

    fn toggle_format(&mut self, markdown: &mut MarkdownState, format: Format) {
        let selection = self.selection();
        if let Some(end) = closing_marker_end(self.buffer(), markdown, format, selection) {
            self.set_selection(Selection::cursor(end));
            return;
        }
        let changes = format_changes(self.buffer(), markdown, format, selection);
        let map = |offset: ByteOffset| ByteOffset(map_through(&changes, offset.0));
        let after = Selection::new(map(selection.anchor), map(selection.head));
        self.transact(|editor| {
            // Back to front, so that earlier ranges stay valid.
            for change in changes.iter().rev() {
                let range = ByteOffset(change.range.start)..ByteOffset(change.range.end);
                editor.replace_range(range, change.text);
            }
            editor.set_selection(after);
        });
    }
}

/// The end of the closing marker of a `format` span if `selection` is a cursor right before it, at the end of the
/// span's text.
fn closing_marker_end(
    buffer: &Buffer,
    markdown: &mut MarkdownState,
    format: Format,
    selection: Selection,
) -> Option<ByteOffset> {
    let cursor = selection.head;
    if !selection.is_empty() {
        return None;
    }
    let decorations = markdown.decorations(buffer, cursor..cursor);
    decorations
        .iter()
        .filter(|d| (format.kind)(&d.kind) && d.markers.len() == 2)
        .find_map(|d| (d.markers[1].start == cursor).then_some(d.markers[1].end))
}

/// The changes, ordered and not overlapping, that toggle `format` for `selection`.
fn format_changes(
    buffer: &Buffer,
    markdown: &mut MarkdownState,
    format: Format,
    selection: Selection,
) -> Vec<Change> {
    let range = selection.range();
    let decorations = markdown.decorations(buffer, range.clone());
    let spans: Vec<&Decoration> = decorations
        .iter()
        .filter(|d| (format.kind)(&d.kind))
        .collect();

    if range.is_empty() {
        let cursor = range.start;
        // An empty pair around the cursor, which the parser does not see, is removed first.
        if format.applies(outer_run(buffer, format, cursor.0, cursor.0)) {
            return textual_changes(buffer, format, cursor.0, cursor.0);
        }
        // The innermost span the cursor is in or right after.
        if let Some(span) = spans
            .iter()
            .rev()
            .find(|d| d.range.start < cursor && cursor <= d.range.end)
        {
            return span.markers.iter().cloned().map(Change::delete).collect();
        }
        let word = motion::word_around(buffer, cursor).unwrap_or(cursor..cursor);
        return textual_changes(buffer, format, word.start.0, word.end.0);
    }

    let range = trim_whitespace(buffer, range);
    let overlapping = |d: &&&Decoration| d.range.start < range.end && range.start < d.range.end;
    let overlaps = spans.iter().any(|d| overlapping(&d));
    if overlaps && covered(buffer, &decorations, &spans, &range) {
        let mut changes: Vec<Change> = spans
            .iter()
            .filter(overlapping)
            .flat_map(|d| d.markers.iter().cloned().map(Change::delete))
            .collect();
        changes.sort_by_key(|change| change.range.start);
        return changes;
    }
    // Delimiters right around the selection are removed whatever the parser pairs them with, which undoes
    // toggling on when the selection's neighbours have stray delimiters.
    if !overlaps && format.applies(outer_run(buffer, format, range.start.0, range.end.0)) {
        return textual_changes(buffer, format, range.start.0, range.end.0);
    }

    // Merge the selection with the spans it overlaps or touches into one span.
    let merged: Vec<&&Decoration> = spans
        .iter()
        .filter(|d| d.range.start <= range.end && range.start <= d.range.end)
        .collect();
    if merged.is_empty() {
        return textual_changes(buffer, format, range.start.0, range.end.0);
    }
    let start = merged
        .iter()
        .map(|d| d.range.start)
        .fold(range.start, Ord::min)
        .0;
    let end = merged
        .iter()
        .map(|d| d.range.end)
        .fold(range.end, Ord::max)
        .0;
    let mut changes = vec![Change::insert(start, format.marker, true)];
    changes.extend(
        merged
            .iter()
            .flat_map(|d| d.markers.iter().cloned().map(Change::delete)),
    );
    changes.push(Change::insert(end, format.marker, false));
    changes.sort_by_key(|change| change.range.start);
    changes
}

/// Toggles `format` on `start..end` by looking at delimiter runs just outside it, then just inside it, for
/// text the parser does not see as formatted (an empty pair, `** x **`).
fn textual_changes(buffer: &Buffer, format: Format, start: usize, end: usize) -> Vec<Change> {
    let n = format.marker.len();
    if format.applies(outer_run(buffer, format, start, end)) {
        vec![
            Change::delete(ByteOffset(start - n)..ByteOffset(start)),
            Change::delete(ByteOffset(end)..ByteOffset(end + n)),
        ]
    } else if format.applies(inner_run(buffer, format, start, end)) {
        vec![
            Change::delete(ByteOffset(start)..ByteOffset(start + n)),
            Change::delete(ByteOffset(end - n)..ByteOffset(end)),
        ]
    } else {
        vec![
            Change::insert(start, format.marker, true),
            Change::insert(end, format.marker, false),
        ]
    }
}

/// Where `offset` ends up after `changes`.
fn map_through(changes: &[Change], offset: usize) -> usize {
    let mut mapped = offset;
    for change in changes {
        let Range { start, end } = change.range;
        if offset < start || (offset == start && start == end && !change.after) {
            break;
        }
        if offset < end {
            return mapped - (offset - start);
        }
        mapped = mapped + change.text.len() - (end - start);
    }
    mapped
}

/// `range` without leading and trailing whitespace, unless that is all it has: formatting does not apply to
/// whitespace at its ends (`** x**` is not bold).
fn trim_whitespace(buffer: &Buffer, range: Range<ByteOffset>) -> Range<ByteOffset> {
    let text = buffer.text_for_range(range.clone());
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return range;
    }
    let start = range.start.0 + (text.len() - text.trim_start().len());
    ByteOffset(start)..ByteOffset(start + trimmed.len())
}

/// Whether all text in `range` that live preview shows (everything but markers and whitespace) lies in the
/// content of one of `spans`.
fn covered(
    buffer: &Buffer,
    decorations: &[Decoration],
    spans: &[&Decoration],
    range: &Range<ByteOffset>,
) -> bool {
    let mut allowed: Vec<&Range<ByteOffset>> = decorations
        .iter()
        .flat_map(|d| &d.markers)
        .chain(spans.iter().map(|d| &d.content))
        .collect();
    allowed.sort_by_key(|r| r.start);
    let mut from = range.start;
    for r in allowed {
        if r.start > from
            && !buffer
                .text_for_range(from..r.start.min(range.end))
                .trim()
                .is_empty()
        {
            return false;
        }
        from = from.max(r.end);
        if from >= range.end {
            return true;
        }
    }
    buffer.text_for_range(from..range.end).trim().is_empty()
}

/// The shorter of the delimiter runs just before `start` and just after `end`, or 0 unless both are runs of the
/// same delimiter byte.
fn outer_run(buffer: &Buffer, format: Format, start: usize, end: usize) -> usize {
    let before =
        buffer.text_for_range(ByteOffset(start.saturating_sub(MAX_RUN))..ByteOffset(start));
    let after = buffer.text_for_range(ByteOffset(end)..ByteOffset(end + MAX_RUN));
    shared_run(
        format,
        before.as_bytes().iter().rev(),
        after.as_bytes().iter(),
    )
}

/// Like [`outer_run`] for the runs at the start and end of `start..end` itself. Both runs fit inside it.
fn inner_run(buffer: &Buffer, format: Format, start: usize, end: usize) -> usize {
    let head = buffer.text_for_range(ByteOffset(start)..ByteOffset(end.min(start + MAX_RUN)));
    let tail =
        buffer.text_for_range(ByteOffset(end.saturating_sub(MAX_RUN).max(start))..ByteOffset(end));
    let run = shared_run(format, head.as_bytes().iter(), tail.as_bytes().iter().rev());
    if 2 * run <= end - start { run } else { 0 }
}

/// The length of the shorter of the runs that `left` and `right` start with, if they are runs of the same
/// delimiter byte.
fn shared_run<'a>(
    format: Format,
    left: impl Iterator<Item = &'a u8> + Clone,
    right: impl Iterator<Item = &'a u8> + Clone,
) -> usize {
    let byte = match (left.clone().next(), right.clone().next()) {
        (Some(a), Some(b)) if a == b && format.bytes.contains(a) => *a,
        _ => return 0,
    };
    let left_len = left.take_while(|b| **b == byte).count();
    let right_len = right.take_while(|b| **b == byte).count();
    left_len.min(right_len)
}
