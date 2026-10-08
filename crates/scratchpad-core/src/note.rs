use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Extension given to new notes (including the dot).
pub(crate) const NOTE_EXTENSION: &str = ".md";

/// A note file as seen in the sidebar. Everything is derived from filesystem metadata; the
/// content is never read to build one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub path: PathBuf,
    /// The file stem. In V1 the title and the file name are kept aligned (PLAN §44).
    pub title: String,
    pub modified_at: SystemTime,
    pub created_at: Option<SystemTime>,
    /// Size in bytes. Together with `modified_at` it tells search whether a cached copy of the
    /// content is still valid.
    pub len: u64,
}

impl Note {
    pub(crate) fn new(path: PathBuf, metadata: &Metadata) -> Self {
        let title = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        Note {
            path,
            title,
            modified_at: metadata.modified().unwrap_or(UNIX_EPOCH),
            created_at: metadata.created().ok(),
            len: metadata.len(),
        }
    }
}

/// Whether `path` is named like a note: a `.md` file (any case) that is not hidden. Hidden files
/// include our own temporary files, which start with a dot.
pub(crate) fn is_note_path(path: &Path) -> bool {
    let has_md_extension = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    let is_hidden = path
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."));
    has_md_extension && !is_hidden
}
