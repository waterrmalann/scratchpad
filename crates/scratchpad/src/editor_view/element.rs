//! The custom element that lays out and paints the visible part of an [`EditorView`].

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window, fill, point, px, relative, size,
};
use scratchpad_editor::ByteOffset;

use super::line_layout::LineLayout;
use super::{AUTOSCROLL_MARGIN_ROWS, EditorView, base_style, text_column};
use crate::theme::ActiveTheme;

/// Lines shaped beyond each edge of the viewport so that they are ready when scrolled in.
const OVERSCAN_LINES: usize = 4;
const CARET_WIDTH: Pixels = px(2.);

pub(super) struct EditorElement {
    view: Entity<EditorView>,
}

impl EditorElement {
    pub fn new(view: Entity<EditorView>) -> Self {
        Self { view }
    }
}

pub(super) struct Frame {
    lines: Vec<(Arc<LineLayout>, gpui::Point<Pixels>)>,
    selection: Vec<Bounds<Pixels>>,
    /// Underlines of IME composition text.
    marked: Vec<Bounds<Pixels>>,
    cursor: Option<Bounds<Pixels>>,
    colors: FrameColors,
}

struct FrameColors {
    selection: Hsla,
    caret: Hsla,
    composition: Hsla,
}

pub(super) struct PrepaintState {
    started: Instant,
    frame: Frame,
    hitbox: Hitbox,
}

impl EditorView {
    /// Scrolls as requested, shapes the visible lines and computes everything paint needs.
    fn layout_frame(&mut self, bounds: Bounds<Pixels>, window: &Window, cx: &App) -> Frame {
        self.bounds = Some(bounds);
        let (left, width) = text_column(bounds);
        self.layouts.set_style(base_style(cx, width));
        let vp = self.viewport();
        let margin = self.layouts.style().line_height * AUTOSCROLL_MARGIN_ROWS;
        let selection = self.editor.selection();
        let marked = self.editor.marked_range();
        let autoscroll = std::mem::take(&mut self.autoscroll);

        let (mut lines, scroll) = self.lines(window);
        let buffer = lines.buffer;
        *scroll = if autoscroll {
            let head = buffer.offset_to_point(selection.head);
            let layout = lines.layout(head.line);
            let row_top = layout.row_top(layout.row_of(head.column));
            let rows = row_top..row_top + layout.line_height();
            scroll.revealing(head.line, rows, margin, &mut lines, &vp)
        } else {
            scroll.clamped(&mut lines, &vp)
        };
        let anchor = *scroll;

        let newline_width = lines.cache.style().font_size / 3.;
        let mut frame = Frame {
            lines: Vec::new(),
            selection: Vec::new(),
            marked: Vec::new(),
            cursor: None,
            colors: FrameColors {
                selection: cx.theme().selection,
                caret: cx.theme().accent,
                composition: cx.theme().foreground,
            },
        };
        let mut y = -anchor.offset;
        let mut line = anchor.line;
        while line < buffer.line_count() && y < vp.height {
            let layout = lines.layout(line);
            let origin = point(left, bounds.top() + y);
            let line_range = buffer.line_start(line)..buffer.line_end(line);
            let rect = |row: usize, x0: Pixels, x1: Pixels, top: Pixels, height: Pixels| {
                let row_top = origin.y + layout.row_top(row);
                Bounds::new(point(left + x0, row_top + top), size(x1 - x0, height))
            };
            if let Some(columns) = columns_in(&selection.range(), &line_range) {
                let newline = selection.end() > line_range.end;
                for (row, x0, x1) in layout.spans(columns, newline, newline_width) {
                    frame
                        .selection
                        .push(rect(row, x0, x1, px(0.), layout.line_height()));
                }
            }
            if let Some(columns) = marked.as_ref().and_then(|m| columns_in(m, &line_range)) {
                for (row, x0, x1) in layout.spans(columns, false, px(0.)) {
                    frame
                        .marked
                        .push(rect(row, x0, x1, layout.baseline() + px(2.), px(1.)));
                }
            }
            if line_range.contains(&selection.head) || line_range.end == selection.head {
                let column = selection.head.0 - line_range.start.0;
                let row = layout.row_of(column);
                let x = layout.x_for(column) - CARET_WIDTH / 2.;
                let (text_top, text_height) = layout.text_extent();
                frame.cursor = Some(rect(row, x, x + CARET_WIDTH, text_top, text_height));
            }
            y += layout.height();
            frame.lines.push((layout, origin));
            line += 1;
        }
        let visible_lines = anchor.line..line;

        for ahead in line..(line + OVERSCAN_LINES).min(buffer.line_count()) {
            lines.layout(ahead);
        }
        for behind in anchor.line.saturating_sub(OVERSCAN_LINES)..anchor.line {
            lines.layout(behind);
        }

        self.visible_lines = visible_lines;
        self.layouts.trim_around(anchor.line);
        frame
    }
}

/// The columns of `range` within the line spanning `line`, if they touch it.
fn columns_in(range: &Range<ByteOffset>, line: &Range<ByteOffset>) -> Option<Range<usize>> {
    if range.end < line.start || range.start > line.end {
        return None;
    }
    let start = range.start.max(line.start).0 - line.start.0;
    let end = range.end.min(line.end).0 - line.start.0;
    Some(start..end)
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let started = Instant::now();
        let frame = self
            .view
            .update(cx, |view, cx| view.layout_frame(bounds, window, cx));
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        PrepaintState {
            started,
            frame,
            hitbox,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.view.read(cx).focus_handle.clone();
        let focused = focus_handle.is_focused(window) && window.is_window_active();
        let show_caret = self.view.update(cx, |view, cx| {
            view.set_blinking(focused, cx);
            view.caret_visible()
        });
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &state.hitbox);

        let frame = &state.frame;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for rect in &frame.selection {
                window.paint_quad(fill(*rect, frame.colors.selection));
            }
            for (layout, origin) in &frame.lines {
                layout.paint(*origin, window);
            }
            for underline in &frame.marked {
                window.paint_quad(fill(*underline, frame.colors.composition));
            }
            if let Some(cursor) = frame.cursor.filter(|_| show_caret) {
                window.paint_quad(fill(cursor, frame.colors.caret));
            }
        });

        // Typing latency budget (PLAN §37): `latency` runs from the input event to here, including
        // the wait for the frame; `frame` is the editor's own layout and paint time.
        let frame_time = state.started.elapsed();
        self.view.update(cx, |view, _| {
            if let Some(input_at) = view.input_at.take() {
                tracing::trace!(latency = ?input_at.elapsed(), frame = ?frame_time, "input painted");
            }
        });
    }
}
