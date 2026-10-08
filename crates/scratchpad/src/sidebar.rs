use gpui::{Context, Window, div, prelude::*, px};

use crate::theme::ActiveTheme;

/// Fixed for now; becomes user-resizable (and persisted, PLAN §32) later.
pub const SIDEBAR_WIDTH: gpui::Pixels = px(260.);

/// Note navigation: search field, "New Note" button and the note list (PLAN §8, §42).
///
/// An empty panel until notes are wired in.
pub struct Sidebar;

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .debug_selector(|| "sidebar".into())
            .flex_none()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
    }
}
