//! Notes, filesystem storage, search and configuration. Free of any UI framework.

mod naming;

pub use naming::{UNTITLED, sanitize_file_stem, title_from_content};
