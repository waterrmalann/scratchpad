use std::time::Instant;

use gpui::{
    App, Application, Bounds, Size, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions,
    prelude::*, px, size,
};

use crate::app_window::AppWindow;
use crate::theme::{self, ThemeMode};
use crate::{actions, logging};

const WINDOW_TITLE: &str = "Scratchpad";
const DEFAULT_WINDOW_SIZE: Size<gpui::Pixels> = size(px(1100.), px(720.));
const MIN_WINDOW_SIZE: Size<gpui::Pixels> = size(px(560.), px(360.));

/// Entry point used by `main`: starts the platform event loop and opens the main window.
pub fn run() {
    let started = Instant::now();
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting Scratchpad");

    Application::new().run(move |cx| {
        init(cx);
        let window = match open_main_window(cx) {
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

pub fn open_main_window(cx: &mut App) -> gpui::Result<WindowHandle<AppWindow>> {
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
        cx.new(|cx| AppWindow::new(window, cx))
    })
}
