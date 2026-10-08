//! The Scratchpad desktop application: GPUI window, views, theme and commands.
//!
//! All app code lives in this library so integration tests can drive the real views in a
//! headless GPUI test window; `main.rs` only calls [`run`].

pub mod actions;
mod app;
pub mod app_window;
pub mod editor_pane;
pub mod editor_view;
mod logging;
pub mod notes;
pub mod sidebar;
pub mod text_input;
pub mod theme;
pub mod toast;

pub use app::{init, open_main_window, run};
pub use app_window::AppWindow;
