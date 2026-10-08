//! A minimal single-line text field for the note search and inline renaming. See ADR 0052.
//!
//! Supports typing (including IME composition through [`EntityInputHandler`]), caret movement,
//! selection with Shift and the mouse, Ctrl+A, Backspace/Delete, Ctrl+Backspace, and the
//! clipboard. Enter and Escape are reported as events; the owner decides what they mean.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::ActiveTheme;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        SelectToHome,
        SelectToEnd,
        Paste,
        Copy,
        Cut,
        Confirm,
        Cancel,
    ]
);

const CONTEXT: &str = "TextInput";
const CARET_WIDTH: Pixels = px(1.5);

pub fn key_bindings() -> Vec<KeyBinding> {
    let context = Some(CONTEXT);
    vec![
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("shift-backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("secondary-a", SelectAll, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
        // A single line has nowhere to go up or down, so the caret goes to its ends (as on
        // macOS). This also keeps the arrows from reaching list navigation behind the field.
        KeyBinding::new("up", Home, context),
        KeyBinding::new("down", End, context),
        KeyBinding::new("shift-home", SelectToHome, context),
        KeyBinding::new("shift-end", SelectToEnd, context),
        KeyBinding::new("secondary-v", Paste, context),
        KeyBinding::new("secondary-c", Copy, context),
        KeyBinding::new("secondary-x", Cut, context),
        KeyBinding::new("enter", Confirm, context),
        KeyBinding::new("escape", Cancel, context),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// The text changed (typing, deleting, pasting or [`TextInput::set_text`]).
    Changed,
    /// Enter was pressed.
    Confirmed,
    /// Escape was pressed.
    Cancelled,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    text: String,
    placeholder: SharedString,
    /// Byte offsets into `text`, on grapheme boundaries.
    selected: Range<usize>,
    /// The caret is at `selected.start` rather than `selected.end`.
    reversed: bool,
    /// The IME composition, if any.
    marked: Option<Range<usize>>,
    /// Horizontal scroll that keeps the caret visible when the text is wider than the field.
    scroll_x: Pixels,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            text: String::new(),
            placeholder: placeholder.into(),
            selected: 0..0,
            reversed: false,
            marked: None,
            scroll_x: px(0.),
            last_layout: None,
            last_bounds: None,
            selecting: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the text and puts the caret at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.text == text {
            return;
        }
        self.text = text.to_owned();
        self.selected = text.len()..text.len();
        self.reversed = false;
        self.marked = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected = 0..self.text.len();
        self.reversed = false;
        cx.notify();
    }

    fn caret(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    /// Moves the caret end of the selection to `offset`, keeping the other end.
    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text[..offset]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(ix, _)| ix)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text[offset..]
            .graphemes(true)
            .next()
            .map_or(self.text.len(), |grapheme| offset + grapheme.len())
    }

    /// Start of the word before `offset`, skipping whitespace first (Ctrl+Backspace).
    fn previous_word_start(&self, offset: usize) -> usize {
        let mut start = offset;
        for (ix, segment) in self.text[..offset].split_word_bound_indices().rev() {
            start = ix;
            if !segment.trim().is_empty() {
                break;
            }
        }
        start
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.previous_boundary(self.caret()), cx);
        } else {
            self.move_to(self.selected.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.move_to(self.next_boundary(self.caret()), cx);
        } else {
            self.move_to(self.selected.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.caret()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.caret()), cx);
    }

    fn select_all_action(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }

    fn select_to_home(&mut self, _: &SelectToHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.previous_boundary(self.caret()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.caret()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.is_empty() {
            self.select_to(self.previous_word_start(self.caret()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let single_line = text.replace("\r\n", " ").replace(['\n', '\r'], " ");
            self.replace_text_in_range(None, &single_line, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            let selected = self.text[self.selected.clone()].to_owned();
            cx.write_to_clipboard(ClipboardItem::new_string(selected));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.copy(&Copy, window, cx);
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Confirmed);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Cancelled);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.selecting = true;
        let offset = self.offset_for_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.offset_for_position(event.position), cx);
        }
    }

    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return 0;
        };
        if self.text.is_empty() {
            return 0;
        }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
    }

    fn offset_to_utf16(&self, utf8: usize) -> usize {
        self.text[..utf8].chars().map(char::len_utf16).sum()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_range_to_utf8(&self.text, range)
    }
}

/// The byte range in `text` of a UTF-16 range from the IME, which is not trusted to be in
/// order, in bounds or on character boundaries.
fn utf16_range_to_utf8(text: &str, range: &Range<usize>) -> Range<usize> {
    let (start, end) = (
        utf16_to_utf8(text, range.start),
        utf16_to_utf8(text, range.end),
    );
    start.min(end)..start.max(end)
}

/// The byte offset in `text` of the UTF-16 offset `utf16`, rounded up to a character boundary
/// and clamped to the end.
fn utf16_to_utf8(text: &str, utf16: usize) -> usize {
    let mut count = 0;
    for (ix, ch) in text.char_indices() {
        if count >= utf16 {
            return ix;
        }
        count += ch.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(range.clone(), new_text);
        let caret = range.start + new_text.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| self.range_from_utf16(&range))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.text.replace_range(range.clone(), new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        // The IME gives the new selection relative to the inserted text.
        let caret = range.start + new_text.len();
        self.selected = new_selected_range_utf16.map_or(caret..caret, |selected| {
            let selected = utf16_range_to_utf8(new_text, &selected);
            range.start + selected.start..range.start + selected.end
        });
        self.reversed = false;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let left = bounds.left() - self.scroll_x;
        Some(Bounds::from_corners(
            point(left + line.x_for_index(range.start), bounds.top()),
            point(left + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        if self.text.is_empty() {
            // The last layout is the placeholder's.
            return Some(0);
        }
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        let index = line.index_for_x(point.x - bounds.left() + self.scroll_x)?;
        Some(self.offset_to_utf16(index))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .w_full()
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_to_home))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement { input: cx.entity() })
    }
}

/// Paints the text, selection and caret, and registers the IME input handler.
struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: ShapedLine,
    scroll_x: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
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
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let theme = cx.theme();
        let style = window.text_style();
        let (text, color) = if input.text.is_empty() {
            (input.placeholder.clone(), theme.muted)
        } else {
            (SharedString::from(input.text.clone()), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match &input.marked {
            Some(marked) => [
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.len(),
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);

        // Scroll just enough to keep the caret inside the field.
        let caret_x = line.x_for_index(input.caret());
        let visible = (bounds.size.width - CARET_WIDTH).max(px(0.));
        let scroll_x = input
            .scroll_x
            .max(caret_x - visible)
            .min(caret_x)
            .min((line.width - visible).max(px(0.)))
            .max(px(0.));
        let left = bounds.left() - scroll_x;

        let selection = (!input.selected.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(left + line.x_for_index(input.selected.start), bounds.top()),
                    point(left + line.x_for_index(input.selected.end), bounds.bottom()),
                ),
                theme.selection,
            )
        });
        let caret = input.selected.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(left + caret_x, bounds.top()),
                    size(CARET_WIDTH, bounds.size.height),
                ),
                theme.accent,
            )
        });
        PrepaintState {
            line,
            scroll_x,
            selection,
            caret,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let line = prepaint.line.clone();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            let origin = point(bounds.left() - prepaint.scroll_x, bounds.top());
            // Painting only fails when the glyph atlas is exhausted; skip the frame then.
            line.paint(origin, window.line_height(), window, cx).ok();
            if focus_handle.is_focused(window)
                && let Some(caret) = prepaint.caret.take()
            {
                window.paint_quad(caret);
            }
        });
        let scroll_x = prepaint.scroll_x;
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_x = scroll_x;
        });
    }
}
