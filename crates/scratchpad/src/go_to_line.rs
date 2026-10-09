//! Go to line (Ctrl+G, as in Notepad): a small box over the top of the editor that asks for a
//! line number, pre-filled with the cursor's, and moves the cursor to the start of that line.
//! Enter goes, Escape cancels; either way the caret is back in the note. See ADR 0141.

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, Focusable, KeyBinding, SharedString, Subscription,
    Window, actions, div, prelude::*, px,
};

use crate::editor_view::EditorView;
use crate::find_bar::{input_box, text_button};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::ActiveTheme;

actions!(
    go_to_line,
    [
        /// Ask for a line number and move the cursor to the start of that line.
        GoToLine,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("secondary-g", GoToLine, None)]
}

/// Emitted when the box is done (gone to the line, cancelled, or left) and should go away.
pub struct Dismissed;

pub struct GoToLineBox {
    editor: Entity<EditorView>,
    input: Entity<TextInput>,
    line_count: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Dismissed> for GoToLineBox {}

impl GoToLineBox {
    /// Opens with the cursor's line number selected and focused.
    pub fn new(editor: Entity<EditorView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (line, line_count) = {
            let engine = editor.read(cx).editor();
            let buffer = engine.buffer();
            (buffer.line_of(engine.selection().head), buffer.line_count())
        };
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Line", cx);
            input.set_text(&(line + 1).to_string(), cx);
            input.select_all(cx);
            input
        });
        let focus = input.focus_handle(cx);
        window.focus(&focus);
        let subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            // E.g. a click in the note or Ctrl+F. Another window becoming active leaves it open.
            cx.on_focus_out(&focus, window, |_, _, window, cx| {
                if window.is_window_active() {
                    cx.emit(Dismissed);
                }
            }),
        ];
        Self {
            editor,
            input,
            line_count,
            _subscriptions: subscriptions,
        }
    }

    /// Goes to the line typed, the last one if it is beyond the end. Without a number it waits
    /// for one.
    fn go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text();
        if text.is_empty() {
            return;
        }
        // Only digits get in, so a number too large for usize is the only way parsing fails.
        let number = text.parse::<usize>().unwrap_or(usize::MAX);
        self.editor.update(cx, |editor, cx| {
            editor.go_to_line(number.saturating_sub(1), cx)
        });
        self.dismiss(window, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor.focus_handle(cx));
        cx.emit(Dismissed);
    }

    fn on_input_event(
        &mut self,
        input: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Typed or pasted: keep only the digits.
            TextInputEvent::Changed => {
                let text = input.read(cx).text();
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                if digits != text {
                    input.update(cx, |input, cx| input.set_text(&digits, cx));
                }
            }
            TextInputEvent::Confirmed => self.go(window, cx),
            TextInputEvent::Cancelled => self.dismiss(window, cx),
        }
    }
}

impl Render for GoToLineBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div()
            .debug_selector(|| "go-to-line".into())
            .block_mouse_except_scroll()
            // Wraps onto two rows in an editor too narrow for one.
            .max_w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .p_1()
            .pl_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .shadow_md()
            .child(div().pr_1().child("Go to line"))
            .child(input_box("go-to-line-field", None, &self.input, &theme, window, cx).w(px(96.)))
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(SharedString::from(format!("of {}", self.line_count))),
            )
            .child(
                text_button("go-to-line-go", "Go to", true, &theme)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.go(window, cx))),
            )
            .child(
                text_button("go-to-line-cancel", "Cancel", true, &theme).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.dismiss(window, cx)),
                ),
            )
    }
}
