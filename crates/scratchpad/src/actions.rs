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

actions!(
    scratchpad,
    [
        /// Quit the application.
        Quit,
        /// Close the focused window.
        CloseWindow,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-w", CloseWindow, None),
    ]
}

/// Handlers for actions that act on the whole application rather than one window.
pub fn register_app_handlers(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
}
