//! The Scratchpad desktop application: GPUI window, views, theme and commands.
//!
//! All app code lives in this library so integration tests can drive the real views in a
//! headless GPUI test window; `main.rs` only calls [`run`].

pub mod actions;
mod app;
pub mod app_window;
pub mod editor_pane;
pub mod editor_view;
pub mod file_dialogs;
pub mod find_bar;
pub mod go_to_line;
mod logging;
pub mod menu_bar;
pub mod notes;
pub mod session;
pub mod settings;
pub mod settings_panel;
pub mod sidebar;
pub mod status_bar;
pub mod text_input;
pub mod theme;
pub mod toast;
#[cfg(windows)]
mod warm_up;

pub use app::{Storage, init, open_main_window, run};
pub use app_window::AppWindow;
