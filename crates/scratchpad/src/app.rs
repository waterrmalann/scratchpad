use std::path::PathBuf;
use std::time::Instant;

use gpui::{
    App, Application, Bounds, Size, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions,
    prelude::*, px, size,
};

use crate::app_window::AppWindow;
use crate::notes::NotesLocation;
use crate::theme::{self, ThemeMode};
use crate::{actions, logging};

const WINDOW_TITLE: &str = "Scratchpad";
const DEFAULT_WINDOW_SIZE: Size<gpui::Pixels> = size(px(1100.), px(720.));
const MIN_WINDOW_SIZE: Size<gpui::Pixels> = size(px(560.), px(360.));
/// Overrides the notes folder, e.g. to try the app against a scratch folder.
const NOTES_DIR_ENV: &str = "SCRATCHPAD_NOTES_DIR";

/// Entry point used by `main`: starts the platform event loop and opens the main window.
pub fn run() {
    let started = Instant::now();
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting Scratchpad");

    let notes = NotesLocation::new(notes_dir());
    tracing::info!(dir = %notes.dir.display(), "notes folder");

    Application::new().run(move |cx| {
        init(cx);
        let window = match open_main_window(notes, cx) {
            Ok(window) => window,
            Err(err) => {
                tracing::error!("failed to open main window: {err:#}");
                cx.quit();
                return;
            }
        };
        tracing::info!(elapsed = ?started.elapsed(), "main window created");
        window
            .update(cx, |_, window, _| {
                // Next-frame callbacks run at the top of the first frame request, which
                // presents the scene drawn by `open_window`, so this is "first pixels".
                window.on_next_frame(move |_, _| {
                    tracing::info!(elapsed = ?started.elapsed(), "first frame shown");
                });
            })
            .ok();
        cx.activate(true);
    });
}

/// App-wide setup that must precede opening windows. Tests call this too.
pub fn init(cx: &mut App) {
    theme::init(ThemeMode::System, cx);
    cx.bind_keys(actions::key_bindings());
    cx.bind_keys(actions::editor::key_bindings());
    actions::register_app_handlers(cx);
}

fn notes_dir() -> PathBuf {
    std::env::var_os(NOTES_DIR_ENV)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(scratchpad_core::default_notes_dir)
}

/// Opens the main window showing the notes in `notes`. The folder is listed in the background
/// after the window has rendered.
pub fn open_main_window(
    notes: NotesLocation,
    cx: &mut App,
) -> gpui::Result<WindowHandle<AppWindow>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            DEFAULT_WINDOW_SIZE,
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            ..Default::default()
        }),
        window_min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| AppWindow::new(notes, window, cx))
    })
}
