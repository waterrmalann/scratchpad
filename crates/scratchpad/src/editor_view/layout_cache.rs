//! Shaped lines kept across frames, keyed by logical line number (PLAN §23).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::WindowTextSystem;
use scratchpad_editor::{Buffer, TextChange};

use super::line_layout::{BaseStyle, LineLayout};

/// Lines kept when the cache is trimmed, on each side of the line it is trimmed around. Several
/// screens' worth, so scrolling back and forth and moving the cursor never re-shape, while memory
/// stays bounded for any document size.
const KEEP_AROUND: usize = 256;

/// Layouts are invalidated per line by buffer changes, and all at once when the style (font,
/// colours, wrap width) changes.
pub(crate) struct LayoutCache {
    style: BaseStyle,
    lines: HashMap<usize, Arc<LineLayout>>,
}

impl LayoutCache {
    pub fn new(style: BaseStyle) -> Self {
        Self {
            style,
            lines: HashMap::new(),
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

    /// The layout of `line`, shaping it if it is not cached.
    pub fn line(
        &mut self,
        line: usize,
        buffer: &Buffer,
        text_system: &WindowTextSystem,
    ) -> Arc<LineLayout> {
        self.lines
            .entry(line)
            .or_insert_with(|| {
                Arc::new(LineLayout::shape(
                    &buffer.line_text(line),
                    &self.style,
                    text_system,
                ))
            })
            .clone()
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
            .filter_map(|(line, layout)| match line {
                line if line < first => Some((line, layout)),
                line if line <= old_last => None,
                line => Some((line - old_last + new_last, layout)),
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
}
