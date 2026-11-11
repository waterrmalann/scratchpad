//! What the app remembers between runs (PLAN §32): window bounds, sidebar width, theme and the
//! last open note, kept in `scratchpad_core::Config`. See ADR 0065.
//!
//! The file is read once before the window opens. Changes are written a moment after they
//! stop (dragging the sidebar edge changes the width on every mouse move) and when the window
//! closes or the app quits.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, Global, Task};
use scratchpad_core::{Config, ThemePreference};
use scratchpad_editor::markdown::{MarkdownOption, MarkdownOptions};

use crate::theme::{self, ActiveTheme, ThemeMode};

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

/// The pinned notes of every folder; empty before [`init`].
pub fn pinned_notes(cx: &App) -> Vec<PathBuf> {
    cx.try_global::<Settings>()
        .map(|settings| settings.config.pinned_notes.clone())
        .unwrap_or_default()
}

/// The config file, if settings are kept between runs.
pub fn path(cx: &App) -> Option<&Path> {
    cx.global::<Settings>().path.as_deref()
}

/// Applies the user's theme choice to every window and remembers it.
pub fn set_theme_mode(mode: ThemeMode, cx: &mut App) {
    theme::set_mode(mode, cx);
    update(cx, |config| config.theme = theme_preference(mode));
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

/// Pairs each Markdown option with the config field that remembers it (ADR 0185, 0190).
macro_rules! markdown_config_keys {
    ($($option:ident => $field:ident),* $(,)?) => {
        /// What `config` remembers for `option`: `None` for the default.
        fn stored(config: &Config, option: MarkdownOption) -> Option<bool> {
            match option {
                $(MarkdownOption::$option => config.$field,)*
            }
        }

        fn stored_mut(config: &mut Config, option: MarkdownOption) -> &mut Option<bool> {
            match option {
                $(MarkdownOption::$option => &mut config.$field,)*
            }
        }
    };
}

markdown_config_keys!(
    Callouts => markdown_callouts,
    Superscript => markdown_superscript,
    Subscript => markdown_subscript,
    HeadingIds => markdown_heading_ids,
    Wikilinks => markdown_wikilinks,
    Highlights => markdown_highlights,
    Math => markdown_math,
    EmojiShortcodes => markdown_emoji_shortcodes,
);

/// The Markdown options remembered in `config`, with the defaults for those never changed.
pub fn markdown_options(config: &Config) -> MarkdownOptions {
    MarkdownOptions::from_fn(|option| stored(config, option).unwrap_or(option.default_on()))
}

/// Remembers `options` in `config`: only those that differ from the default, so a default changed in a
/// later version reaches everyone who has not turned that option on or off themselves. A value written
/// by an older version that equals the default is read as it always was and dropped on the next write.
pub fn store_markdown_options(config: &mut Config, options: MarkdownOptions) {
    for option in MarkdownOption::ALL {
        let on = options.get(option);
        *stored_mut(config, option) = (on != option.default_on()).then_some(on);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_markdown_options_that_differ_from_the_default_are_stored() {
        // An older version wrote every option; the values that equal the default still read back.
        let mut config = Config {
            markdown_callouts: Some(true),
            markdown_superscript: Some(true),
            markdown_math: Some(false),
            ..Config::default()
        };
        let options = markdown_options(&config);
        assert!(options.callouts && options.superscript && !options.math);
        store_markdown_options(&mut config, options);
        assert_eq!(config.markdown_callouts, None);
        assert_eq!(config.markdown_superscript, Some(true));
        assert_eq!(config.markdown_math, None);
        assert_eq!(markdown_options(&config), options);
        store_markdown_options(&mut config, MarkdownOptions::default());
        assert_eq!(config, Config::default());
        for option in MarkdownOption::ALL {
            let mut options = MarkdownOptions::default();
            options.set(option, !option.default_on());
            store_markdown_options(&mut config, options);
            assert_eq!(stored(&config, option), Some(!option.default_on()));
            assert_eq!(markdown_options(&config), options);
        }
    }
}
