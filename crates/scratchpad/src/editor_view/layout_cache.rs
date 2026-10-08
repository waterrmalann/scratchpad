//! Shaped lines kept across frames, keyed by logical line number (PLAN §23).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::WindowTextSystem;
use scratchpad_editor::{Buffer, TextChange};

use super::line_layout::{BaseStyle, LineKey, LineLayout};

/// Lines kept when the cache is trimmed, on each side of the line it is trimmed around. Several
/// screens' worth, so scrolling back and forth and moving the cursor never re-shape, while memory
/// stays bounded for any document size.
const KEEP_AROUND: usize = 256;

struct Entry {
    key: LineKey,
    layout: Arc<LineLayout>,
    /// The [`LayoutCache::restyle`] generation in which `key` was last found current.
    checked: u64,
}

/// Layouts are invalidated per line by buffer changes, and all at once when the style (font,
/// colours, wrap width) changes. Edits elsewhere and selection changes can change a line's Markdown
/// style too (closing a code fence, revealing markers); after [`LayoutCache::restyle`] each line's
/// [`LineKey`] is recomputed when it is next needed, and the line re-shaped only if it differs.
pub(crate) struct LayoutCache {
    style: BaseStyle,
    lines: HashMap<usize, Entry>,
    generation: u64,
    /// Lines shaped so far, for tests that check what a change re-shapes.
    shaped: u64,
}

impl LayoutCache {
    pub fn new(style: BaseStyle) -> Self {
        Self {
            style,
            lines: HashMap::new(),
            generation: 0,
            shaped: 0,
        }
    }

    pub fn style(&self) -> &BaseStyle {
        &self.style
    }

    pub fn set_style(&mut self, style: BaseStyle) {
        if style != self.style {
            self.style = style;
            self.lines.clear();
        }
    }

    /// The layout of `line`, shaping it if it is not cached or `key` (called only if the line's key
    /// may have changed) differs from the one it was shaped for.
    pub fn line(
        &mut self,
        line: usize,
        buffer: &Buffer,
        text_system: &WindowTextSystem,
        key: impl FnOnce() -> LineKey,
    ) -> Arc<LineLayout> {
        let generation = self.generation;
        if let Some(entry) = self.lines.get_mut(&line) {
            if entry.checked == generation {
                return entry.layout.clone();
            }
            let key = key();
            if entry.key == key {
                entry.checked = generation;
                return entry.layout.clone();
            }
            return self.shape(line, buffer, text_system, key);
        }
        self.shape(line, buffer, text_system, key())
    }

    /// The layout `line` has with `key` rather than its current key, e.g. with the cursor moved
    /// somewhere else. It replaces the cached layout, but [`line`](Self::line) checks its key again.
    pub fn line_with_key(
        &mut self,
        line: usize,
        buffer: &Buffer,
        text_system: &WindowTextSystem,
        key: LineKey,
    ) -> Arc<LineLayout> {
        if let Some(entry) = self.lines.get(&line)
            && entry.key == key
        {
            return entry.layout.clone();
        }
        let layout = self.shape(line, buffer, text_system, key);
        if let Some(entry) = self.lines.get_mut(&line) {
            entry.checked = self.generation.wrapping_sub(1);
        }
        layout
    }

    fn shape(
        &mut self,
        line: usize,
        buffer: &Buffer,
        text_system: &WindowTextSystem,
        key: LineKey,
    ) -> Arc<LineLayout> {
        self.shaped += 1;
        let text = buffer.line_text(line);
        let layout = Arc::new(LineLayout::shape(&text, &key, &self.style, text_system));
        let entry = Entry {
            key,
            layout: layout.clone(),
            checked: self.generation,
        };
        self.lines.insert(line, entry);
        layout
    }

    /// Makes every line check its [`LineKey`] again when next needed: after any edit or selection
    /// change.
    pub fn restyle(&mut self) {
        self.generation += 1;
    }

    /// Drops the layouts of lines touched by `change` and renumbers the lines after it.
    pub fn apply_change(&mut self, change: &TextChange) {
        let first = change.start_point.line;
        let old_last = change.old_end_point.line;
        let new_last = change.new_end_point.line;
        if old_last == new_last {
            for line in first..=old_last {
                self.lines.remove(&line);
            }
            return;
        }
        self.lines = std::mem::take(&mut self.lines)
            .into_iter()
            .filter_map(|(line, entry)| match line {
                line if line < first => Some((line, entry)),
                line if line <= old_last => None,
                line => Some((line - old_last + new_last, entry)),
            })
            .collect();
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Bounds memory by dropping layouts far from `line`.
    pub fn trim_around(&mut self, line: usize) {
        if self.lines.len() > 2 * KEEP_AROUND {
            self.lines
                .retain(|cached, _| cached.abs_diff(line) <= KEEP_AROUND);
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn shaped(&self) -> u64 {
        self.shaped
    }
}
