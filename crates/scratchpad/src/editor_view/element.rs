//! The custom element that lays out and paints the visible part of an [`EditorView`].

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent,
    Style, Window, fill, point, px, relative, size,
};
use scratchpad_editor::ByteOffset;

use super::line_layout::LineLayout;
use super::scroll::{LineHeights, ScrollAnchor};
use super::{AUTOSCROLL_MARGIN_ROWS, EditorView, base_style, text_column};
use crate::theme::ActiveTheme;

/// Lines shaped beyond each edge of the viewport so that they are ready when scrolled in.
const OVERSCAN_LINES: usize = 4;
const CARET_WIDTH: Pixels = px(2.);
/// Width of the right-edge strip that takes scrollbar clicks.
const SCROLLBAR_TRACK_WIDTH: Pixels = px(12.);
const SCROLLBAR_THUMB_WIDTH: Pixels = px(6.);
const SCROLLBAR_MIN_THUMB: Pixels = px(24.);
const CODE_BLOCK_RADIUS: Pixels = px(6.);

pub(super) struct EditorElement {
    view: Entity<EditorView>,
}

impl EditorElement {
    pub fn new(view: Entity<EditorView>) -> Self {
        Self { view }
    }
}

/// The scrollbar as last laid out, for hit testing and dragging.
///
/// The document's real height is never known (ADR 0030), so the thumb maps a fractional line
/// position estimated from the visible lines' average height.
#[derive(Clone, Copy, Debug)]
pub(super) struct ScrollbarLayout {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// The fractional line position at which the thumb reaches the bottom of the track.
    max_position: f32,
}

impl ScrollbarLayout {
    /// The fractional line position at the top of the viewport when the thumb's top is at `top`.
    pub fn position_for_thumb_top(&self, top: Pixels) -> f32 {
        let travel = self.track.size.height - self.thumb.size.height;
        let fraction = if travel > px(0.) {
            ((top - self.track.top()) / travel).clamp(0., 1.)
        } else {
            0.
        };
        fraction * self.max_position
    }
}

pub(super) struct Frame {
    lines: Vec<(Arc<LineLayout>, gpui::Point<Pixels>)>,
    /// Backgrounds of code blocks, one per run of consecutive code lines.
    code_blocks: Vec<Bounds<Pixels>>,
    selection: Vec<Bounds<Pixels>>,
    /// Underlines of IME composition text.
    marked: Vec<Bounds<Pixels>>,
    cursor: Option<Bounds<Pixels>>,
    scrollbar: Option<ScrollbarLayout>,
    colors: FrameColors,
}

struct FrameColors {
    code_block: Hsla,
    selection: Hsla,
    caret: Hsla,
    composition: Hsla,
    scrollbar: Hsla,
}

pub(super) struct PrepaintState {
    started: Instant,
    frame: Frame,
    hitbox: Hitbox,
    scrollbar_hitbox: Option<Hitbox>,
}

impl EditorView {
    /// Scrolls as requested, shapes the visible lines and computes everything paint needs.
    fn layout_frame(&mut self, bounds: Bounds<Pixels>, window: &Window, cx: &App) -> Frame {
        self.bounds = Some(bounds);
        let (left, width) = text_column(bounds);
        self.layouts
            .set_style(base_style(cx, width, &self.mono_family));
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
            code_blocks: Vec::new(),
            selection: Vec::new(),
            marked: Vec::new(),
            cursor: None,
            scrollbar: None,
            colors: FrameColors {
                code_block: cx.theme().surface,
                selection: cx.theme().selection,
                caret: cx.theme().accent,
                composition: cx.theme().foreground,
                scrollbar: cx.theme().muted.opacity(0.45),
            },
        };
        let mut y = -anchor.offset;
        let mut line = anchor.line;
        let mut painted_height = px(0.);
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
            let height = layout.height();
            if layout.code_block() {
                let block = Bounds::new(origin, size(width, height));
                match frame.code_blocks.last_mut() {
                    Some(last) if last.bottom() == block.top() => last.size.height += height,
                    _ => frame.code_blocks.push(block),
                }
            }
            frame.lines.push((layout, origin));
            painted_height += height;
            y += height;
            line += 1;
        }
        let visible_lines = anchor.line..line;
        let fits =
            anchor == ScrollAnchor::top(&vp) && line == buffer.line_count() && y <= vp.height;

        for ahead in line..(line + OVERSCAN_LINES).min(buffer.line_count()) {
            lines.layout(ahead);
        }
        for behind in anchor.line.saturating_sub(OVERSCAN_LINES)..anchor.line {
            lines.layout(behind);
        }

        if !fits {
            let visible = frame.lines.len().max(1) as f32;
            let average_height = (painted_height / visible).max(px(1.));
            let viewport_lines = vp.height / average_height;
            let position = anchor.line as f32
                + (anchor.offset / lines.height(anchor.line).max(px(1.))).max(0.);
            // The last line can scroll up to the middle of the viewport (see `ScrollAnchor::clamped`).
            let max_position = (buffer.line_count() as f32 - viewport_lines / 2.).max(1.);
            frame.scrollbar = Some(scrollbar_layout(
                bounds,
                position,
                max_position,
                viewport_lines,
            ));
        }
        self.visible_lines = visible_lines;
        self.scrollbar = frame.scrollbar;
        self.layouts.trim_around(anchor.line);
        frame
    }
}

fn scrollbar_layout(
    bounds: Bounds<Pixels>,
    position: f32,
    max_position: f32,
    viewport_lines: f32,
) -> ScrollbarLayout {
    let track = Bounds::new(
        point(bounds.right() - SCROLLBAR_TRACK_WIDTH, bounds.top()),
        size(SCROLLBAR_TRACK_WIDTH, bounds.size.height),
    );
    let content_lines = max_position + viewport_lines;
    let thumb_height = (track.size.height * (viewport_lines / content_lines).min(1.))
        .max(SCROLLBAR_MIN_THUMB)
        .min(track.size.height);
    let travel = track.size.height - thumb_height;
    let thumb_top = track.top() + travel * (position / max_position).clamp(0., 1.);
    let inset = (SCROLLBAR_TRACK_WIDTH - SCROLLBAR_THUMB_WIDTH) / 2.;
    ScrollbarLayout {
        track,
        thumb: Bounds::new(
            point(track.left() + inset, thumb_top),
            size(SCROLLBAR_THUMB_WIDTH, thumb_height),
        ),
        max_position,
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
        let scrollbar_hitbox = frame.scrollbar.map(|scrollbar| {
            window.insert_hitbox(scrollbar.track, HitboxBehavior::BlockMouseExceptScroll)
        });
        PrepaintState {
            started,
            frame,
            hitbox,
            scrollbar_hitbox,
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
        if let Some(scrollbar) = &state.scrollbar_hitbox {
            window.set_cursor_style(CursorStyle::Arrow, scrollbar);
        }
        self.register_mouse_listeners(state, window);

        let frame = &state.frame;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for block in &frame.code_blocks {
                window.paint_quad(
                    fill(*block, frame.colors.code_block).corner_radii(CODE_BLOCK_RADIUS),
                );
            }
            for (layout, origin) in &frame.lines {
                layout.paint_background(*origin, window);
            }
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
            if let Some(scrollbar) = &frame.scrollbar {
                window.paint_quad(
                    fill(scrollbar.thumb, frame.colors.scrollbar)
                        .corner_radii(SCROLLBAR_THUMB_WIDTH / 2.),
                );
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

impl EditorElement {
    fn register_mouse_listeners(&self, state: &PrepaintState, window: &mut Window) {
        let view = self.view.clone();
        let hitbox = state.hitbox.clone();
        let scrollbar = state.scrollbar_hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if scrollbar
                .as_ref()
                .is_some_and(|hitbox| hitbox.is_hovered(window))
            {
                view.update(cx, |view, cx| {
                    view.scrollbar_mouse_down(event.position, window, cx)
                });
                cx.stop_propagation();
            } else if hitbox.is_hovered(window) {
                view.update(cx, |view, cx| view.mouse_down(event, window, cx));
                cx.stop_propagation();
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && view.read(cx).drag.is_some() {
                view.update(cx, |view, cx| view.mouse_move(event, window, cx));
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                view.update(cx, |view, _| view.mouse_up());
            }
        });

        let view = self.view.clone();
        let hitbox = state.hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                view.update(cx, |view, cx| view.scroll_wheel(event, window, cx));
                cx.stop_propagation();
            }
        });
    }
}
