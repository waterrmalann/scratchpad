//! The editor view: the typing surface on top of the editing engine (PLAN §10–16, §22–27, §45–46).
//!
//! [`EditorView`] owns the engine's [`Editor`], which is the source of truth for text, selection and
//! history. The view only adds what is about presentation: the scroll position, shaped line layouts,
//! caret blinking. [`element::EditorElement`] lays out and paints the visible lines.

mod element;
mod geometry;
mod layout_cache;
mod line_layout;
mod scroll;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, Pixels, Subscription, Task, Window,
    WindowTextSystem, div, font, prelude::*, px,
};
use scratchpad_editor::{Bias, Buffer, ByteOffset, Editor, Goal, Motion};

use crate::actions::editor::*;
use crate::theme::{ActiveTheme, typography};
use element::EditorElement;
use layout_cache::LayoutCache;
use line_layout::{BaseStyle, LineLayout};
use scroll::{LineHeights, ScrollAnchor, Viewport};

/// Caret on/off period (PLAN §24).
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// The text column never gets wider than this; wider panes centre it. Around 85 characters of body
/// text, a comfortable measure for prose.
const MAX_TEXT_WIDTH: Pixels = px(680.);
const MIN_SIDE_PADDING: Pixels = px(32.);
const TOP_PADDING: Pixels = px(32.);
/// Rows kept between the cursor and the viewport edge when the view scrolls to the cursor.
const AUTOSCROLL_MARGIN_ROWS: f32 = 2.;

/// A text editor for one document.
pub struct EditorView {
    editor: Editor,
    focus_handle: FocusHandle,
    layouts: LayoutCache,
    /// Buffer version the layouts and scroll anchor were last updated to.
    synced_version: u64,
    scroll: ScrollAnchor,
    /// Set by keyboard input; the next layout scrolls the cursor into view.
    autoscroll: bool,
    /// Element bounds from the last layout; `None` until the first frame.
    bounds: Option<Bounds<Pixels>>,
    visible_lines: Range<usize>,
    cursor_visible: bool,
    /// Running while the editor is focused in an active window.
    blink_task: Option<Task<()>>,
    /// When the input being processed arrived, for the input-to-paint trace (PLAN §37).
    input_at: Option<Instant>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for EditorView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditorView {
    pub fn new(text: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        // GPUI treats an inactive window as having nothing focused, so this also fires when the
        // user switches to another window.
        let subscriptions = vec![cx.on_blur(&focus_handle, window, |this, _, _| {
            this.editor.break_undo_group();
        })];
        let base_style = base_style(cx, MAX_TEXT_WIDTH);
        Self {
            editor: Editor::from_text(text),
            focus_handle,
            layouts: LayoutCache::new(base_style),
            synced_version: 0,
            scroll: ScrollAnchor::top(&viewport(None)),
            autoscroll: false,
            bounds: None,
            visible_lines: 0..0,
            cursor_visible: true,
            blink_task: None,
            input_at: None,
            _subscriptions: subscriptions,
        }
    }

    /// Replaces the document (e.g. when another note is opened): fresh history, cursor and scroll
    /// position at the start.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.editor = Editor::from_text(text);
        self.layouts.clear();
        self.synced_version = self.editor.buffer().version();
        self.scroll = ScrollAnchor::top(&viewport(self.bounds));
        cx.notify();
    }

    /// The document as it should be saved, with its original line endings.
    pub fn text(&self) -> String {
        self.editor.to_text()
    }

    /// Read access to the engine: buffer, selection, history state.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// Logical lines painted in the last frame.
    pub fn visible_lines(&self) -> Range<usize> {
        self.visible_lines.clone()
    }

    /// Whether the caret is drawn right now: the editor is focused in the active window and the
    /// caret is in the "on" phase of its blink.
    pub fn caret_visible(&self) -> bool {
        self.cursor_visible && self.blink_task.is_some()
    }

    /// Number of shaped lines kept in the layout cache, for virtualization tests.
    #[doc(hidden)]
    pub fn cached_layout_count(&self) -> usize {
        self.layouts.len()
    }

    // --- Engine plumbing ---

    fn selection_changed(&mut self, cx: &mut Context<Self>) {
        self.input_at.get_or_insert_with(Instant::now);
        // Typing and moving keep the caret solid; blinking resumes after a pause.
        self.cursor_visible = true;
        if self.blink_task.is_some() {
            self.blink_task = Some(self.spawn_blink(cx));
        }
        cx.notify();
    }

    fn motion(&mut self, motion: Motion, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_cursor(motion, extend);
        self.autoscroll = true;
        self.selection_changed(cx);
    }

    /// Starts or stops the caret blink. Called on every paint with whether the editor is focused in
    /// the active window: focus events alone miss the focus the window starts with.
    fn set_blinking(&mut self, blinking: bool, cx: &mut Context<Self>) {
        if blinking != self.blink_task.is_some() {
            self.cursor_visible = true;
            self.blink_task = blinking.then(|| self.spawn_blink(cx));
        }
    }

    fn spawn_blink(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let blinked = this.update(cx, |this, cx| {
                    this.cursor_visible = !this.cursor_visible;
                    cx.notify();
                });
                if blinked.is_err() {
                    break;
                }
            }
        })
    }

    /// Brings the layouts and scroll anchor up to date with edits made since the last sync.
    fn sync_layouts(&mut self) {
        let buffer = self.editor.buffer();
        if buffer.version() == self.synced_version {
            return;
        }
        match buffer.changes_since(self.synced_version) {
            Some(changes) => {
                for change in changes {
                    self.layouts.apply_change(change);
                    self.scroll.apply_change(change);
                }
            }
            None => self.layouts.clear(),
        }
        self.synced_version = buffer.version();
    }

    /// The document's laid-out lines and the scroll anchor, in sync with the buffer.
    fn lines<'a>(&'a mut self, window: &'a Window) -> (Lines<'a>, &'a mut ScrollAnchor) {
        self.sync_layouts();
        let lines = Lines {
            buffer: self.editor.buffer(),
            cache: &mut self.layouts,
            text_system: window.text_system(),
        };
        (lines, &mut self.scroll)
    }

    fn viewport(&self) -> Viewport {
        viewport(self.bounds)
    }

    // --- Keyboard ---

    /// Moves the cursor to the row `distance` away (one row for Up/Down, a page for PageUp/Down),
    /// keeping its horizontal position as the goal.
    fn move_vertically(
        &mut self,
        distance: Pixels,
        extend: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.editor.selection();
        let goal = self.editor.goal();
        let (mut lines, _) = self.lines(window);
        let (offset, x) = vertical_target(&mut lines, selection.head, goal, distance);
        self.editor
            .move_to_with_goal(offset, extend, Goal::Horizontal(x.into()));
        self.autoscroll = true;
        self.selection_changed(cx);
    }

    /// PageUp/PageDown: scrolls by a viewport less one line and moves the cursor as far, so it
    /// keeps its place on screen.
    fn page(&mut self, down: bool, extend: bool, window: &Window, cx: &mut Context<Self>) {
        let vp = self.viewport();
        let line_height = self.layouts.style().line_height;
        let page = (vp.height - line_height).max(line_height);
        let distance = if down { page } else { -page };
        let (mut lines, scroll) = self.lines(window);
        *scroll = scroll.scrolled_by(distance, &mut lines, &vp);
        self.move_vertically(distance, extend, window, cx);
    }

    /// Home/End: to the start or end of the cursor's visual row.
    fn move_in_row(&mut self, to_end: bool, extend: bool, window: &Window, cx: &mut Context<Self>) {
        let head = self.editor.selection().head;
        let (mut lines, _) = self.lines(window);
        let point = lines.buffer.offset_to_point(head);
        let layout = lines.layout(point.line);
        let row = layout.row_of(point.column);
        let column = if to_end {
            layout.row_end(row)
        } else {
            layout.row_start(row)
        };
        let offset = lines.offset(point.line, column);
        self.editor.move_to(offset, extend);
        self.autoscroll = true;
        self.selection_changed(cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.editor.select_all();
        self.selection_changed(cx);
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Editor")
            .track_focus(&self.focus_handle)
            .size_full()
            .on_action(
                cx.listener(|this, _: &MoveLeft, _, cx| this.motion(Motion::Left, false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &MoveRight, _, cx| this.motion(Motion::Right, false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &MoveWordLeft, _, cx| {
                    this.motion(Motion::WordLeft, false, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &MoveWordRight, _, cx| {
                this.motion(Motion::WordRight, false, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveToDocumentStart, _, cx| {
                this.motion(Motion::DocumentStart, false, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveToDocumentEnd, _, cx| {
                this.motion(Motion::DocumentEnd, false, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SelectLeft, _, cx| this.motion(Motion::Left, true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectRight, _, cx| this.motion(Motion::Right, true, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| {
                this.motion(Motion::WordLeft, true, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| {
                this.motion(Motion::WordRight, true, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToDocumentStart, _, cx| {
                this.motion(Motion::DocumentStart, true, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToDocumentEnd, _, cx| {
                this.motion(Motion::DocumentEnd, true, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveUp, window, cx| {
                this.move_vertically(px(-1.), false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveDown, window, cx| {
                this.move_vertically(px(1.), false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectUp, window, cx| {
                this.move_vertically(px(-1.), true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectDown, window, cx| {
                this.move_vertically(px(1.), true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PageUp, window, cx| this.page(false, false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageDown, window, cx| this.page(true, false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectPageUp, window, cx| {
                    this.page(false, true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SelectPageDown, window, cx| {
                    this.page(true, true, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &MoveToRowStart, window, cx| {
                this.move_in_row(false, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveToRowEnd, window, cx| {
                this.move_in_row(true, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToRowStart, window, cx| {
                this.move_in_row(false, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectToRowEnd, window, cx| {
                this.move_in_row(true, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .child(EditorElement::new(cx.entity()))
    }
}

/// The document as laid-out lines: what scroll math, hit testing and vertical motion walk over.
struct Lines<'a> {
    buffer: &'a Buffer,
    cache: &'a mut LayoutCache,
    text_system: &'a WindowTextSystem,
}

impl Lines<'_> {
    fn layout(&mut self, line: usize) -> Arc<LineLayout> {
        self.cache.line(line, self.buffer, self.text_system)
    }

    /// The buffer offset of `column` in `line`, on a grapheme boundary.
    fn offset(&self, line: usize, column: usize) -> ByteOffset {
        let start = self.buffer.line_start(line);
        self.buffer
            .clip_offset(ByteOffset(start.0 + column), Bias::Left)
    }
}

impl LineHeights for Lines<'_> {
    fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    fn height(&mut self, line: usize) -> Pixels {
        self.layout(line).height()
    }
}

/// Where vertical movement by `distance` lands: walks visual rows from the cursor until at least
/// `distance` has been covered, then picks the column nearest the goal x. Past the first or last
/// row it goes to the document start or end, like the engine's logical-line motion.
fn vertical_target(
    lines: &mut Lines,
    head: ByteOffset,
    goal: Goal,
    distance: Pixels,
) -> (ByteOffset, Pixels) {
    let buffer = lines.buffer;
    let point = buffer.offset_to_point(head);
    let mut line = point.line;
    let mut layout = lines.layout(line);
    let mut row = layout.row_of(point.column);
    let x = match goal {
        Goal::Horizontal(x) => px(x),
        _ => layout.x_for(point.column),
    };
    let mut covered = px(0.);
    while covered < distance.abs() {
        if distance > px(0.) {
            if row + 1 < layout.row_count() {
                row += 1;
            } else if line + 1 < buffer.line_count() {
                line += 1;
                layout = lines.layout(line);
                row = 0;
            } else {
                return (buffer.end(), x);
            }
        } else if row > 0 {
            row -= 1;
        } else if line > 0 {
            line -= 1;
            layout = lines.layout(line);
            row = layout.row_count() - 1;
        } else {
            return (ByteOffset(0), x);
        }
        covered += layout.line_height();
    }
    (lines.offset(line, layout.column_at(row, x)), x)
}

fn base_style(cx: &App, wrap_width: Pixels) -> BaseStyle {
    let font_size = typography::BODY_FONT_SIZE;
    BaseStyle {
        font: font(typography::BODY_FONT_FAMILY),
        font_size,
        line_height: font_size * typography::BODY_LINE_HEIGHT,
        color: cx.theme().foreground,
        wrap_width,
    }
}

fn viewport(bounds: Option<Bounds<Pixels>>) -> Viewport {
    Viewport {
        height: bounds.map_or(px(0.), |bounds| bounds.size.height),
        top_padding: TOP_PADDING,
    }
}

/// Left edge and width of the centred text column inside the editor's bounds.
fn text_column(bounds: Bounds<Pixels>) -> (Pixels, Pixels) {
    let width = (bounds.size.width - MIN_SIDE_PADDING * 2.)
        .min(MAX_TEXT_WIDTH)
        .max(px(1.));
    let left = bounds.left() + ((bounds.size.width - width) / 2.).floor();
    (left, width)
}
