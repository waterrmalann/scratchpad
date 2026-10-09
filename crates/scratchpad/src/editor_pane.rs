use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Pixels, Subscription,
    Window, div, prelude::*, px,
};

use crate::editor_view::{Direction, EditorView};
use crate::find_bar::{CloseFind, FindBar};
use crate::go_to_line::{Dismissed, GoToLineBox};
use crate::session::{Choice, Notice, Session};
use crate::settings;
use crate::sidebar::sidebar_toggle_button;
use crate::status_bar::StatusBar;
use crate::theme::ActiveTheme;

/// The narrowest the note gets beside the docked sidebar, which narrows instead (ADR 0135).
pub const MIN_NOTE_WIDTH: Pixels = px(460.);

/// Hosts the editor for the open note (PLAN §42), with a bar above it for decisions about the
/// note: a conflict with another program, a deleted file, unreadable characters, or text
/// recovered after a crash. The find bar floats over the editor's top right corner, Go to line
/// over its top centre and, while the sidebar is hidden, the button that shows it over the top
/// left one. The status bar runs along the bottom unless the user hid it.
pub struct EditorPane {
    editor: Entity<EditorView>,
    session: Entity<Session>,
    find_bar: Entity<FindBar>,
    status_bar: Entity<StatusBar>,
    sidebar_button: bool,
    /// Go to line, while it asks for a line number.
    go_to_line: Option<(Entity<GoToLineBox>, Subscription)>,
    _session_changed: Subscription,
}

impl EditorPane {
    pub fn new(
        editor: Entity<EditorView>,
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _session_changed: cx.observe(&session, |_, _, cx| cx.notify()),
            find_bar: cx.new(|cx| FindBar::new(editor.clone(), window, cx)),
            status_bar: cx.new(|cx| StatusBar::new(editor.clone(), session.clone(), cx)),
            go_to_line: None,
            editor,
            session,
            sidebar_button: false,
        }
    }

    /// Shows the button that brings back the hidden sidebar.
    pub fn set_sidebar_button(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.sidebar_button != visible {
            self.sidebar_button = visible;
            cx.notify();
        }
    }

    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }

    pub fn find_bar(&self) -> &Entity<FindBar> {
        &self.find_bar
    }

    pub fn status_bar(&self) -> &Entity<StatusBar> {
        &self.status_bar
    }

    /// Ctrl+F: opens the find bar, or moves focus back to it.
    pub fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_bar.update(cx, |bar, cx| bar.open(window, cx));
    }

    /// Ctrl+H: opens the find bar with its replace row, in the replacement.
    pub fn replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_bar
            .update(cx, |bar, cx| bar.open_replace(window, cx));
    }

    /// The Go to line box, while it is shown.
    pub fn go_to_line_box(&self) -> Option<&Entity<GoToLineBox>> {
        self.go_to_line.as_ref().map(|(go_to_line, _)| go_to_line)
    }

    /// Ctrl+G: asks for a line number, starting afresh from the cursor's line if already asking.
    pub fn go_to_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let go_to_line = cx.new(|cx| GoToLineBox::new(self.editor.clone(), window, cx));
        let dismissed = cx.subscribe(&go_to_line, |this, _, _: &Dismissed, cx| {
            this.go_to_line = None;
            cx.notify();
        });
        self.go_to_line = Some((go_to_line, dismissed));
        cx.notify();
    }

    /// F3 / Shift+F3: selects the next or previous match, reopening the find bar if needed.
    pub fn select_match(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_bar
            .update(cx, |bar, cx| bar.select_match(direction, window, cx));
    }

    /// Escape in the note closes the find bar; with the bar closed it means nothing here.
    fn close_find(&mut self, _: &CloseFind, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_bar.read(cx).is_open() {
            self.find_bar.update(cx, |bar, cx| bar.close(window, cx));
        } else {
            cx.propagate();
        }
    }

    fn render_notice(&self, notice: &Notice, cx: &App) -> AnyElement {
        // A file from outside the notes folder is not called a note (ADR 0145).
        let file = self.session.read(cx).is_external();
        let noun = if file { "file" } else { "note" };
        let (selector, message, choices): (_, String, &[(Choice, &str)]) = match notice {
            Notice::NotUtf8 => (
                "notice:not-utf8",
                format!(
                    "This {noun} contains characters that are not valid UTF-8 (shown as \
                     \u{FFFD}). Editing it replaces them when it is saved."
                ),
                &[(Choice::EditAnyway, "Edit Anyway")],
            ),
            Notice::ChangedOnDisk => (
                "notice:changed",
                format!("This {noun} was changed by another program while you were editing it."),
                &[
                    (Choice::KeepMine, "Keep My Version"),
                    (Choice::LoadDisk, "Load Their Version"),
                ],
            ),
            Notice::DeletedOnDisk => (
                "notice:deleted",
                format!("This {noun} was deleted by another program."),
                if file {
                    &[
                        (Choice::KeepDeleted, "Keep File"),
                        (Choice::CloseDeleted, "Close File"),
                    ]
                } else {
                    &[
                        (Choice::KeepDeleted, "Keep Note"),
                        (Choice::CloseDeleted, "Close Note"),
                    ]
                },
            ),
            Notice::Recovered { title, new_note } => (
                "notice:recovered",
                if *new_note {
                    format!("Scratchpad recovered an unsaved new note, \u{201C}{title}\u{201D}.")
                } else {
                    format!("Scratchpad recovered unsaved changes to \u{201C}{title}\u{201D}.")
                },
                &[(Choice::Restore, "Restore"), (Choice::Discard, "Discard")],
            ),
        };
        let theme = cx.theme();
        let buttons = choices.iter().map(|&(choice, label)| {
            div()
                .id(label)
                .debug_selector(move || format!("choice:{label}"))
                .flex_none()
                .px_3()
                .py(px(3.))
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_color(theme.foreground)
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|style| style.border_color(theme.accent))
                .on_click({
                    let session = self.session.clone();
                    move |_, window, cx| {
                        session.update(cx, |session, cx| session.choose(choice, window, cx))
                    }
                })
                .child(label)
        });
        div()
            .debug_selector(move || selector.into())
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .text_color(theme.foreground)
            .child(div().flex_1().min_w_0().child(message))
            .children(buttons)
            .into_any_element()
    }
}

impl Focusable for EditorPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for EditorPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notices = self.session.read(cx).notices();
        let find_bar = self
            .find_bar
            .read(cx)
            .is_open()
            .then(|| self.find_bar.clone());
        let go_to_line = self.go_to_line_box().cloned();
        div()
            .debug_selector(|| "editor-pane".into())
            .on_action(cx.listener(Self::close_find))
            .flex_1()
            .h_full()
            .min_w(MIN_NOTE_WIDTH)
            .flex()
            .flex_col()
            .children(notices.iter().map(|notice| self.render_notice(notice, cx)))
            .child(
                div()
                    .debug_selector(|| "note-area".into())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(self.editor.clone())
                    // In the editor's top padding, clear of the text.
                    .when(self.sidebar_button, |editor| {
                        editor.child(
                            sidebar_toggle_button("show-sidebar", cx.theme())
                                .absolute()
                                .top(px(4.))
                                .left(px(4.)),
                        )
                    })
                    // Clear of the scrollbar on the right edge, and no wider than the editor.
                    .children(find_bar.map(|bar| {
                        div()
                            .absolute()
                            .top(px(8.))
                            .left(px(8.))
                            .right(px(20.))
                            .flex()
                            .justify_end()
                            .child(bar)
                    }))
                    .children(go_to_line.map(|go_to_line| {
                        div()
                            .absolute()
                            .top(px(8.))
                            .left_0()
                            .right_0()
                            .flex()
                            .justify_center()
                            .px_2()
                            .child(go_to_line)
                    })),
            )
            .when(!settings::get(cx).status_bar_hidden, |pane| {
                pane.child(self.status_bar.clone())
            })
    }
}
