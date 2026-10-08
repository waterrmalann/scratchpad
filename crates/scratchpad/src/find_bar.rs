//! Find in the open note (PLAN §29): a small bar over the top right of the editor. See ADR 0100.
//!
//! Ctrl+F opens it with the selected text (if it is short and on one line) as the query. Typing
//! searches after a pause and selects the first match from the cursor; Enter and F3 go to the
//! next match, Shift+Enter and Shift+F3 to the previous one, wrapping around. Escape closes it and
//! puts the caret back in the note with the match still selected. The editor finds and highlights
//! the matches; this view is the query field, the count and the buttons.

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, KeyBinding, SharedString,
    Subscription, Window, actions, div, prelude::*, px,
};
use scratchpad_editor::search::CaseSensitivity;

use crate::editor_view::{Direction, EditorView};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{ActiveTheme, typography};

actions!(
    find,
    [
        /// Open the find bar, or select its query if it is open (PLAN §35).
        FindInNote,
        /// Select the next match of the find bar's query.
        FindNext,
        /// Select the previous match of the find bar's query.
        FindPrevious,
        /// Close the find bar.
        CloseFind,
        /// Switch between ignoring case (the default) and matching it.
        ToggleMatchCase,
    ]
);

const CONTEXT: &str = "FindBar";

/// Longer selections (in bytes) are not taken as the query: nobody searches for a paragraph, and
/// the query field lays out its whole text on every frame.
const MAX_QUERY_FROM_SELECTION: usize = 1000;

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-f", FindInNote, None),
        KeyBinding::new("f3", FindNext, None),
        KeyBinding::new("shift-f3", FindPrevious, None),
        // Enter is the query field's own Confirm.
        KeyBinding::new("shift-enter", FindPrevious, Some(CONTEXT)),
        KeyBinding::new("alt-c", ToggleMatchCase, Some(CONTEXT)),
        KeyBinding::new("escape", CloseFind, Some("Editor")),
    ]
}

// Segoe MDL2 Assets glyphs; plain-text stand-ins elsewhere.
const SEARCH_ICON: &str = if cfg!(windows) { "\u{E721}" } else { "" };
const PREVIOUS_ICON: &str = if cfg!(windows) {
    "\u{E70E}"
} else {
    "\u{2191}"
};
const NEXT_ICON: &str = if cfg!(windows) {
    "\u{E70D}"
} else {
    "\u{2193}"
};
const CLOSE_ICON: &str = if cfg!(windows) {
    "\u{E8BB}"
} else {
    "\u{00D7}"
};

pub struct FindBar {
    editor: Entity<EditorView>,
    query: Entity<TextInput>,
    case: CaseSensitivity,
    open: bool,
    _subscriptions: Vec<Subscription>,
}

impl FindBar {
    pub fn new(editor: Entity<EditorView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| TextInput::new("Find in note", cx));
        let subscriptions = vec![
            cx.subscribe_in(&query, window, Self::on_query_event),
            // The count follows the editor's selection and searches.
            cx.observe(&editor, |_, _, cx| cx.notify()),
        ];
        Self {
            editor,
            query,
            case: CaseSensitivity::Insensitive,
            open: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn query(&self, cx: &App) -> String {
        self.query.read(cx).text().to_owned()
    }

    /// The count shown next to the query: "3 of 12", "12 matches", "No results", or nothing
    /// while there is no query or it has not been searched yet.
    pub fn status(&self, cx: &App) -> String {
        let Some(status) = self.editor.read(cx).find_status() else {
            return String::new();
        };
        match (status.current, status.total) {
            (_, 0) => "No results".into(),
            (Some(current), total) => format!("{} of {total}", current + 1),
            (None, 1) => "1 match".into(),
            (None, total) => format!("{total} matches"),
        }
    }

    pub fn match_case(&self) -> bool {
        self.case == CaseSensitivity::Sensitive
    }

    /// Opens the bar with the selected text as the query, if it is short and on one line, and moves
    /// focus to the query with all of it selected.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx).editor();
        let selection = editor.selection().range();
        // Checked before copying: the selection may be all of a 10 MB note.
        let len = selection.end.0 - selection.start.0;
        if (1..=MAX_QUERY_FROM_SELECTION).contains(&len) {
            let selected = editor.buffer().text_for_range(selection).into_owned();
            if !selected.contains('\n') {
                self.query
                    .update(cx, |query, cx| query.set_text(&selected, cx));
            }
        }
        self.open = true;
        self.search(cx);
        window.focus(&self.query.focus_handle(cx));
        self.query.update(cx, |query, cx| query.select_all(cx));
        cx.notify();
    }

    /// Closes the bar and puts the caret back in the note, keeping the selected match.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.editor.update(cx, |editor, cx| editor.end_find(cx));
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Selects the next or previous match. With the bar closed this reopens it on the last query
    /// (focus stays where it is), or opens it to type one.
    pub fn select_match(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            if self.query(cx).is_empty() {
                self.open(window, cx);
                return;
            }
            self.open = true;
            self.search(cx);
            cx.notify();
        }
        self.editor
            .update(cx, |editor, cx| editor.select_match(direction, cx));
    }

    fn toggle_match_case(&mut self, cx: &mut Context<Self>) {
        self.case = match self.case {
            CaseSensitivity::Insensitive => CaseSensitivity::Sensitive,
            CaseSensitivity::Sensitive => CaseSensitivity::Insensitive,
        };
        self.search(cx);
        cx.notify();
    }

    fn search(&self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let case = self.case;
        self.editor
            .update(cx, |editor, cx| editor.find(&query, case, cx));
    }

    fn on_query_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                if self.open {
                    self.search(cx);
                }
            }
            TextInputEvent::Confirmed => self.select_match(Direction::Next, window, cx),
            TextInputEvent::Cancelled => self.close(window, cx),
        }
    }
}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let focused = self.query.focus_handle(cx).is_focused(window);
        let status = self.status(cx);
        let icon_button = |id: &'static str, icon: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .font_family(typography::ICON_FONT_FAMILY)
                .text_xs()
                .text_color(theme.muted)
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(theme.foreground.opacity(0.06))
                        .text_color(theme.foreground)
                })
                .child(icon)
        };
        let match_case = self.match_case();

        div()
            .key_context(CONTEXT)
            .block_mouse_except_scroll()
            .debug_selector(|| "find-bar".into())
            .on_action(cx.listener(|this, _: &ToggleMatchCase, _, cx| this.toggle_match_case(cx)))
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .p_1()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .shadow_md()
            .child(
                div()
                    .id("find-field")
                    // Narrower in a narrow editor, down to room for a few words.
                    .w(px(260.))
                    .min_w(px(120.))
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(theme.background)
                    .border_1()
                    .border_color(if focused { theme.accent } else { theme.border })
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        window.focus(&this.query.focus_handle(cx))
                    }))
                    .child(
                        div()
                            .font_family(typography::ICON_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme.muted)
                            .child(SEARCH_ICON),
                    )
                    .child(div().flex_1().min_w_0().child(self.query.clone()))
                    .child(
                        div()
                            .debug_selector(|| "find-status".into())
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(SharedString::from(status)),
                    ),
            )
            .child(
                div()
                    .id("find-match-case")
                    .debug_selector(|| "find-match-case".into())
                    .flex_none()
                    .h(px(26.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .text_xs()
                    .cursor_pointer()
                    .map(|button| {
                        if match_case {
                            button
                                .border_color(theme.accent)
                                .bg(theme.selection)
                                .text_color(theme.foreground)
                        } else {
                            button
                                .border_color(gpui::transparent_black())
                                .text_color(theme.muted)
                                .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                        }
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_match_case(cx)))
                    .child("Aa"),
            )
            .child(
                icon_button("find-previous", PREVIOUS_ICON).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.select_match(Direction::Previous, window, cx)
                    },
                )),
            )
            .child(icon_button("find-next", NEXT_ICON).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.select_match(Direction::Next, window, cx),
            )))
            .child(
                icon_button("find-close", CLOSE_ICON).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                ),
            )
    }
}
