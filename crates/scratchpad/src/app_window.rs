use gpui::{Context, Entity, FocusHandle, Focusable, Subscription, Window, div, prelude::*};

use crate::actions::{CloseWindow, NewNote, SearchNotes};
use crate::editor_pane::EditorPane;
use crate::notes::{Notes, NotesLocation};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::theme::{self, ActiveTheme, typography};
use crate::toast;

/// Root view of the main window: sidebar on the left, editor pane filling the rest (PLAN §42).
pub struct AppWindow {
    focus_handle: FocusHandle,
    notes: Entity<Notes>,
    sidebar: Entity<Sidebar>,
    editor_pane: Entity<EditorPane>,
    _subscriptions: [Subscription; 2],
}

impl AppWindow {
    pub fn new(notes: NotesLocation, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance_subscription = cx.observe_window_appearance(window, |_, window, cx| {
            theme::system_appearance_changed(window.appearance(), cx);
        });
        let focus_handle = cx.focus_handle();
        let editor_pane = cx.new(|cx| EditorPane::new(window, cx));
        editor_pane.focus_handle(cx).focus(window);
        let notes = cx.new(|cx| Notes::new(notes, None, cx));
        let sidebar = cx.new(|cx| Sidebar::new(notes.clone(), window, cx));
        let sidebar_subscription =
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::FocusEditor => window.focus(&this.editor_pane.focus_handle(cx)),
            });
        Self {
            focus_handle,
            sidebar,
            notes,
            editor_pane,
            _subscriptions: [appearance_subscription, sidebar_subscription],
        }
    }

    pub fn editor_pane(&self) -> &Entity<EditorPane> {
        &self.editor_pane
    }

    /// The note list and the open note. The editor integration subscribes to its events.
    pub fn notes(&self) -> &Entity<Notes> {
        &self.notes
    }

    pub fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    // Window-wide commands live on the root so they work wherever focus is.
    fn new_note(&mut self, _: &NewNote, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.new_note(window, cx));
    }

    fn search_notes(&mut self, _: &SearchNotes, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
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
            .on_action(cx.listener(Self::new_note))
            .on_action(cx.listener(Self::search_notes))
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
