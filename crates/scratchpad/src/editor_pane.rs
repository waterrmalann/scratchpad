use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Subscription, Window,
    div, prelude::*, px,
};

use crate::editor_view::{Direction, EditorView};
use crate::find_bar::{CloseFind, FindBar, FindNext, FindPrevious};
use crate::session::{Choice, Notice, Session};
use crate::theme::ActiveTheme;

/// Hosts the editor for the open note (PLAN §42), with a bar above it for decisions about the
/// note: a conflict with another program, a deleted file, unreadable characters, or text
/// recovered after a crash. The find bar floats over the editor's top right corner.
pub struct EditorPane {
    editor: Entity<EditorView>,
    session: Entity<Session>,
    find_bar: Entity<FindBar>,
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
            editor,
            session,
        }
    }

    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
    }

    pub fn find_bar(&self) -> &Entity<FindBar> {
        &self.find_bar
    }

    /// Ctrl+F: opens the find bar, or moves focus back to it.
    pub fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_bar.update(cx, |bar, cx| bar.open(window, cx));
    }

    fn select_match(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
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
        let (selector, message, choices): (_, String, &[(Choice, &str)]) = match notice {
            Notice::NotUtf8 => (
                "notice:not-utf8",
                "This note contains characters that are not valid UTF-8 (shown as \u{FFFD}). \
                 Editing it replaces them when it is saved."
                    .into(),
                &[(Choice::EditAnyway, "Edit Anyway")],
            ),
            Notice::ChangedOnDisk => (
                "notice:changed",
                "This note was changed by another program while you were editing it.".into(),
                &[
                    (Choice::KeepMine, "Keep My Version"),
                    (Choice::LoadDisk, "Load Their Version"),
                ],
            ),
            Notice::DeletedOnDisk => (
                "notice:deleted",
                "This note was deleted by another program.".into(),
                &[
                    (Choice::KeepDeleted, "Keep Note"),
                    (Choice::CloseDeleted, "Close Note"),
                ],
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
        div()
            .debug_selector(|| "editor-pane".into())
            .on_action(cx.listener(|this, _: &FindNext, window, cx| {
                this.select_match(Direction::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FindPrevious, window, cx| {
                this.select_match(Direction::Previous, window, cx)
            }))
            .on_action(cx.listener(Self::close_find))
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .children(notices.iter().map(|notice| self.render_notice(notice, cx)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(self.editor.clone())
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
                    })),
            )
    }
}
