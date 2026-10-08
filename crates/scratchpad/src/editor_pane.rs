use gpui::{App, Context, Entity, FocusHandle, Focusable, Window, div, prelude::*};

use crate::editor_view::EditorView;

/// Hosts the editor for the open note (PLAN §42). Until notes are wired in, it holds an empty
/// document.
pub struct EditorPane {
    editor: Entity<EditorView>,
}

impl EditorPane {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            editor: cx.new(|cx| EditorView::new("", window, cx)),
        }
    }

    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }
}

impl Focusable for EditorPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for EditorPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "editor-pane".into())
            .flex_1()
            .h_full()
            .child(self.editor.clone())
    }
}
