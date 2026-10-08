use gpui::{Context, Entity, FocusHandle, Focusable, Subscription, Window, div, prelude::*, px};

use crate::actions::{CloseWindow, NewNote, OpenSettings, SaveNote, SearchNotes};
use crate::app::Storage;
use crate::editor_pane::EditorPane;
use crate::editor_view::EditorView;
use crate::notes::{Notes, Selection};
use crate::session::Session;
use crate::settings_panel::{SettingsPanel, SettingsPanelEvent};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::theme::{self, ActiveTheme, typography};
use crate::{settings, toast};

/// Root view of the main window: sidebar on the left, editor pane filling the rest (PLAN §42).
pub struct AppWindow {
    focus_handle: FocusHandle,
    notes: Entity<Notes>,
    sidebar: Entity<Sidebar>,
    editor_pane: Entity<EditorPane>,
    session: Entity<Session>,
    settings: Option<OpenSettingsPanel>,
    _subscriptions: Vec<Subscription>,
}

/// The settings panel while it is shown.
struct OpenSettingsPanel {
    panel: Entity<SettingsPanel>,
    _subscriptions: [Subscription; 2],
}

impl AppWindow {
    pub fn new(storage: Storage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let config = settings::get(cx);
        let reopen = config.last_opened_note.clone();
        let sidebar_width = config.sidebar_width;
        let notes = cx.new(|cx| Notes::new(storage.notes.clone(), reopen, cx));
        let editor = cx.new(|cx| EditorView::new("", window, cx));
        let session =
            cx.new(|cx| Session::new(notes.clone(), editor.clone(), &storage, window, cx));
        let editor_pane = cx.new(|cx| EditorPane::new(editor.clone(), session.clone(), cx));
        window.focus(&editor.focus_handle(cx));
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(notes.clone(), window, cx);
            if let Some(width) = sidebar_width {
                sidebar.set_width(px(width), cx);
            }
            sidebar
        });

        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::system_appearance_changed(window.appearance(), cx);
            }),
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::FocusEditor => window.focus(&this.editor_pane.focus_handle(cx)),
            }),
            cx.observe(&notes, |_, notes, cx| {
                if let Selection::Note(path) = notes.read(cx).selection() {
                    let path = path.clone();
                    settings::update(cx, |config| config.last_opened_note = Some(path));
                }
            }),
            cx.observe(&sidebar, |_, sidebar, cx| {
                let width = f32::from(sidebar.read(cx).width());
                settings::update(cx, |config| config.sidebar_width = Some(width));
            }),
            cx.observe_window_bounds(window, |_, window, cx| {
                let bounds = window_bounds(window);
                settings::update(cx, |config| config.window = Some(bounds));
            }),
            // Quitting (Ctrl+Q, or the last window closing) runs this before the app exits.
            cx.on_app_quit(|this, cx| {
                this.save_all(cx);
                async {}
            }),
        ];
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            this.update(cx, |this, cx| this.save_all(cx)).ok();
            true
        });

        Self {
            focus_handle,
            sidebar,
            notes,
            editor_pane,
            session,
            settings: None,
            _subscriptions: subscriptions,
        }
    }

    /// The settings panel, while it is open.
    pub fn settings_panel(&self) -> Option<&Entity<SettingsPanel>> {
        self.settings.as_ref().map(|settings| &settings.panel)
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

    /// The open note's loading, saving and recovery.
    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    /// Calls `callback` once, when the first note opened has been loaded into the editor.
    pub fn on_first_note_shown(&self, cx: &mut Context<Self>, callback: impl FnOnce() + 'static) {
        self.session
            .update(cx, |session, _| session.on_first_load(callback));
    }

    /// Writes the open note, pending saves and the settings before the window goes away.
    fn save_all(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.flush_sync(cx));
        settings::save_now(cx);
    }

    // Window-wide commands live on the root so they work wherever focus is.
    fn new_note(&mut self, _: &NewNote, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.new_note(window, cx));
    }

    fn save_note(&mut self, _: &SaveNote, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| session.flush(cx));
    }

    fn search_notes(&mut self, _: &SearchNotes, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        window.remove_window();
    }

    // --- Settings ---

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let panel = cx.new(|cx| SettingsPanel::new(window, cx));
        let panel_focus = panel.focus_handle(cx);
        let subscriptions = [
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                SettingsPanelEvent::Close => this.close_settings(window, cx),
            }),
            // E.g. Ctrl+N or Ctrl+P while it is open: the panel goes and focus stays there.
            cx.on_focus_out(&panel_focus, window, |this, _, _, cx| {
                this.settings = None;
                cx.notify();
            }),
        ];
        self.settings = Some(OpenSettingsPanel {
            panel,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Closes the settings panel, if open, and puts the caret back in the note.
    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            window.focus(&self.editor_pane.focus_handle(cx));
            cx.notify();
        }
    }
}

/// The window's restorable bounds (the normal size and position even while maximized).
fn window_bounds(window: &Window) -> scratchpad_core::WindowBounds {
    let (bounds, maximized) = match window.window_bounds() {
        gpui::WindowBounds::Windowed(bounds) => (bounds, false),
        gpui::WindowBounds::Maximized(bounds) | gpui::WindowBounds::Fullscreen(bounds) => {
            (bounds, true)
        }
    };
    scratchpad_core::WindowBounds {
        x: bounds.origin.x.into(),
        y: bounds.origin.y.into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
        maximized,
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
            .on_action(cx.listener(Self::save_note))
            .on_action(cx.listener(Self::search_notes))
            .on_action(cx.listener(Self::open_settings))
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
            .children(self.settings_panel().cloned())
            .children(toast::render(cx))
    }
}
