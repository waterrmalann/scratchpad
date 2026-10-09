//! Find and replace in the open note (PLAN §29): a small bar over the top right of the editor. See
//! ADR 0100 and ADR 0140.
//!
//! Ctrl+F opens it with the selected text (if it is short and on one line) as the query. Typing
//! searches after a pause and selects the first match from the cursor; Enter and F3 go to the
//! next match, Shift+Enter and Shift+F3 to the previous one, wrapping around. Escape closes it and
//! puts the caret back in the note with the match still selected. Ctrl+H, or the chevron on the
//! left, adds a second row with the replacement: Enter there replaces the selected match and
//! selects the next, Ctrl+Alt+Enter (or Alt+A) replaces them all. The editor finds, highlights and
//! replaces the matches; this view is the fields, the count and the buttons.

use gpui::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, KeyBinding, SharedString,
    Stateful, Subscription, Window, actions, div, prelude::*, px,
};
use scratchpad_editor::search::CaseSensitivity;

use crate::editor_view::{Direction, EditorView};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{ActiveTheme, Theme, typography};

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
        /// Open the find bar with its replace row, in the replacement field.
        ReplaceInNote,
        /// Replace every match.
        ReplaceAll,
        /// Move between the query and the replacement fields.
        SwitchField,
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
        // Replace (ADR 0140). Enter in the replacement field is its Confirm, which replaces.
        KeyBinding::new("secondary-h", ReplaceInNote, None),
        // VS Code's key, and the access key of Notepad's Replace All button.
        KeyBinding::new("ctrl-alt-enter", ReplaceAll, Some(CONTEXT)),
        KeyBinding::new("alt-a", ReplaceAll, Some(CONTEXT)),
        KeyBinding::new("tab", SwitchField, Some(CONTEXT)),
        KeyBinding::new("shift-tab", SwitchField, Some(CONTEXT)),
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
const COLLAPSED_ICON: &str = if cfg!(windows) {
    "\u{E76C}"
} else {
    "\u{203A}"
};
const EXPANDED_ICON: &str = NEXT_ICON;

pub struct FindBar {
    editor: Entity<EditorView>,
    query: Entity<TextInput>,
    replacement: Entity<TextInput>,
    case: CaseSensitivity,
    open: bool,
    /// The replace row is shown.
    replacing: bool,
    _subscriptions: Vec<Subscription>,
}

impl FindBar {
    pub fn new(editor: Entity<EditorView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| TextInput::new("Find in note", cx));
        let replacement = cx.new(|cx| TextInput::new("Replace with", cx));
        let subscriptions = vec![
            cx.subscribe_in(&query, window, Self::on_query_event),
            cx.subscribe_in(&replacement, window, Self::on_replacement_event),
            // The count follows the editor's selection and searches.
            cx.observe(&editor, |_, _, cx| cx.notify()),
        ];
        Self {
            editor,
            query,
            replacement,
            case: CaseSensitivity::Insensitive,
            open: false,
            replacing: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the replace row is shown.
    pub fn is_replacing(&self) -> bool {
        self.open && self.replacing
    }

    pub fn query(&self, cx: &App) -> String {
        self.query.read(cx).text().to_owned()
    }

    pub fn replacement(&self, cx: &App) -> String {
        self.replacement.read(cx).text().to_owned()
    }

    /// The count shown next to the query: "3 of 12", "12 matches", "No results", or nothing
    /// while there is no query or it has not been searched yet. After Replace All, how many
    /// matches it replaced.
    pub fn status(&self, cx: &App) -> String {
        let Some(status) = self.editor.read(cx).find_status() else {
            return String::new();
        };
        if let Some(replaced) = status.replaced {
            return format!("Replaced {replaced}");
        }
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

    /// Ctrl+F: opens the bar without the replace row, with the selected text as the query if it
    /// is short and on one line, and moves focus to the query with all of it selected.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show(false, window, cx);
    }

    /// Ctrl+H: like [`open`](Self::open), but with the replace row, and focus in the replacement.
    pub fn open_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show(true, window, cx);
    }

    fn show(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        self.replacing = replacing;
        self.search(cx);
        let field = if replacing {
            &self.replacement
        } else {
            &self.query
        };
        window.focus(&field.focus_handle(cx));
        field.update(cx, |field, cx| field.select_all(cx));
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

    /// Shows or hides the replace row. Hiding it takes focus out of the replacement.
    fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replacing = !self.replacing;
        if !self.replacing && self.replacement.focus_handle(cx).is_focused(window) {
            window.focus(&self.query.focus_handle(cx));
        }
        cx.notify();
    }

    /// Tab and Shift+Tab go between the two fields; with one field they stay in it.
    fn switch_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.replacing {
            return;
        }
        let field = if self.query.focus_handle(cx).is_focused(window) {
            &self.replacement
        } else {
            &self.query
        };
        window.focus(&field.focus_handle(cx));
        field.update(cx, |field, cx| field.select_all(cx));
    }

    /// Replace and Replace All act only while the replace row shows what they replace with.
    fn replace(&mut self, all: bool, cx: &mut Context<Self>) {
        if !self.is_replacing() {
            return;
        }
        let replacement = self.replacement(cx);
        self.editor.update(cx, |editor, cx| {
            if all {
                editor.replace_all(&replacement, cx);
            } else {
                editor.replace_match(&replacement, cx);
            }
        });
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

    fn on_replacement_event(
        &mut self,
        _: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {}
            TextInputEvent::Confirmed => self.replace(false, cx),
            TextInputEvent::Cancelled => self.close(window, cx),
        }
    }
}

/// A text field in the find bar's style: an outlined box around `input` that focuses it when
/// clicked, after an icon if there is one. An empty icon leaves the room for one, so that the
/// text of fields above each other lines up. Also used by Go to line.
pub(crate) fn input_box(
    id: &'static str,
    icon: Option<&'static str>,
    input: &Entity<TextInput>,
    theme: &Theme,
    window: &Window,
    cx: &App,
) -> Stateful<Div> {
    let focus = input.focus_handle(cx);
    let focused = focus.is_focused(window);
    div()
        .id(id)
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .bg(theme.background)
        .border_1()
        .border_color(if focused { theme.accent } else { theme.border })
        .on_click(move |_: &ClickEvent, window, _| window.focus(&focus))
        .children(icon.map(|icon| {
            div()
                .flex_none()
                .w(px(12.))
                .font_family(typography::ICON_FONT_FAMILY)
                .text_xs()
                .text_color(theme.muted)
                .child(icon)
        }))
        .child(div().flex_1().min_w_0().child(input.clone()))
}

/// A small push button in the find bar's style; muted and inert while not `enabled`.
pub(crate) fn text_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .h(px(26.))
        .px_2()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_xs()
        .map(|button| {
            if enabled {
                button
                    .text_color(theme.foreground)
                    .cursor_pointer()
                    .hover(move |style| style.border_color(accent))
            } else {
                button.text_color(theme.muted)
            }
        })
        .child(label)
}

impl Focusable for FindBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl Render for FindBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
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
        let can_replace = !self.editor.read(cx).is_read_only();

        let find_row = div()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .child(
                input_box(
                    "find-field",
                    Some(SEARCH_ICON),
                    &self.query,
                    &theme,
                    window,
                    cx,
                )
                // Narrower in a narrow editor, down to room for a few words.
                .w(px(260.))
                .min_w(px(120.))
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
            );
        let replace_row = self.replacing.then(|| {
            div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    // Lined up with the query, except in a narrow editor, where the buttons
                    // after it take more room than those after the query.
                    input_box(
                        "replace-field",
                        Some(""),
                        &self.replacement,
                        &theme,
                        window,
                        cx,
                    )
                    .w(px(260.))
                    .min_w(px(64.)),
                )
                .child(
                    text_button("replace-next", "Replace", can_replace, &theme).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.replace(false, cx)),
                    ),
                )
                .child(
                    text_button("replace-all", "Replace all", can_replace, &theme).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.replace(true, cx)),
                    ),
                )
        });
        let toggle_replace = div()
            .id("find-toggle-replace")
            .debug_selector(|| "find-toggle-replace".into())
            .flex_none()
            .w(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .font_family(typography::ICON_FONT_FAMILY)
            .text_size(px(10.))
            .text_color(theme.muted)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(theme.foreground.opacity(0.06))
                    .text_color(theme.foreground)
            })
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_replace(window, cx)),
            )
            .child(if self.replacing {
                EXPANDED_ICON
            } else {
                COLLAPSED_ICON
            });

        // Not aligned: the chevron stretches over both rows.
        div()
            .key_context(CONTEXT)
            .block_mouse_except_scroll()
            .debug_selector(|| "find-bar".into())
            .on_action(cx.listener(|this, _: &ToggleMatchCase, _, cx| this.toggle_match_case(cx)))
            .on_action(cx.listener(|this, _: &ReplaceAll, _, cx| this.replace(true, cx)))
            .on_action(
                cx.listener(|this, _: &SwitchField, window, cx| this.switch_field(window, cx)),
            )
            .min_w_0()
            .flex()
            .gap_1()
            .p_1()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .shadow_md()
            .child(toggle_replace)
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(find_row)
                    .children(replace_row),
            )
    }
}
