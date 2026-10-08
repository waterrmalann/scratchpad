//! Commands and their default key bindings (PLAN §34–35).
//!
//! To add a command:
//! 1. add its name to the `actions!` list below,
//! 2. bind a key in [`key_bindings`] (`secondary` is Ctrl on Windows/Linux, Cmd on macOS;
//!    pass a key context such as `Some("Editor")` to scope the binding to a view that sets
//!    `.key_context("Editor")`),
//! 3. handle it where the behaviour lives: `.on_action(cx.listener(Self::handler))` on the
//!    owning view's root element, or in [`register_app_handlers`] if it needs no window.

use gpui::{App, KeyBinding, actions};

use crate::text_input;

actions!(
    scratchpad,
    [
        /// Quit the application.
        Quit,
        /// Close the focused window.
        CloseWindow,
        /// Start a new note (PLAN §9).
        NewNote,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
        KeyBinding::new("secondary-n", NewNote, None),
    ];
    // Components that own their actions, scoped to their key context.
    bindings.extend(text_input::key_bindings());
    bindings
}

/// Handlers for actions that act on the whole application rather than one window.
pub fn register_app_handlers(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// Text editing commands, handled by [`EditorView`](crate::editor_view::EditorView) while it has
/// focus (key context `Editor`).
pub mod editor {
    use gpui::{KeyBinding, actions};

    actions!(
        editor,
        [
            MoveLeft,
            MoveRight,
            /// Up one visual (wrapped) row.
            MoveUp,
            /// Down one visual (wrapped) row.
            MoveDown,
            MoveWordLeft,
            MoveWordRight,
            /// Start of the visual row.
            MoveToRowStart,
            /// End of the visual row.
            MoveToRowEnd,
            MoveToDocumentStart,
            MoveToDocumentEnd,
            PageUp,
            PageDown,
            SelectLeft,
            SelectRight,
            SelectUp,
            SelectDown,
            SelectWordLeft,
            SelectWordRight,
            SelectToRowStart,
            SelectToRowEnd,
            SelectToDocumentStart,
            SelectToDocumentEnd,
            SelectPageUp,
            SelectPageDown,
            SelectAll,
            Backspace,
            Delete,
            DeleteWordLeft,
            DeleteWordRight,
            Newline,
            Tab,
            DuplicateLines,
            MoveLinesUp,
            MoveLinesDown,
            Undo,
            Redo,
            Copy,
            Cut,
            Paste,
        ]
    );

    pub fn key_bindings() -> Vec<KeyBinding> {
        let context = Some("Editor");
        vec![
            KeyBinding::new("left", MoveLeft, context),
            KeyBinding::new("right", MoveRight, context),
            KeyBinding::new("up", MoveUp, context),
            KeyBinding::new("down", MoveDown, context),
            KeyBinding::new("ctrl-left", MoveWordLeft, context),
            KeyBinding::new("ctrl-right", MoveWordRight, context),
            KeyBinding::new("home", MoveToRowStart, context),
            KeyBinding::new("end", MoveToRowEnd, context),
            KeyBinding::new("ctrl-home", MoveToDocumentStart, context),
            KeyBinding::new("ctrl-end", MoveToDocumentEnd, context),
            KeyBinding::new("pageup", PageUp, context),
            KeyBinding::new("pagedown", PageDown, context),
            KeyBinding::new("shift-left", SelectLeft, context),
            KeyBinding::new("shift-right", SelectRight, context),
            KeyBinding::new("shift-up", SelectUp, context),
            KeyBinding::new("shift-down", SelectDown, context),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, context),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, context),
            KeyBinding::new("shift-home", SelectToRowStart, context),
            KeyBinding::new("shift-end", SelectToRowEnd, context),
            KeyBinding::new("ctrl-shift-home", SelectToDocumentStart, context),
            KeyBinding::new("ctrl-shift-end", SelectToDocumentEnd, context),
            KeyBinding::new("shift-pageup", SelectPageUp, context),
            KeyBinding::new("shift-pagedown", SelectPageDown, context),
            KeyBinding::new("secondary-a", SelectAll, context),
            KeyBinding::new("backspace", Backspace, context),
            KeyBinding::new("shift-backspace", Backspace, context),
            KeyBinding::new("delete", Delete, context),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, context),
            KeyBinding::new("ctrl-delete", DeleteWordRight, context),
            KeyBinding::new("enter", Newline, context),
            KeyBinding::new("shift-enter", Newline, context),
            KeyBinding::new("tab", Tab, context),
            KeyBinding::new("secondary-shift-d", DuplicateLines, context),
            KeyBinding::new("alt-up", MoveLinesUp, context),
            KeyBinding::new("alt-down", MoveLinesDown, context),
            KeyBinding::new("secondary-z", Undo, context),
            KeyBinding::new("secondary-shift-z", Redo, context),
            KeyBinding::new("secondary-y", Redo, context),
            KeyBinding::new("secondary-c", Copy, context),
            KeyBinding::new("ctrl-insert", Copy, context),
            KeyBinding::new("secondary-x", Cut, context),
            KeyBinding::new("shift-delete", Cut, context),
            KeyBinding::new("secondary-v", Paste, context),
            KeyBinding::new("shift-insert", Paste, context),
        ]
    }
}
