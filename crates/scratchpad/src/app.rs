use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use gpui::{
    App, Application, Bounds, DisplayId, Pixels, Size, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, point, prelude::*, px, size,
};
use scratchpad_core::{Config, RecoveryStore, default_config_path};

use crate::app_window::AppWindow;
use crate::notes::NotesLocation;
use crate::theme::{self, ThemeMode};
use crate::{actions, logging, settings};

const WINDOW_TITLE: &str = "Scratchpad";
const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1100.), px(720.));
const MIN_WINDOW_SIZE: Size<Pixels> = size(px(560.), px(360.));
/// Overrides the notes folder, e.g. to try the app against a scratch folder.
const NOTES_DIR_ENV: &str = "SCRATCHPAD_NOTES_DIR";
/// Overrides where the config file and recovery snapshots live, so a trial run (or a manual
/// test) never touches the user's own settings. See ADR 0082.
const CONFIG_DIR_ENV: &str = "SCRATCHPAD_CONFIG_DIR";

/// Where the app keeps its files. [`run`] uses the platform's folders; tests use temp folders.
#[derive(Clone, Debug)]
pub struct Storage {
    pub notes: NotesLocation,
    /// The notes folder was set by `SCRATCHPAD_NOTES_DIR`, so the settings cannot change it.
    pub notes_dir_overridden: bool,
    /// The config file. `None` keeps settings for this run only.
    pub config_path: Option<PathBuf>,
    /// The folder for crash recovery snapshots. `None` turns recovery off.
    pub recovery_dir: Option<PathBuf>,
    /// Watch the notes folder for changes by other programs. Tests turn this off and pass
    /// events to [`Session::disk_events`](crate::session::Session::disk_events) themselves.
    pub watch: bool,
}

/// Entry point used by `main`: starts the platform event loop and opens the main window.
pub fn run() {
    let started = Instant::now();
    logging::init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting Scratchpad");

    // Only the tiny config file is read before the window opens (PLAN §39).
    let config_dir = config_dir_override();
    let config_path = match &config_dir {
        Some(dir) => Some(dir.join("config.json")),
        None => default_config_path(),
    };
    let config = config_path.as_deref().map(Config::load).unwrap_or_default();
    let notes_dir_env = env_var(NOTES_DIR_ENV);
    let storage = Storage {
        notes_dir_overridden: notes_dir_env.is_some(),
        notes: NotesLocation::new(notes_dir(notes_dir_env, &config)),
        config_path,
        recovery_dir: match config_dir {
            Some(dir) => Some(dir.join("recovery")),
            None => RecoveryStore::default_dir(),
        },
        watch: true,
    };
    tracing::info!(dir = %storage.notes.dir.display(), "notes folder");

    Application::new().run(move |cx| {
        init(cx);
        let window = match open_main_window(storage, config, cx) {
            Ok(window) => window,
            Err(err) => {
                tracing::error!("failed to open main window: {err:#}");
                cx.quit();
                return;
            }
        };
        tracing::info!(elapsed = ?started.elapsed(), "main window created");
        window
            .update(cx, |root, window, cx| {
                // Next-frame callbacks run at the top of the first frame request, which
                // presents the scene drawn by `open_window`, so this is "first pixels".
                window.on_next_frame(move |_, _| {
                    tracing::info!(elapsed = ?started.elapsed(), "first frame shown");
                });
                root.on_first_note_shown(cx, move || {
                    tracing::info!(elapsed = ?started.elapsed(), "first note shown");
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

/// The folder from `SCRATCHPAD_CONFIG_DIR`, which then holds the config file, recovery
/// snapshots and the release log instead of the platform folders.
pub(crate) fn config_dir_override() -> Option<PathBuf> {
    env_var(CONFIG_DIR_ENV).map(PathBuf::from)
}

/// The environment variable `name`, unless it is unset or empty.
fn env_var(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// The folder from `SCRATCHPAD_NOTES_DIR` (`env`), else from the config, else
/// `Documents\Scratchpad`.
fn notes_dir(env: Option<OsString>, config: &Config) -> PathBuf {
    env.filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| config.notes_dir_or_default())
}

/// Opens the main window on the notes in `storage`, restoring what `config` remembers: window
/// bounds, theme, sidebar width and the last open note. The folder is listed in the background
/// after the window has rendered.
pub fn open_main_window(
    storage: Storage,
    config: Config,
    cx: &mut App,
) -> gpui::Result<WindowHandle<AppWindow>> {
    theme::set_mode(settings::theme_mode(config.theme), cx);
    let (display_id, window_bounds) = window_bounds(config.window, cx);
    settings::init(config, storage.config_path.clone(), cx);
    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        display_id,
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            ..Default::default()
        }),
        window_min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| AppWindow::new(storage, window, cx))
    })
}

/// The remembered window bounds, on the display that holds their centre, or the default size
/// centred on the main display when that display is gone (PLAN §32).
fn window_bounds(
    saved: Option<scratchpad_core::WindowBounds>,
    cx: &App,
) -> (Option<DisplayId>, WindowBounds) {
    let restored = saved.and_then(|saved| {
        let bounds = Bounds::new(
            point(px(saved.x), px(saved.y)),
            size(px(saved.width), px(saved.height)).max(&MIN_WINDOW_SIZE),
        );
        let display = cx
            .displays()
            .into_iter()
            .find(|display| display.bounds().contains(&bounds.center()))?;
        // Keep the whole window, title bar included, on that display.
        let area = display.bounds();
        let size = bounds.size.min(&area.size);
        let origin = point(
            bounds
                .origin
                .x
                .clamp(area.left(), area.right() - size.width),
            bounds
                .origin
                .y
                .clamp(area.top(), area.bottom() - size.height),
        );
        let bounds = Bounds::new(origin, size);
        let window_bounds = if saved.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        Some((Some(display.id()), window_bounds))
    });
    restored.unwrap_or_else(|| {
        (
            None,
            WindowBounds::Windowed(Bounds::centered(None, DEFAULT_WINDOW_SIZE, cx)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_overrides_the_configured_notes_folder() {
        let config = Config {
            notes_dir: Some(PathBuf::from("D:/Configured")),
            ..Config::default()
        };
        let env = |dir: &str| Some(OsString::from(dir));

        assert_eq!(notes_dir(env("D:/Env"), &config), PathBuf::from("D:/Env"));
        assert_eq!(notes_dir(env(""), &config), PathBuf::from("D:/Configured"));
        assert_eq!(notes_dir(None, &config), PathBuf::from("D:/Configured"));
        assert_eq!(
            notes_dir(None, &Config::default()),
            scratchpad_core::default_notes_dir()
        );
    }
}
