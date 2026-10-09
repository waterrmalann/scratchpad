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

use crate::{find_bar, go_to_line, settings_panel, text_input};

actions!(
    scratchpad,
    [
        /// Quit the application.
        Quit,
        /// Close the focused window.
        CloseWindow,
        /// Start a new note (PLAN §9).
        NewNote,
        /// Save the open note now (PLAN §35); it is also saved automatically.
        SaveNote,
        /// Move focus to the note search (PLAN §28).
        SearchNotes,
        /// Open the note above the open one in the note list.
        SelectPreviousNote,
        /// Open the note below the open one in the note list.
        SelectNextNote,
        /// Move from the note list into the open note.
        FocusOpenNote,
        /// Rename the open note in place.
        RenameNote,
        /// Move the open note to the recycle bin.
        DeleteNote,
        /// Show the settings (PLAN §35).
        OpenSettings,
    ]
);

// Window layout (ADR 0135, 0137).
actions!(
    scratchpad,
    [
        /// Show or hide the note list. In a narrow window it floats over the note.
        ToggleSidebar,
        /// Show or hide the status bar under the note. No key; the View menu offers it.
        ToggleStatusBar,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
        KeyBinding::new("secondary-n", NewNote, None),
        KeyBinding::new("secondary-s", SaveNote, None),
        KeyBinding::new("secondary-p", SearchNotes, None),
        KeyBinding::new("secondary-shift-f", SearchNotes, None),
        KeyBinding::new("up", SelectPreviousNote, Some("NoteList")),
        KeyBinding::new("down", SelectNextNote, Some("NoteList")),
        KeyBinding::new("enter", FocusOpenNote, Some("NoteList")),
        KeyBinding::new("f2", RenameNote, Some("NoteList")),
        KeyBinding::new("delete", DeleteNote, Some("NoteList")),
        KeyBinding::new("secondary-,", OpenSettings, None),
    ];
    // Window layout (ADR 0135).
    bindings.push(KeyBinding::new("secondary-\\", ToggleSidebar, None));
    bindings.extend(file_key_bindings());
    // Components that own their actions, scoped to their key context.
    bindings.extend(text_input::key_bindings());
    bindings.extend(settings_panel::key_bindings());
    bindings.extend(find_bar::key_bindings());
    bindings.extend(view::key_bindings());
    bindings.extend(go_to_line::key_bindings());
    bindings
}

// --- Files from outside the notes folder (ADR 0145) ---

actions!(
    scratchpad,
    [
        /// Open a file from anywhere: a note of the notes folder opens as a note, any other
        /// file is edited in place (File > Open).
        OpenFile,
    ]
);

fn file_key_bindings() -> [KeyBinding; 1] {
    [KeyBinding::new("secondary-o", OpenFile, None)]
}

/// Handlers for actions that act on the whole application rather than one window.
pub fn register_app_handlers(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// How the note is shown: text size and word wrap. Handled by the main window, so they work
/// wherever focus is, and they apply to the note's text only, not to the rest of the window.
pub mod view {
    use gpui::{KeyBinding, actions};

    actions!(
        view,
        [
            /// Makes the note's text 10% larger.
            ZoomIn,
            /// Makes the note's text 10% smaller.
            ZoomOut,
            /// Back to the normal text size.
            ResetZoom,
            /// Turns wrapping long lines at the window's edge on or off.
            ToggleWordWrap,
        ]
    );

    pub fn key_bindings() -> Vec<KeyBinding> {
        // Windows reports Ctrl+Shift+= and Ctrl+numpad-plus as `ctrl-+`, and Ctrl+numpad-minus
        // as `ctrl--`. Word wrap has no key, as in Notepad.
        vec![
            KeyBinding::new("secondary-=", ZoomIn, None),
            KeyBinding::new("secondary-+", ZoomIn, None),
            KeyBinding::new("secondary--", ZoomOut, None),
            KeyBinding::new("secondary-0", ResetZoom, None),
        ]
    }
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
            /// Nests the list items the selection touches one level deeper, or inserts four spaces.
            Tab,
            /// Moves the list items the selection touches one level out.
            Outdent,
            DuplicateLines,
            MoveLinesUp,
            MoveLinesDown,
            Undo,
            Redo,
            Copy,
            Cut,
            Paste,
            /// A line break that does not continue the list item or quote (Enter does).
            PlainNewline,
            /// Markdown formatting of the selection or the word at the cursor (PLAN §36).
            ToggleBold,
            ToggleItalic,
            ToggleStrikethrough,
            ToggleInlineCode,
            /// Wraps the selection in `[...]()` and puts the cursor where the address goes.
            InsertLink,
            /// Switches between live preview and showing every Markdown marker.
            ToggleSourceMode,
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
            KeyBinding::new("shift-enter", PlainNewline, context),
            KeyBinding::new("tab", Tab, context),
            KeyBinding::new("shift-tab", Outdent, context),
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
            KeyBinding::new("secondary-b", ToggleBold, context),
            KeyBinding::new("secondary-i", ToggleItalic, context),
            KeyBinding::new("secondary-shift-x", ToggleStrikethrough, context),
            KeyBinding::new("secondary-e", ToggleInlineCode, context),
            KeyBinding::new("secondary-k", InsertLink, context),
            KeyBinding::new("secondary-/", ToggleSourceMode, context),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    #[test]
    fn no_two_bindings_share_keys_in_one_context() {
        let mut seen = HashSet::new();
        for binding in super::key_bindings()
            .iter()
            .chain(&super::editor::key_bindings())
        {
            let keys = format!("{:?} in {:?}", binding.keystrokes(), binding.predicate());
            assert!(seen.insert(keys.clone()), "{keys} is bound twice");
        }
    }
}
