use std::path::PathBuf;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseDownEvent, Pixels,
    Subscription, Task, Window, div, prelude::*, px,
};
use scratchpad_core::NoteStore;

use crate::actions::view::{ResetZoom, ToggleWordWrap, ZoomIn, ZoomOut};
use crate::actions::{
    CloseWindow, NewNote, OpenSettings, SaveNote, SearchNotes, ToggleSidebar, ToggleStatusBar,
};
use crate::app::Storage;
use crate::editor_pane::EditorPane;
use crate::editor_view::EditorView;
use crate::find_bar::FindInNote;
use crate::notes::{Notes, NotesLocation, Selection, folder_name};
use crate::session::Session;
use crate::settings_panel::{SettingsPanel, SettingsPanelEvent};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::theme::{self, ActiveTheme, typography};
use crate::{settings, toast};

/// Narrower windows do not dock the sidebar, so the note keeps a comfortable width: at this
/// width a sidebar of the default 260 px leaves 460 px for it (ADR 0135).
pub const AUTO_COLLAPSE_WIDTH: Pixels = px(720.);

/// How the sidebar is shown (ADR 0135).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    /// Beside the note.
    Docked,
    /// Not shown: the user collapsed it, or the window is too narrow to dock it.
    Hidden,
    /// Floating over the note until the user is done with it.
    Overlay,
}

/// Root view of the main window: sidebar on the left, editor pane filling the rest (PLAN §42).
pub struct AppWindow {
    focus_handle: FocusHandle,
    notes: Entity<Notes>,
    sidebar: Entity<Sidebar>,
    /// Tracked by the sidebar's container while it is shown over the note, to notice focus
    /// leaving it.
    sidebar_overlay_focus: FocusHandle,
    /// The sidebar was asked for while it is not docked.
    sidebar_overlay: bool,
    /// The window is narrower than [`AUTO_COLLAPSE_WIDTH`].
    narrow: bool,
    editor_pane: Entity<EditorPane>,
    session: Entity<Session>,
    notes_dir_overridden: bool,
    settings: Option<OpenSettingsPanel>,
    /// Opening a newly picked notes folder; a newer pick replaces (cancels) it.
    folder_change: Option<Task<()>>,
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
        let sidebar_collapsed = config.sidebar_collapsed;
        let (zoom, word_wrap) = (config.zoom_percent, config.word_wrap);
        let notes = cx.new(|cx| Notes::new(storage.notes.clone(), reopen, cx));
        let editor = cx.new(|cx| {
            let mut editor = EditorView::new("", window, cx);
            editor.set_zoom_percent(zoom.unwrap_or(100), cx);
            editor.set_soft_wrap(word_wrap.unwrap_or(true), cx);
            editor
        });
        let session =
            cx.new(|cx| Session::new(notes.clone(), editor.clone(), &storage, window, cx));
        let editor_pane = cx.new(|cx| EditorPane::new(editor.clone(), session.clone(), window, cx));
        window.focus(&editor.focus_handle(cx));
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(notes.clone(), window, cx);
            if let Some(width) = sidebar_width {
                sidebar.set_width(px(width), cx);
            }
            sidebar
        });
        let sidebar_overlay_focus = cx.focus_handle();
        let narrow = is_narrow(window);
        let sidebar_hidden = narrow || sidebar_collapsed;
        editor_pane.update(cx, |pane, cx| pane.set_sidebar_button(sidebar_hidden, cx));

        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::system_appearance_changed(window.appearance(), cx);
            }),
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::FocusEditor => window.focus(&this.editor_pane.focus_handle(cx)),
                SidebarEvent::NoteChosen => {
                    if this.sidebar_mode(cx) == SidebarMode::Overlay {
                        this.hide_sidebar_overlay(window, cx);
                    }
                }
            }),
            // Clicking the note, Escape in the search field, Ctrl+N...: the user is done with
            // the sidebar. Another window becoming active (e.g. the delete confirmation) is not.
            cx.on_focus_out(&sidebar_overlay_focus, window, |this, _, window, cx| {
                if window.is_window_active() {
                    this.hide_sidebar_overlay(window, cx);
                }
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
            // Zoom and word wrap belong to the editor; checked on each of its updates, written
            // only when they change.
            cx.observe(&editor, |_, editor, cx| {
                let editor = editor.read(cx);
                let (zoom, wrap) = (editor.zoom_percent(), editor.soft_wrap());
                let config = settings::get(cx);
                if config.zoom_percent.unwrap_or(100) != zoom
                    || config.word_wrap.unwrap_or(true) != wrap
                {
                    settings::update(cx, |config| {
                        config.zoom_percent = Some(zoom);
                        config.word_wrap = Some(wrap);
                    });
                }
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                let bounds = window_bounds(window);
                settings::update(cx, |config| config.window = Some(bounds));
                let narrow = is_narrow(window);
                if narrow != this.narrow {
                    this.narrow = narrow;
                    this.sidebar_changed(window, cx);
                }
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
            sidebar_overlay_focus,
            sidebar_overlay: false,
            narrow,
            notes,
            editor_pane,
            session,
            notes_dir_overridden: storage.notes_dir_overridden,
            settings: None,
            folder_change: None,
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
        if self.sidebar_mode(cx) == SidebarMode::Hidden {
            self.sidebar_overlay = true;
            self.sidebar_changed(window, cx);
        }
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
    }

    fn find_in_note(&mut self, _: &FindInNote, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane
            .update(cx, |pane, cx| pane.find(window, cx));
    }

    /// Zoom and word wrap act on the note wherever focus is.
    fn update_editor(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut EditorView, &mut Context<EditorView>),
    ) {
        let editor = self.editor_pane.read(cx).editor().clone();
        editor.update(cx, update);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        window.remove_window();
    }

    // --- Sidebar (ADR 0135) ---

    /// How the sidebar is shown right now.
    pub fn sidebar_mode(&self, cx: &App) -> SidebarMode {
        if !self.narrow && !settings::get(cx).sidebar_collapsed {
            SidebarMode::Docked
        } else if self.sidebar_overlay {
            SidebarMode::Overlay
        } else {
            SidebarMode::Hidden
        }
    }

    /// Ctrl+\ and the sidebar buttons. Collapsing or expanding the docked sidebar is remembered;
    /// in a narrow window the sidebar is only shown over the note for a while.
    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        match self.sidebar_mode(cx) {
            SidebarMode::Docked => settings::update(cx, |config| config.sidebar_collapsed = true),
            SidebarMode::Overlay => self.sidebar_overlay = false,
            SidebarMode::Hidden if self.narrow => {
                self.sidebar_overlay = true;
                self.sidebar.read(cx).focus_list(window);
            }
            SidebarMode::Hidden => settings::update(cx, |config| config.sidebar_collapsed = false),
        }
        self.sidebar_changed(window, cx);
    }

    fn hide_sidebar_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_overlay {
            self.sidebar_overlay = false;
            self.sidebar_changed(window, cx);
        }
    }

    /// Brings the rest of the window in line with a change of [`sidebar_mode`](Self::sidebar_mode).
    fn sidebar_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.sidebar_mode(cx);
        if mode == SidebarMode::Docked {
            // E.g. the window was widened while the sidebar was shown over the note.
            self.sidebar_overlay = false;
        }
        let hidden = mode == SidebarMode::Hidden;
        if hidden && self.sidebar.read(cx).contains_focus(window, cx) {
            window.focus(&self.editor_pane.focus_handle(cx));
        }
        self.editor_pane
            .update(cx, |pane, cx| pane.set_sidebar_button(hidden, cx));
        cx.notify();
    }

    fn render_sidebar_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .size_full()
            // Clicking anywhere else only closes it, as with any flyout.
            .child(
                div()
                    .id("sidebar-overlay-backdrop")
                    .debug_selector(|| "sidebar-overlay-backdrop".into())
                    .absolute()
                    .size_full()
                    .occlude()
                    .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.hide_sidebar_overlay(window, cx)
                    })),
            )
            .child(
                div()
                    .track_focus(&self.sidebar_overlay_focus)
                    .absolute()
                    .h_full()
                    .occlude()
                    .shadow_lg()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.hide_sidebar_overlay(window, cx);
                        }
                    }))
                    .child(self.sidebar.clone()),
            )
    }

    /// Shows or hides the status bar under the note (ADR 0137).
    fn toggle_status_bar(&mut self, _: &ToggleStatusBar, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |config| {
            config.status_bar_hidden = !config.status_bar_hidden;
        });
        cx.notify();
    }

    // --- Settings ---

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let panel = cx.new(|cx| {
            SettingsPanel::new(self.notes.clone(), self.notes_dir_overridden, window, cx)
        });
        let panel_focus = panel.focus_handle(cx);
        let subscriptions = [
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                SettingsPanelEvent::Close => this.close_settings(window, cx),
                SettingsPanelEvent::NotesFolderPicked(dir) => {
                    this.change_notes_folder(dir.clone(), window, cx)
                }
            }),
            // E.g. Ctrl+N or Ctrl+P while it is open: the panel goes and focus stays there.
            // Another window becoming active (such as the folder dialog, whose answer the
            // panel waits for) leaves it open.
            cx.on_focus_out(&panel_focus, window, |this, _, window, cx| {
                if window.is_window_active() {
                    this.settings = None;
                    cx.notify();
                }
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

    /// Switches to the notes in `dir` if notes can be written there; otherwise shows why and
    /// stays on the current folder. The folder is opened off the UI thread: it may be slow,
    /// e.g. on a network drive.
    fn change_notes_folder(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.notes.read(cx).location();
        if dir == current.dir {
            // Also the last word on a slow switch to another folder that is still running.
            self.folder_change = None;
            self.close_settings(window, cx);
            return;
        }
        let location = NotesLocation {
            dir,
            ..current.clone()
        };
        let opening = cx.background_spawn({
            let location = location.clone();
            async move { location.open_writable() }
        });
        self.folder_change = Some(cx.spawn_in(window, async move |this, cx| {
            let opened = opening.await;
            this.update_in(cx, |this, window, cx| match opened {
                Ok(store) => this.switch_notes_folder(location, store, window, cx),
                Err(error) => toast::show_file_error(&folder_name(&location.dir), &error, cx),
            })
            .ok();
        }));
    }

    fn switch_notes_folder(
        &mut self,
        location: NotesLocation,
        store: NoteStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = location.dir.clone();
        tracing::info!(dir = %dir.display(), "changing the notes folder");
        // The open note, new notes and pending renames go to the old folder first.
        self.session
            .update(cx, |session, cx| session.change_folder(dir.clone(), cx));
        self.notes
            .update(cx, |notes, cx| notes.change_folder(location, store, cx));
        settings::update(cx, |config| config.notes_dir = Some(dir));
        self.close_settings(window, cx);
    }
}

fn is_narrow(window: &Window) -> bool {
    window.viewport_size().width < AUTO_COLLAPSE_WIDTH
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
        let mode = self.sidebar_mode(cx);
        let theme = cx.theme();
        div()
            .key_context("AppWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::new_note))
            .on_action(cx.listener(Self::save_note))
            .on_action(cx.listener(Self::search_notes))
            .on_action(cx.listener(Self::find_in_note))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_status_bar))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.update_editor(cx, |editor, cx| editor.zoom_by(1, cx))
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.update_editor(cx, |editor, cx| editor.zoom_by(-1, cx))
            }))
            .on_action(cx.listener(|this, _: &ResetZoom, _, cx| {
                this.update_editor(cx, |editor, cx| editor.set_zoom_percent(100, cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleWordWrap, _, cx| {
                this.update_editor(cx, |editor, cx| {
                    editor.set_soft_wrap(!editor.soft_wrap(), cx)
                })
            }))
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(typography::UI_FONT_FAMILY)
            .text_size(typography::UI_FONT_SIZE)
            .when(mode == SidebarMode::Docked, |root| {
                root.child(self.sidebar.clone())
            })
            .child(self.editor_pane.clone())
            .when(mode == SidebarMode::Overlay, |root| {
                root.child(self.render_sidebar_overlay(cx))
            })
            .children(self.settings_panel().cloned())
            .children(toast::render(cx))
    }
}
