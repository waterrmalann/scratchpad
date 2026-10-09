//! The editor view: the typing surface on top of the editing engine (PLAN §10–27, §36, §45–50).
//!
//! [`EditorView`] owns the engine's [`Editor`], which is the source of truth for text, selection and
//! history, and the document's [`MarkdownState`], which styles it. The view only adds what is about
//! presentation: the scroll position, shaped line layouts, caret blinking, mouse drags and the
//! matches of the find bar. [`element::EditorElement`] lays out and paints the visible lines.

mod element;
mod find;
mod geometry;
mod layout_cache;
mod line_layout;
mod scroll;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    SharedString, Subscription, Task, UTF16Selection, Window, WindowTextSystem, div, font,
    prelude::*, px,
};
use scratchpad_editor::markdown::MarkdownState;
use scratchpad_editor::{Bias, Buffer, ByteOffset, Editor, Goal, Motion, Selection, Utf16Offset};

use crate::actions::editor::*;
use crate::theme::{ActiveTheme, typography};
use element::{EditorElement, ScrollbarLayout};
pub use find::{BACKGROUND_SEARCH_BYTES, Direction, FindStatus, SEARCH_DEBOUNCE};
use layout_cache::LayoutCache;
use line_layout::{BaseStyle, LineKey, LineLayout};
use scroll::{LineHeights, ScrollAnchor, Viewport};

/// Caret on/off period (PLAN §24).
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// Space left of the text, in multiples of the font size: room for a heading's `#`s to hang in
/// (ADR 0070, 0130). `### ` at the H3 size is about 2.56 em of body text in Segoe UI semibold;
/// deeper headings move their text by what does not fit. It scales with the zoom, as the markers
/// do.
const LEFT_PADDING_EMS: f32 = 2.8;
/// Space right of the text, clear of the scrollbar.
const RIGHT_PADDING: Pixels = px(24.);
const TOP_PADDING: Pixels = px(32.);
/// The text size range, in percent of the normal size, and the step of Ctrl+= and Ctrl+-.
const MIN_ZOOM: u16 = 50;
const MAX_ZOOM: u16 = 400;
const ZOOM_STEP: u16 = 10;
/// Ctrl+wheel distance per zoom step: one notch of a mouse wheel at Windows' default of three
/// lines. A precision touchpad sends fractions of a notch, which add up.
const ZOOM_WHEEL_LINES: f32 = 3.;
/// Rows kept between the cursor and the viewport edge when the view scrolls to the cursor.
const AUTOSCROLL_MARGIN_ROWS: f32 = 2.;
/// How often the view scrolls while a selection is dragged past its top or bottom edge.
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(16);
/// Inserted by the Tab key: spaces look the same in every font and Markdown reads them as
/// indentation (ADR 0031).
const TAB: &str = "    ";

/// Emitted by [`EditorView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorEvent {
    /// The text changed through user input (typing, IME, paste, undo, ...). Not emitted by
    /// [`EditorView::set_text`].
    Changed,
}

/// A text editor for one document.
pub struct EditorView {
    editor: Editor,
    /// The Markdown structure of `editor`'s buffer; replaced with the document.
    markdown: MarkdownState,
    /// Shows every Markdown marker instead of hiding them away from the cursor (live preview).
    source_mode: bool,
    /// Text size in percent of the normal size.
    zoom: u16,
    /// Ctrl+wheel distance, in lines, not yet turned into a zoom step.
    zoom_wheel: f32,
    focus_handle: FocusHandle,
    layouts: LayoutCache,
    /// Buffer version the layouts and scroll anchor were last updated to.
    synced_version: u64,
    /// Buffer version and selection the cached layouts' keys were last checked against.
    styled_for: Option<(u64, Option<Selection>)>,
    /// Looked up once: enumerating fonts takes about a millisecond.
    mono_family: SharedString,
    scroll: ScrollAnchor,
    /// Set by keyboard input and search; the next layout scrolls the cursor into view.
    autoscroll: Option<Autoscroll>,
    /// The top of the cursor's row below the viewport's top in the last frame, if it was on screen.
    cursor_row_top: Option<Pixels>,
    /// Element bounds from the last layout; `None` until the first frame.
    bounds: Option<Bounds<Pixels>>,
    visible_lines: Range<usize>,
    scrollbar: Option<ScrollbarLayout>,
    cursor_visible: bool,
    /// Running while the editor is focused in an active window.
    blink_task: Option<Task<()>>,
    drag: Option<Drag>,
    /// Where the last click put the cursor, and the buffer version then. A double or triple click
    /// selects around it rather than around what is under the pointer now: the first click may
    /// have revealed markers and moved the text.
    click_offset: Option<(u64, ByteOffset)>,
    /// When the input being processed arrived, for the input-to-paint trace (PLAN §37).
    input_at: Option<Instant>,
    /// Edits are ignored, e.g. while a note loads; the cursor still moves.
    read_only: bool,
    /// The find bar's query and its matches while the bar is open.
    find: Option<find::Find>,
    /// The match highlights painted in the last frame.
    #[cfg(feature = "test-support")]
    match_highlights: Vec<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

/// How the next layout brings the cursor into view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Autoscroll {
    /// Scroll as little as possible: the cursor moved by a step (typing, arrow keys).
    Cursor,
    /// Put it in the middle if it is not on screen: the cursor jumped (to a search match).
    Center,
    /// Put its row's top this far below the viewport's top: the text changed size, and the
    /// cursor stays where it was on screen.
    Keep(Pixels),
}

enum Drag {
    /// Selecting with the mouse, by the unit the drag started with.
    Select {
        granularity: Granularity,
        /// What the initial click selected.
        initial: Range<ByteOffset>,
        position: Point<Pixels>,
        /// Scrolls while the pointer is above or below the editor.
        autoscroll: Option<Task<()>>,
    },
    /// Dragging the scrollbar thumb, grabbed `grab` below its top.
    Scrollbar { grab: Pixels },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    Line,
}

impl EventEmitter<EditorEvent> for EditorView {}

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
        let mono_family = SharedString::from(typography::mono_font_family(cx));
        // Replaced by the first layout, which knows the width.
        let base_style = base_style(cx, typography::BODY_FONT_SIZE, px(0.), &mono_family);
        let (editor, markdown) = open_document(text);
        Self {
            editor,
            markdown,
            source_mode: false,
            zoom: 100,
            zoom_wheel: 0.,
            focus_handle,
            layouts: LayoutCache::new(base_style),
            synced_version: 0,
            styled_for: None,
            mono_family,
            scroll: ScrollAnchor::top(&viewport(None)),
            autoscroll: None,
            cursor_row_top: None,
            bounds: None,
            visible_lines: 0..0,
            scrollbar: None,
            cursor_visible: true,
            blink_task: None,
            drag: None,
            click_offset: None,
            input_at: None,
            read_only: false,
            find: None,
            #[cfg(feature = "test-support")]
            match_highlights: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Replaces the document (e.g. when another note is opened): fresh history, cursor and scroll
    /// position at the start.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        (self.editor, self.markdown) = open_document(text);
        self.layouts.clear();
        self.synced_version = self.editor.buffer().version();
        self.scroll = ScrollAnchor::top(&viewport(self.bounds));
        self.drag = None;
        self.click_offset = None;
        self.find_text_changed(true, cx);
        cx.notify();
    }

    /// Replaces the document with a newer version of it (e.g. edited by another program), keeping
    /// the cursor on the same line and column and the scroll position as far as the new text
    /// allows. History starts afresh, as with [`set_text`](Self::set_text).
    pub fn reload_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let cursor = self
            .editor
            .buffer()
            .offset_to_point(self.editor.selection().head);
        let scroll = self.scroll;
        self.set_text(text, cx);
        let offset = self.editor.buffer().point_to_offset(cursor);
        self.editor.move_to(offset, false);
        // Clamped to the new document when it is next laid out.
        self.scroll = scroll;
    }

    /// Like [`reload_text`](Self::reload_text), but as one undoable edit: for the user's choice
    /// between two versions of a note, so the one replaced stays one undo away. Not reported as
    /// [`EditorEvent::Changed`] (the caller decides whether the note now needs saving), and done
    /// even while read-only.
    pub fn replace_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let cursor = self
            .editor
            .buffer()
            .offset_to_point(self.editor.selection().head);
        let end = ByteOffset(self.editor.buffer().len());
        self.editor.replace_range(ByteOffset(0)..end, text);
        let offset = self.editor.buffer().point_to_offset(cursor);
        self.editor.move_to(offset, false);
        self.find_text_changed(false, cx);
        cx.notify();
    }

    /// While set, edits from the keyboard, IME and clipboard are ignored.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
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

    /// Number of times a line has been shaped, for tests of what input re-shapes.
    #[doc(hidden)]
    pub fn shaped_line_count(&self) -> u64 {
        self.layouts.shaped()
    }

    /// The search match highlights painted in the last frame, in window coordinates.
    #[cfg(feature = "test-support")]
    pub fn match_highlights(&self) -> &[Bounds<Pixels>] {
        &self.match_highlights
    }

    /// Whether every Markdown marker is shown (source mode) rather than only those near the
    /// selection (live preview).
    pub fn source_mode(&self) -> bool {
        self.source_mode
    }

    /// The note's text size in percent of the normal size.
    pub fn zoom_percent(&self) -> u16 {
        self.zoom
    }

    /// Sets the text size, within 50% to 400%. The cursor's row stays where it is on screen;
    /// with the cursor off screen, the text at the top stays there.
    pub fn set_zoom_percent(&mut self, percent: u16, cx: &mut Context<Self>) {
        let percent = percent.clamp(MIN_ZOOM, MAX_ZOOM);
        if percent == self.zoom {
            return;
        }
        let scale = f32::from(percent) / f32::from(self.zoom);
        self.zoom = percent;
        self.keep_place(scale);
        cx.notify();
    }

    /// Zooms in (positive `steps`) or out by steps of 10%, to a multiple of 10%.
    pub fn zoom_by(&mut self, steps: i32, cx: &mut Context<Self>) {
        // A level off the steps (edited by hand) first goes to the nearest step that way.
        let from = if steps > 0 {
            self.zoom / ZOOM_STEP
        } else {
            self.zoom.div_ceil(ZOOM_STEP)
        };
        let percent = (i32::from(from) + steps) * i32::from(ZOOM_STEP);
        let percent = percent.clamp(MIN_ZOOM.into(), MAX_ZOOM.into());
        self.set_zoom_percent(percent as u16, cx);
    }

    /// Before the line heights change by about `scale`: the next layout puts the cursor's row back
    /// where it was on screen, or, with the cursor off screen, keeps the top line in place.
    fn keep_place(&mut self, scale: f32) {
        match self.cursor_row_top {
            Some(top) => self.autoscroll = Some(Autoscroll::Keep(top)),
            // Not the top padding, which does not scale.
            None if self.scroll.offset > px(0.) => self.scroll.offset *= scale,
            None => {}
        }
    }

    fn font_size(&self) -> Pixels {
        typography::BODY_FONT_SIZE * (f32::from(self.zoom) / 100.)
    }

    /// The left edge of the text column in window coordinates, and the column's width.
    fn text_column(&self, bounds: Bounds<Pixels>) -> (Pixels, Pixels) {
        text_column(bounds, self.font_size())
    }

    // --- Engine plumbing ---

    /// Runs an edit, notifying observers if the text changed and scrolling to the cursor.
    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Editor)) {
        self.edit_markdown(cx, |editor, _| edit(editor));
    }

    /// [`EditorView::edit`] for commands that read the document's Markdown structure.
    fn edit_markdown(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Editor, &mut MarkdownState),
    ) {
        if self.read_only {
            return;
        }
        let version = self.editor.buffer().version();
        edit(&mut self.editor, &mut self.markdown);
        if self.editor.buffer().version() != version {
            cx.emit(EditorEvent::Changed);
            self.find_text_changed(false, cx);
        }
        self.autoscroll = Some(Autoscroll::Cursor);
        self.selection_changed(cx);
    }

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
        self.autoscroll = Some(Autoscroll::Cursor);
        self.selection_changed(cx);
    }

    /// Ctrl+Left/Right. Markers live preview hides are no word to stop at: the cursor moves on
    /// over them as over text that is not there (ADR 0126).
    fn word_motion(
        &mut self,
        motion: Motion,
        extend: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let head = self.editor.selection().head;
        let (mut lines, _) = self.lines(window);
        let line = lines.buffer.line_of(head);
        let line_start = lines.buffer.line_start(line).0;
        let layout = lines.layout(line);
        let mut from = head;
        loop {
            self.editor.move_cursor(motion, extend);
            let to = self.editor.selection().head;
            let buffer = self.editor.buffer();
            if to == from || buffer.line_of(to) != line {
                break;
            }
            let crossed = from.min(to)..from.max(to);
            let shows_text = buffer
                .text_for_range(crossed.clone())
                .char_indices()
                .any(|(i, c)| {
                    !c.is_whitespace() && !layout.hides(crossed.start.0 + i - line_start)
                });
            if shows_text {
                break;
            }
            from = to;
        }
        self.autoscroll = Some(Autoscroll::Cursor);
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

    /// The document's laid-out lines and the scroll anchor, in sync with the buffer and selection.
    fn lines<'a>(&'a mut self, window: &'a Window) -> (Lines<'a>, &'a mut ScrollAnchor) {
        self.sync_layouts();
        // Live preview shows markers depending on the selection; source mode shows them all.
        let selection = (!self.source_mode).then(|| self.editor.selection());
        let styled_for = Some((self.editor.buffer().version(), selection));
        if self.styled_for != styled_for {
            self.styled_for = styled_for;
            self.layouts.restyle();
        }
        let lines = Lines {
            buffer: self.editor.buffer(),
            markdown: &mut self.markdown,
            selection,
            cache: &mut self.layouts,
            text_system: window.text_system(),
        };
        (lines, &mut self.scroll)
    }

    fn viewport(&self) -> Viewport {
        viewport(self.bounds)
    }

    /// The line under a window position, and the position relative to the line's top-left corner.
    /// Above or below the text, the first or last line.
    fn line_at(&mut self, position: Point<Pixels>, window: &Window) -> Option<LineHit> {
        let bounds = self.bounds?;
        let (left, _) = self.text_column(bounds);
        let (mut lines, scroll) = self.lines(window);
        let (line, y) = scroll.line_at(position.y - bounds.top(), &mut lines);
        Some(LineHit {
            line,
            layout: lines.layout(line),
            position: gpui::point(position.x - left, y),
        })
    }

    /// The buffer offset nearest to a window position.
    fn offset_at(&mut self, position: Point<Pixels>, window: &Window) -> ByteOffset {
        let Some(hit) = self.line_at(position, window) else {
            return self.editor.selection().head;
        };
        let layout = &hit.layout;
        let column = layout.column_at(layout.row_at(hit.position.y), hit.position.x);
        self.offset(hit.line, column)
    }

    /// The buffer offset of `column` in `line`, on a grapheme boundary.
    fn offset(&self, line: usize, column: usize) -> ByteOffset {
        let buffer = self.editor.buffer();
        let start = buffer.line_start(line);
        buffer.clip_offset(ByteOffset(start.0 + column), Bias::Left)
    }

    /// The destination of the link drawn at a window position.
    fn link_at(&mut self, position: Point<Pixels>, window: &Window) -> Option<String> {
        let hit = self.line_at(position, window)?;
        let layout = &hit.layout;
        if hit.position.y < px(0.) || hit.position.y >= layout.height() {
            return None;
        }
        let column = layout.column_under(layout.row_at(hit.position.y), hit.position.x)?;
        let offset = self.offset(hit.line, column);
        self.markdown.link_at(self.editor.buffer(), offset)
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
        let (offset, x) = vertical_target(&mut lines, selection, extend, goal, distance);
        self.editor
            .move_to_with_goal(offset, extend, Goal::Horizontal(x.into()));
        self.autoscroll = Some(Autoscroll::Cursor);
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

    /// Home/End: to the start or end of the cursor's visual row. Home on a list item, quote or
    /// heading goes to its text first, after the markers, and from there to the line start
    /// (ADR 0126).
    fn move_in_row(&mut self, to_end: bool, extend: bool, window: &Window, cx: &mut Context<Self>) {
        let selection = self.editor.selection();
        let (mut lines, _) = self.lines(window);
        let offset = if to_end {
            row_end_target(&mut lines, selection, extend)
        } else {
            row_start_target(&mut lines, selection.head)
        };
        self.editor.move_to(offset, extend);
        self.autoscroll = Some(Autoscroll::Cursor);
        self.selection_changed(cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.editor.select_all();
        self.selection_changed(cx);
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.editor.copy() {
            cx.write_to_clipboard(clipboard_item(text));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        let mut cut = None;
        self.edit(cx, |editor| cut = editor.cut());
        if let Some(text) = cut {
            cx.write_to_clipboard(clipboard_item(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |editor| editor.paste(&text));
        }
    }

    fn toggle_source_mode(&mut self, cx: &mut Context<Self>) {
        self.source_mode = !self.source_mode;
        cx.notify();
    }

    // --- Mouse ---

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let previous_click = self.click_offset.take();
        if event.click_count <= 1 && !event.modifiers.shift {
            // A read-only note's task boxes are text like any other: the click places the cursor.
            if !self.read_only
                && let Some(task_box) = self.task_box_at(event.position, window)
            {
                self.toggle_task(task_box, cx);
                return;
            }
            if event.modifiers.secondary()
                && let Some(url) = self.link_at(event.position, window)
            {
                open_link(&url, cx);
                return;
            }
        }
        let version = self.editor.buffer().version();
        let offset = match previous_click {
            Some((clicked_in, offset)) if event.click_count > 1 && clicked_in == version => offset,
            _ => self.offset_at(event.position, window),
        };
        self.click_offset = Some((version, offset));
        let granularity = match event.click_count {
            0 | 1 => Granularity::Character,
            2 => Granularity::Word,
            _ => Granularity::Line,
        };
        match granularity {
            Granularity::Character => self.editor.move_to(offset, event.modifiers.shift),
            Granularity::Word => self.editor.select_word_at(offset),
            Granularity::Line => self.editor.select_line_at(offset),
        }
        self.drag = Some(Drag::Select {
            granularity,
            initial: self.editor.selection().range(),
            position: event.position,
            autoscroll: None,
        });
        self.selection_changed(cx);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The button may have been released outside the window, where no mouse-up reaches us.
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        match &mut self.drag {
            Some(Drag::Select {
                position,
                autoscroll,
                ..
            }) => {
                *position = event.position;
                let outside = self
                    .bounds
                    .is_some_and(|b| event.position.y < b.top() || event.position.y > b.bottom());
                if !outside {
                    *autoscroll = None;
                } else if autoscroll.is_none() {
                    *autoscroll = Some(cx.spawn_in(window, async move |this, cx| {
                        loop {
                            cx.background_executor().timer(DRAG_SCROLL_INTERVAL).await;
                            let scrolled =
                                this.update_in(cx, |this, window, cx| this.drag_scroll(window, cx));
                            if !matches!(scrolled, Ok(true)) {
                                break;
                            }
                        }
                    }));
                }
                self.extend_drag_selection(window, cx);
            }
            Some(Drag::Scrollbar { grab }) => {
                let grab = *grab;
                self.drag_scrollbar(event.position.y - grab, window, cx);
            }
            None => {}
        }
    }

    fn mouse_up(&mut self) {
        self.drag = None;
    }

    /// The offset of the `[` of the task box drawn at a window position.
    fn task_box_at(&mut self, position: Point<Pixels>, window: &Window) -> Option<ByteOffset> {
        let hit = self.line_at(position, window)?;
        let column = hit.layout.task_box_at(hit.position)?;
        Some(self.offset(hit.line, column))
    }

    /// Checks or unchecks the task whose box starts at `task_box`, as one undo step that leaves the
    /// selection and the scroll position alone.
    fn toggle_task(&mut self, task_box: ByteOffset, cx: &mut Context<Self>) {
        let mark = ByteOffset(task_box.0 + 1)..ByteOffset(task_box.0 + 2);
        let checked = self.editor.buffer().text_for_range(mark.clone()) != " ";
        self.edit(cx, |editor| {
            editor.transact(|editor| {
                let selection = editor.selection();
                editor.replace_range(mark, if checked { " " } else { "x" });
                editor.set_selection(selection);
            })
        });
        self.autoscroll = None;
    }

    /// Extends a mouse selection to the pointer, by the unit the drag started with.
    fn extend_drag_selection(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (
            Some(bounds),
            Some(Drag::Select {
                granularity,
                initial,
                position,
                ..
            }),
        ) = (self.bounds, &self.drag)
        else {
            return;
        };
        let (granularity, initial) = (*granularity, initial.clone());
        // Past the top or bottom edge the selection follows the first or last visible row.
        let y = position.y.clamp(bounds.top(), bounds.bottom() - px(1.));
        let offset = self.offset_at(Point { x: position.x, y }, window);
        let unit = match granularity {
            Granularity::Character => {
                self.editor.move_to(offset, true);
                self.selection_changed(cx);
                return;
            }
            Granularity::Word => self.editor.word_range_at(offset),
            Granularity::Line => self.editor.line_range_at(offset),
        };
        let selection = if unit.start < initial.start {
            Selection::new(initial.end, unit.start)
        } else {
            Selection::new(initial.start, unit.end.max(initial.end))
        };
        self.editor.set_selection(selection);
        self.selection_changed(cx);
    }

    /// One step of scrolling while a selection is dragged outside the editor. Returns false once
    /// the pointer is back inside.
    fn drag_scroll(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let (Some(bounds), Some(Drag::Select { position, .. })) = (self.bounds, &self.drag) else {
            return false;
        };
        let overshoot = if position.y < bounds.top() {
            position.y - bounds.top()
        } else if position.y > bounds.bottom() {
            position.y - bounds.bottom()
        } else {
            return false;
        };
        let vp = self.viewport();
        let (mut lines, scroll) = self.lines(window);
        // Faster the further the pointer is from the edge.
        *scroll = scroll.scrolled_by(overshoot / 2., &mut lines, &vp);
        self.extend_drag_selection(window, cx);
        true
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        if event.modifiers.control {
            self.zoom_wheel(event.delta, cx);
            return;
        }
        self.input_at.get_or_insert_with(Instant::now);
        let delta = event.delta.pixel_delta(self.layouts.style().line_height);
        let vp = self.viewport();
        let (mut lines, scroll) = self.lines(window);
        *scroll = scroll.scrolled_by(-delta.y, &mut lines, &vp);
        if matches!(self.drag, Some(Drag::Select { .. })) {
            self.extend_drag_selection(window, cx);
        }
        cx.notify();
    }

    /// Ctrl+wheel: a step of zoom per notch of a mouse wheel, forward to zoom in. Touchpads send
    /// fractions of a notch, which add up; at most one step is taken per event, so a wheel set to
    /// scroll more lines than usual still zooms a step per notch.
    fn zoom_wheel(&mut self, delta: ScrollDelta, cx: &mut Context<Self>) {
        let lines = match delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => {
                pixels.y / (typography::BODY_FONT_SIZE * typography::BODY_LINE_HEIGHT)
            }
        };
        // Turning the other way starts afresh rather than first undoing what had added up.
        if lines == 0. || lines.signum() != self.zoom_wheel.signum() {
            self.zoom_wheel = 0.;
        }
        self.zoom_wheel += lines;
        if self.zoom_wheel.abs() >= ZOOM_WHEEL_LINES {
            let step = self.zoom_wheel.signum() as i32;
            self.zoom_wheel %= ZOOM_WHEEL_LINES;
            self.zoom_by(step, cx);
        }
    }

    fn scrollbar_mouse_down(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scrollbar) = self.scrollbar else {
            return;
        };
        // Grabbing the thumb keeps it under the pointer; clicking the track centres it there.
        let grab = if scrollbar.thumb.contains(&position) {
            position.y - scrollbar.thumb.top()
        } else {
            scrollbar.thumb.size.height / 2.
        };
        self.drag = Some(Drag::Scrollbar { grab });
        self.drag_scrollbar(position.y - grab, window, cx);
    }

    /// Scrolls so the scrollbar thumb's top is at `thumb_top`.
    fn drag_scrollbar(&mut self, thumb_top: Pixels, window: &Window, cx: &mut Context<Self>) {
        let Some(scrollbar) = self.scrollbar else {
            return;
        };
        let position = scrollbar.position_for_thumb_top(thumb_top);
        let vp = self.viewport();
        let (mut lines, scroll) = self.lines(window);
        *scroll = if position <= 0. {
            ScrollAnchor::top(&vp)
        } else {
            let line = (position as usize).min(lines.line_count() - 1);
            let offset = lines.height(line) * position.fract();
            ScrollAnchor { line, offset }.clamped(&mut lines, &vp)
        };
        cx.notify();
    }

    // --- IME (UTF-16 offsets at this boundary) ---

    /// Converts a range from the platform, which is not trusted to be ordered or in bounds.
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<ByteOffset> {
        let buffer = self.editor.buffer();
        let (start, end) = (range.start.min(range.end), range.start.max(range.end));
        buffer.utf16_to_offset(Utf16Offset(start))..buffer.utf16_to_offset(Utf16Offset(end))
    }

    fn range_to_utf16(&self, range: &Range<ByteOffset>) -> Range<usize> {
        let buffer = self.editor.buffer();
        buffer.offset_to_utf16(range.start).0..buffer.offset_to_utf16(range.end).0
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
            .on_action(cx.listener(|this, _: &MoveWordLeft, window, cx| {
                this.word_motion(Motion::WordLeft, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveWordRight, window, cx| {
                this.word_motion(Motion::WordRight, false, window, cx)
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
            .on_action(cx.listener(|this, _: &SelectWordLeft, window, cx| {
                this.word_motion(Motion::WordLeft, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectWordRight, window, cx| {
                this.word_motion(Motion::WordRight, true, window, cx)
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
            .on_action(
                cx.listener(|this, _: &Backspace, _, cx| this.edit(cx, Editor::backspace_typed)),
            )
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.edit(cx, Editor::delete_forward)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _, cx| {
                this.edit(cx, Editor::delete_word_backward)
            }))
            .on_action(cx.listener(|this, _: &DeleteWordRight, _, cx| {
                this.edit(cx, Editor::delete_word_forward)
            }))
            .on_action(cx.listener(|this, _: &Newline, _, cx| {
                this.edit_markdown(cx, Editor::insert_markdown_newline)
            }))
            .on_action(
                cx.listener(|this, _: &PlainNewline, _, cx| this.edit(cx, Editor::insert_newline)),
            )
            .on_action(cx.listener(|this, _: &Tab, _, cx| {
                this.edit_markdown(cx, |editor, markdown| {
                    if !editor.indent_list_items(markdown) {
                        editor.insert_text(TAB);
                    }
                })
            }))
            .on_action(cx.listener(|this, _: &Outdent, _, cx| {
                this.edit_markdown(cx, |editor, markdown| {
                    editor.outdent_list_items(markdown);
                })
            }))
            .on_action(
                cx.listener(|this, _: &DuplicateLines, _, cx| {
                    this.edit(cx, Editor::duplicate_lines)
                }),
            )
            .on_action(
                cx.listener(|this, _: &MoveLinesUp, _, cx| this.edit(cx, Editor::move_lines_up)),
            )
            .on_action(
                cx.listener(|this, _: &MoveLinesDown, _, cx| {
                    this.edit(cx, Editor::move_lines_down)
                }),
            )
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.edit(cx, |e| _ = e.undo())))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.edit(cx, |e| _ = e.redo())))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Cut, _, cx| this.cut(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &ToggleBold, _, cx| {
                this.edit_markdown(cx, Editor::toggle_bold)
            }))
            .on_action(cx.listener(|this, _: &ToggleItalic, _, cx| {
                this.edit_markdown(cx, Editor::toggle_italic)
            }))
            .on_action(cx.listener(|this, _: &ToggleStrikethrough, _, cx| {
                this.edit_markdown(cx, Editor::toggle_strikethrough)
            }))
            .on_action(cx.listener(|this, _: &ToggleInlineCode, _, cx| {
                this.edit_markdown(cx, Editor::toggle_inline_code)
            }))
            .on_action(
                cx.listener(|this, _: &InsertLink, _, cx| this.edit(cx, Editor::insert_link)),
            )
            .on_action(cx.listener(|this, _: &ToggleSourceMode, _, cx| this.toggle_source_mode(cx)))
            .child(EditorElement::new(cx.entity()))
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.editor.buffer().text_for_range(range).into_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let selection = self.editor.selection();
        Some(UTF16Selection {
            range: self.range_to_utf16(&selection.range()),
            reversed: selection.head < selection.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.editor
            .marked_range()
            .map(|range| self.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.editor.unmark();
        cx.notify();
    }

    /// Typed text, a dead-key result, or a committed IME composition.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let marked = self.editor.marked_range();
        // Japanese IMEs report "no composition" this way; there is nothing to replace.
        if text.is_empty() && range_utf16.is_none() && marked.is_none() {
            return;
        }
        let range = range_utf16.map(|range| self.range_from_utf16(&range));
        self.edit(cx, |editor| match range {
            Some(range) if marked.as_ref() != Some(&range) => {
                editor.unmark();
                editor.replace_range(range, text);
            }
            // Keystrokes pair brackets; IME commits (with marked text) are inserted as they are.
            _ => editor.insert_typed(text),
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16.map(|range| self.range_from_utf16(&range));
        let selected = new_selected_range_utf16.map(|selected| {
            utf16_to_byte(new_text, selected.start)..utf16_to_byte(new_text, selected.end)
        });
        self.edit(cx, |editor| {
            editor.replace_and_mark(range, new_text, selected)
        });
        // Lets the platform move the candidate window along with the composition.
        window.invalidate_character_coordinates();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let bounds = self.bounds?;
        let (left, _) = self.text_column(bounds);
        let vp = self.viewport();
        let (mut lines, scroll) = self.lines(window);
        let buffer = lines.buffer;
        let start = buffer.offset_to_point(range.start);
        let top = scroll.line_top(start.line, &mut lines, vp.height)?;
        let layout = lines.layout(start.line);
        // One rectangle can only describe the range's first row; the platform uses it to place the
        // candidate window, so that is the part that matters.
        let line_start = buffer.line_start(start.line);
        let end_column = range.end.min(buffer.line_end(start.line)).0 - line_start.0;
        let (row, start_x, end_x) = layout
            .spans(start.column..end_column, false, px(0.))
            .first()
            .copied()
            .unwrap_or_else(|| {
                let x = layout.x_for(start.column);
                (layout.row_of(start.column), x, x)
            });
        let row_top = bounds.top() + top + layout.row_top(row);
        Some(Bounds::from_corners(
            gpui::point(left + start_x, row_top),
            gpui::point(left + end_x, row_top + layout.line_height()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at(point, window);
        Some(self.editor.buffer().offset_to_utf16(offset).0)
    }
}

/// The document as laid-out lines: what scroll math, hit testing and vertical motion walk over.
struct Lines<'a> {
    buffer: &'a Buffer,
    markdown: &'a mut MarkdownState,
    /// The selection markers are revealed around; `None` in source mode.
    selection: Option<Selection>,
    cache: &'a mut LayoutCache,
    text_system: &'a WindowTextSystem,
}

impl Lines<'_> {
    fn layout(&mut self, line: usize) -> Arc<LineLayout> {
        let (buffer, markdown, selection) = (self.buffer, &mut *self.markdown, self.selection);
        self.cache.line(line, buffer, self.text_system, || {
            LineKey::new(markdown, buffer, line, selection)
        })
    }

    /// The layout `line` would have with `selection` instead of the current selection: moving the
    /// cursor can reveal or hide markers and so move the line's text.
    fn layout_with(&mut self, line: usize, selection: Selection) -> Arc<LineLayout> {
        if self.selection.is_none() {
            return self.layout(line);
        }
        let key = LineKey::new(self.markdown, self.buffer, line, Some(selection));
        self.cache
            .line_with_key(line, self.buffer, self.text_system, key)
    }

    /// The buffer offset of `column` in `line`, on a grapheme boundary.
    fn offset(&self, line: usize, column: usize) -> ByteOffset {
        let start = self.buffer.line_start(line);
        self.buffer
            .clip_offset(ByteOffset(start.0 + column), Bias::Left)
    }
}

/// How often a cursor target is checked against the layout its line gets with the cursor there.
/// Revealing markers changes that layout once, so two rounds settle in practice.
const SETTLE_ROUNDS: usize = 3;

/// The selection after the head moves to `head`, the anchor staying with `extend`.
fn moved(selection: Selection, head: ByteOffset, extend: bool) -> Selection {
    Selection::new(if extend { selection.anchor } else { head }, head)
}

/// Where Home goes: the start of the cursor's row, but on a list item, quote or heading first the
/// start of its text, after the markers, unless the cursor is already there.
fn row_start_target(lines: &mut Lines, head: ByteOffset) -> ByteOffset {
    let point = lines.buffer.offset_to_point(head);
    let layout = lines.layout(point.line);
    let row = layout.row_of(point.column);
    let text_start = layout.text_start();
    let column = if layout.row_of(text_start) == row && point.column != text_start {
        text_start
    } else {
        layout.row_start(row)
    };
    lines.offset(point.line, column)
}

/// Where End goes: the end of the cursor's row. A cursor there may reveal markers that push the
/// row's last word onto the next row; then End stops at the end of what is left of the row, so
/// that it never takes the cursor to another row (ADR 0125).
fn row_end_target(lines: &mut Lines, selection: Selection, extend: bool) -> ByteOffset {
    let point = lines.buffer.offset_to_point(selection.head);
    let layout = lines.layout(point.line);
    let row_start = layout.row_start(layout.row_of(point.column));
    let mut column = layout.row_end(layout.row_of(point.column));
    for _ in 0..SETTLE_ROUNDS {
        let offset = lines.offset(point.line, column);
        let layout = lines.layout_with(point.line, moved(selection, offset, extend));
        let row = layout.row_of(row_start);
        if layout.row_of(column) == row {
            break;
        }
        column = layout.row_end(row);
    }
    lines.offset(point.line, column)
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
/// `distance` has been covered, then picks the column nearest the goal x in the layout the line
/// gets with the cursor there (ADR 0125). Past the first or last row it goes to the document
/// start or end, like the engine's logical-line motion.
fn vertical_target(
    lines: &mut Lines,
    selection: Selection,
    extend: bool,
    goal: Goal,
    distance: Pixels,
) -> (ByteOffset, Pixels) {
    let buffer = lines.buffer;
    let point = buffer.offset_to_point(selection.head);
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
    // The cursor reveals the markers it lands next to, which moves the text after them. Each
    // candidate is judged in the layout it gives: on the row, and as close to x as possible.
    let mut column = layout.column_at(row, x);
    let mut best: Option<((bool, Pixels), usize)> = None;
    for _ in 0..SETTLE_ROUNDS {
        let offset = lines.offset(line, column);
        let layout = lines.layout_with(line, moved(selection, offset, extend));
        let row = row.min(layout.row_count() - 1);
        let miss = (
            layout.row_of(column) != row,
            (layout.x_for(column) - x).abs(),
        );
        if best.is_none_or(|(best_miss, _)| miss < best_miss) {
            best = Some((miss, column));
        }
        let next = layout.column_at(row, x);
        if next == column {
            break;
        }
        column = next;
    }
    let column = best.map_or(column, |(_, column)| column);
    (lines.offset(line, column), x)
}

/// A new document's engine and Markdown structure. The only place either is created: a
/// [`MarkdownState`] left over from another buffer could pass for current (same version and
/// length) and style the new text with the old document's markers.
fn open_document(text: &str) -> (Editor, MarkdownState) {
    let editor = Editor::from_text(text);
    let markdown = MarkdownState::new(editor.buffer());
    (editor, markdown)
}

/// A line under a point, with the point relative to the line's top-left corner.
struct LineHit {
    line: usize,
    layout: Arc<LineLayout>,
    position: Point<Pixels>,
}

/// The style of body text at `font_size` (the zoomed body size) in a text column `width` wide.
fn base_style(cx: &App, font_size: Pixels, width: Pixels, mono_family: &SharedString) -> BaseStyle {
    BaseStyle {
        font: font(typography::BODY_FONT_FAMILY),
        mono_family: mono_family.clone(),
        font_size,
        line_height: font_size * typography::BODY_LINE_HEIGHT,
        theme: cx.theme().clone(),
        wrap_width: width,
        hang_room: left_padding(font_size),
    }
}

/// Opens a link with Ctrl+click (PLAN §47). Markdown is text, not executable content (PLAN §60):
/// only web and mail links are handed to the system, never files, scripts or other schemes.
fn open_link(url: &str, cx: &App) {
    if is_web_or_mail_link(url) {
        cx.open_url(url);
    } else {
        // Only the scheme: the address itself may be private.
        let scheme = url.split_once(':').map_or("", |(scheme, _)| scheme);
        tracing::info!(
            scheme,
            "not opening a link that is not a web or mail address"
        );
    }
}

fn is_web_or_mail_link(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(scheme, _)| scheme);
    scheme.is_some_and(|scheme| {
        ["http", "https", "mailto"]
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    })
}

fn viewport(bounds: Option<Bounds<Pixels>>) -> Viewport {
    Viewport {
        height: bounds.map_or(px(0.), |bounds| bounds.size.height),
        top_padding: TOP_PADDING,
    }
}

/// Left edge and width of the text column inside the editor's bounds, for body text at
/// `font_size`. Like Notepad, the text uses the whole width of the window.
fn text_column(bounds: Bounds<Pixels>, font_size: Pixels) -> (Pixels, Pixels) {
    let left_padding = left_padding(font_size);
    let width = (bounds.size.width - left_padding - RIGHT_PADDING).max(px(1.));
    (bounds.left() + left_padding, width)
}

/// Space left of the text column, where heading markers hang.
fn left_padding(font_size: Pixels) -> Pixels {
    (font_size * LEFT_PADDING_EMS).round()
}

/// Windows apps expect CRLF line breaks on the clipboard; the engine uses LF (ADR 0003). Pasted
/// text is normalised by the engine.
fn clipboard_item(text: String) -> ClipboardItem {
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    ClipboardItem::new_string(text)
}

/// Converts a UTF-16 offset within `text` to a byte offset, rounding down inside a surrogate pair.
fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units + c.len_utf16() > utf16 {
            return byte;
        }
        units += c.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_are_opened() {
        for url in [
            "https://example.com/a?b=c",
            "HTTP://EXAMPLE.COM",
            "mailto:someone@example.com",
        ] {
            assert!(is_web_or_mail_link(url), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "C:\\Windows\\notepad.exe",
            "ms-settings:privacy",
            "example.com",
            "/relative/path",
            " https://example.com",
            "",
        ] {
            assert!(!is_web_or_mail_link(url), "{url}");
        }
    }

    #[test]
    fn utf16_offsets_within_inserted_text_map_to_bytes() {
        // "é" is 2 bytes / 1 unit, "😀" 4 bytes / 2 units.
        let text = "aé😀b";
        assert_eq!(utf16_to_byte(text, 0), 0);
        assert_eq!(utf16_to_byte(text, 2), 3);
        assert_eq!(utf16_to_byte(text, 3), 3, "inside the surrogate pair");
        assert_eq!(utf16_to_byte(text, 4), 7);
        assert_eq!(utf16_to_byte(text, 99), 8);
    }
}
