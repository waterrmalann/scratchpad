//! Notes, filesystem storage, search and configuration. Free of any UI framework.

mod atomic;
mod config;
mod error;
mod grouping;
mod naming;
mod note;
mod recovery;
mod search;
mod store;
mod watcher;

pub use config::{Config, ThemePreference, WindowBounds, default_config_path, default_notes_dir};
pub use error::{Error, Result};
pub use grouping::{DateGroup, NaiveDate, local_date};
pub use naming::{UNTITLED, sanitize_file_stem, title_from_content};
pub use note::{Note, is_note_path};
pub use recovery::{RecoveryStore, Snapshot};
pub use search::{NoteSearch, SearchHit};
pub use store::{NoteStore, NoteText};
pub use watcher::{NoteEvent, NoteWatcher};
