//! What the app remembers between runs (PLAN §32): window bounds, sidebar width, theme and the
//! last open note, kept in `scratchpad_core::Config`. See ADR 0065.
//!
//! The file is read once before the window opens. Changes are written a moment after they
//! stop (dragging the sidebar edge changes the width on every mouse move) and when the window
//! closes or the app quits.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, Global, Task};
use scratchpad_core::{Config, ThemePreference};

use crate::theme::{ActiveTheme, ThemeMode};

/// How long changes settle before the config file is written.
pub const SAVE_DELAY: Duration = Duration::from_secs(1);

struct Settings {
    config: Config,
    /// `None` when the platform has no config folder: settings then last for this run only.
    path: Option<PathBuf>,
    /// The pending delayed write; replacing it cancels the previous one.
    save_task: Option<Task<()>>,
}

impl Global for Settings {}

/// Makes `config`, read from `path`, the app's settings.
pub fn init(config: Config, path: Option<PathBuf>, cx: &mut App) {
    cx.set_global(Settings {
        config,
        path,
        save_task: None,
    });
}

pub fn get(cx: &App) -> &Config {
    &cx.global::<Settings>().config
}

/// Changes the settings and writes them to disk after [`SAVE_DELAY`] if anything changed.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Config)) {
    let settings = cx.global_mut::<Settings>();
    let before = settings.config.clone();
    change(&mut settings.config);
    if settings.config == before {
        return;
    }
    let timer = cx.background_executor().timer(SAVE_DELAY);
    let save = cx.spawn(async move |cx| {
        timer.await;
        cx.update(save_now).ok();
    });
    cx.global_mut::<Settings>().save_task = Some(save);
}

/// Writes the settings now, e.g. when the app closes. The file is tiny, so this runs on the
/// UI thread: no write can then land after it.
pub fn save_now(cx: &mut App) {
    let theme = theme_preference(cx.theme_mode());
    let settings = cx.global_mut::<Settings>();
    settings.save_task = None;
    settings.config.theme = theme;
    let Some(path) = &settings.path else {
        return;
    };
    match settings.config.save(path) {
        Ok(()) => tracing::debug!(path = %path.display(), "saved settings"),
        Err(error) => tracing::warn!(%error, "could not save settings"),
    }
}

pub fn theme_mode(preference: ThemePreference) -> ThemeMode {
    match preference {
        ThemePreference::System => ThemeMode::System,
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    }
}

fn theme_preference(mode: ThemeMode) -> ThemePreference {
    match mode {
        ThemeMode::System => ThemePreference::System,
        ThemeMode::Light => ThemePreference::Light,
        ThemeMode::Dark => ThemePreference::Dark,
    }
}
