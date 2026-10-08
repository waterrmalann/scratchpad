use gpui::{Context, Entity, FocusHandle, Focusable, Subscription, Window, div, prelude::*};

use crate::actions::CloseWindow;
use crate::editor_pane::EditorPane;
use crate::sidebar::Sidebar;
use crate::theme::{self, ActiveTheme, typography};
use crate::toast;

/// Root view of the main window: sidebar on the left, editor pane filling the rest (PLAN §42).
pub struct AppWindow {
    focus_handle: FocusHandle,
    sidebar: Entity<Sidebar>,
    editor_pane: Entity<EditorPane>,
    _appearance_subscription: Subscription,
}

impl AppWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance_subscription = cx.observe_window_appearance(window, |_, window, cx| {
            theme::system_appearance_changed(window.appearance(), cx);
        });
        let focus_handle = cx.focus_handle();
        let editor_pane = cx.new(|cx| EditorPane::new(window, cx));
        editor_pane.focus_handle(cx).focus(window);
        Self {
            focus_handle,
            sidebar: cx.new(|_| Sidebar),
            editor_pane,
            _appearance_subscription: appearance_subscription,
        }
    }

    pub fn editor_pane(&self) -> &Entity<EditorPane> {
        &self.editor_pane
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }
}

impl Focusable for AppWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .key_context("AppWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_window))
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(typography::UI_FONT_FAMILY)
            .text_size(typography::UI_FONT_SIZE)
            .child(self.sidebar.clone())
            .child(self.editor_pane.clone())
            .children(toast::render(cx))
    }
}
