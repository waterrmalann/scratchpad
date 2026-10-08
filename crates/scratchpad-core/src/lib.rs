//! Notes, filesystem storage, search and configuration. Free of any UI framework.

mod atomic;
mod error;
mod grouping;
mod naming;
mod note;
mod store;

pub use error::{Error, Result};
pub use grouping::{DateGroup, NaiveDate, group_notes, local_date};
pub use naming::{UNTITLED, sanitize_file_stem, title_from_content};
pub use note::Note;
pub use store::{NoteStore, NoteText};
