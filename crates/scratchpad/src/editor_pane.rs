use gpui::{Context, Window, div, prelude::*};

use crate::theme::ActiveTheme;

/// Hosts the editor for the open note (PLAN §42). Shows an empty state until the editor
/// view exists.
pub struct EditorPane;

impl Render for EditorPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "editor-pane".into())
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted)
            .child("No note selected")
    }
}
