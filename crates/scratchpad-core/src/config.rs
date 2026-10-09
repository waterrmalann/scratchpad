//! The small application configuration file (PLAN §32-33).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::atomic::write_atomic;
use crate::error::{Result, io_context};

const APP_DIR_NAME: &str = "Scratchpad";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// Window geometry in logical pixels. Not validated: the app should clamp it to the available
/// displays, since monitors come and go.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

/// Everything Scratchpad remembers between runs. `None` means "use the app's default".
///
/// Loading never fails: a field that is missing or has the wrong type falls back to its default
/// without affecting the other fields.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(deserialize_with = "or_default")]
    pub window: Option<WindowBounds>,
    #[serde(deserialize_with = "or_default")]
    pub sidebar_width: Option<f32>,
    #[serde(deserialize_with = "or_default")]
    pub last_opened_note: Option<PathBuf>,
    #[serde(deserialize_with = "or_default")]
    pub theme: ThemePreference,
    /// Overrides [`default_notes_dir`].
    #[serde(deserialize_with = "or_default")]
    pub notes_dir: Option<PathBuf>,
    /// The user hid the sidebar. Narrow windows hide it whatever this says.
    #[serde(deserialize_with = "or_default")]
    pub sidebar_collapsed: bool,
    /// The user hid the status bar; it is shown by default.
    #[serde(deserialize_with = "or_default")]
    pub status_bar_hidden: bool,
    /// Size of the note's text in percent of the normal size (View > Zoom).
    #[serde(deserialize_with = "or_default")]
    pub zoom_percent: Option<u16>,
    /// Whether long lines wrap at the window's edge (View > Word wrap); `None` is on.
    #[serde(deserialize_with = "or_default")]
    pub word_wrap: Option<bool>,
}

impl Config {
    /// Reads the config at `path`. A missing file, unreadable file or malformed JSON yields the
    /// defaults and is logged; startup must never fail because of this file.
    pub fn load(path: &Path) -> Config {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tracing::debug!(path = %path.display(), "no config file yet, using defaults");
                return Config::default();
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot read config, using defaults");
                return Config::default();
            }
        };
        let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        // Going through `Value` first: serde would also accept a JSON array for this struct.
        let parsed = match serde_json::from_slice::<serde_json::Value>(json) {
            Ok(value @ serde_json::Value::Object(_)) => {
                Config::deserialize(value).map_err(|error| error.to_string())
            }
            Ok(_) => Err("expected a JSON object".to_owned()),
            Err(error) => Err(error.to_string()),
        };
        parsed.unwrap_or_else(|error| {
            tracing::warn!(path = %path.display(), %error, "invalid config, using defaults");
            Config::default()
        })
    }

    /// Writes the config to `path` atomically, creating its folder if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(io::Error::other)
            .map_err(io_context("encode config", path))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_context("create config folder", parent))?;
        }
        write_atomic(path, &json).map_err(io_context("save config", path))
    }

    /// The configured notes folder, or the default one.
    pub fn notes_dir_or_default(&self) -> PathBuf {
        self.notes_dir.clone().unwrap_or_else(default_notes_dir)
    }
}

/// `<platform config dir>/Scratchpad/config.json`, e.g. `%APPDATA%\Scratchpad\config.json`.
/// `None` when the platform has no config directory; the app then runs without persistence.
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR_NAME).join("config.json"))
}

/// `<Documents>/Scratchpad`, falling back to `~/Documents/Scratchpad` and finally to a
/// `Scratchpad` folder in the working directory.
pub fn default_notes_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_default()
        .join(APP_DIR_NAME)
}

/// Deserializes a field, substituting its default (and logging) instead of failing the whole
/// file when the value has the wrong shape.
fn or_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_else(|error| {
        tracing::warn!(%error, "ignoring invalid config value");
        T::default()
    }))
}
