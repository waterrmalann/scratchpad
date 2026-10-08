use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, Subscription, Window,
    div, prelude::*, px,
};

use crate::editor_view::EditorView;
use crate::session::{Choice, Notice, Session};
use crate::theme::ActiveTheme;

/// Hosts the editor for the open note (PLAN §42), with a bar above it for decisions about the
/// note: a conflict with another program, a deleted file, unreadable characters, or text
/// recovered after a crash.
pub struct EditorPane {
    editor: Entity<EditorView>,
    session: Entity<Session>,
    _session_changed: Subscription,
}

impl EditorPane {
    pub fn new(
        editor: Entity<EditorView>,
        session: Entity<Session>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _session_changed: cx.observe(&session, |_, _, cx| cx.notify()),
            editor,
            session,
        }
    }

    pub fn editor(&self) -> &Entity<EditorView> {
        &self.editor
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
        div()
            .debug_selector(|| "editor-pane".into())
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .children(notices.iter().map(|notice| self.render_notice(notice, cx)))
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
    }
}
