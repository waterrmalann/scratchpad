//! A transient message at the bottom of the window for errors the user should know about but
//! that need no decision, e.g. `Could not rename "Ideas". Access is denied.` (PLAN §56).
//! See ADR 0054.
//!
//! One message at a time: a new one replaces the current one. It disappears after
//! [`TOAST_DURATION`] or when clicked.

use std::io;
use std::time::Duration;

use gpui::{App, Global, IntoElement, SharedString, Task, div, prelude::*, px};

use crate::theme::ActiveTheme;

pub const TOAST_DURATION: Duration = Duration::from_secs(6);

struct Toast {
    message: SharedString,
    /// Hides this message; dropped (and so cancelled) when a newer message replaces it.
    _dismiss: Task<()>,
}

impl Global for Toast {}

/// Shows `message`, replacing any message already shown.
pub fn show_error(message: impl Into<SharedString>, cx: &mut App) {
    let message = message.into();
    tracing::warn!(%message, "showing error");
    let timer = cx.background_executor().timer(TOAST_DURATION);
    let dismiss = cx.spawn(async move |cx| {
        timer.await;
        cx.update(dismiss).ok();
    });
    cx.set_global(Toast {
        message,
        _dismiss: dismiss,
    });
    cx.refresh_windows();
}

/// Shows a failed file operation on the note or folder called `name`, e.g.
/// `Could not save "Meeting Notes". Access is denied.`
pub fn show_file_error(name: &str, error: &scratchpad_core::Error, cx: &mut App) {
    let scratchpad_core::Error::Io { action, source, .. } = error;
    show_error(
        format!("Could not {action} \"{name}\". {}", describe(source)),
        cx,
    );
}

/// The OS description of `error` without the trailing ` (os error 5)`.
pub fn describe(error: &io::Error) -> String {
    let text = error.to_string();
    match text.rfind(" (os error ") {
        Some(start) if text.ends_with(')') => text[..start].to_owned(),
        _ => text,
    }
}

/// The message currently shown, if any.
pub fn current(cx: &App) -> Option<SharedString> {
    cx.try_global::<Toast>().map(|toast| toast.message.clone())
}

pub fn dismiss(cx: &mut App) {
    if cx.has_global::<Toast>() {
        cx.remove_global::<Toast>();
        cx.refresh_windows();
    }
}

/// The toast element, positioned at the bottom centre of its (relatively positioned) parent.
pub fn render(cx: &App) -> Option<impl IntoElement + use<>> {
    let message = current(cx)?;
    let theme = cx.theme();
    Some(
        div()
            .absolute()
            .bottom(px(20.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("toast")
                    .debug_selector(|| "toast".into())
                    .max_w(px(520.))
                    .mx_4()
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .shadow_lg()
                    // Inverted colours so it stands out from both the page and the sidebar.
                    .bg(theme.foreground)
                    .text_color(theme.background)
                    .cursor_pointer()
                    // The click only dismisses it, without also landing in the note under it.
                    .block_mouse_except_scroll()
                    .on_click(|_, _, cx| dismiss(cx))
                    .child(message),
            ),
    )
}
