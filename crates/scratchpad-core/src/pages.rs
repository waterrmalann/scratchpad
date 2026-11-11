//! The pages wikilinks resolve to: the notes and plain text files of one folder, looked up by stem ignoring
//! case (ADR 0188).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, io_context};
use crate::naming::sanitize_file_stem;

/// Extensions of the files a page name resolves to, in the order they are preferred: `[[Ideas]]` opens
/// `Ideas.md` rather than `Ideas.txt` when both exist.
pub const PAGE_EXTENSIONS: [&str; 2] = ["md", "txt"];

/// Whether `path` is named like a page: a `.md` or `.txt` file (any case) that is not hidden.
pub fn is_page_path(path: &Path) -> bool {
    page_rank(path).is_some()
        && path
            .file_name()
            .is_some_and(|name| !name.as_encoded_bytes().starts_with(b"."))
}

/// The index in [`PAGE_EXTENSIONS`] of `path`'s extension.
fn page_rank(path: &Path) -> Option<usize> {
    let extension = path.extension()?;
    PAGE_EXTENSIONS
        .iter()
        .position(|page| extension.eq_ignore_ascii_case(page))
}

/// The pages of one folder by lower-cased stem, for checking whether a wikilink's page exists without going to
/// the disk: lookups, insertions and removals are hash map operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageIndex {
    /// Per lower-cased stem, the file with each of [`PAGE_EXTENSIONS`], if any.
    pages: HashMap<String, [Option<PathBuf>; PAGE_EXTENSIONS.len()]>,
}

impl PageIndex {
    /// The pages among `paths`; other paths are left out.
    pub fn from_paths<P: Into<PathBuf>>(paths: impl IntoIterator<Item = P>) -> Self {
        let mut index = Self::default();
        for path in paths {
            index.insert(path.into());
        }
        index
    }

    /// Lists the pages directly in `dir` from its directory entries (no file is read). Hidden files, folders
    /// and files of other types are left out. Blocks on the disk: call it off the UI thread.
    pub fn read(dir: &Path) -> Result<Self> {
        let entries = fs::read_dir(dir).map_err(io_context("list pages in", dir))?;
        let files = entries.filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
            (is_file && is_page_path(&path) && !is_hidden(&entry)).then_some(path)
        });
        Ok(Self::from_paths(files))
    }

    /// Adds the page at `path`, if it is named like one. Returns whether the index changed.
    pub fn insert(&mut self, path: PathBuf) -> bool {
        let (Some(key), Some(rank)) = (stem_key(&path), page_rank(&path)) else {
            return false;
        };
        if !is_page_path(&path) {
            return false;
        }
        let slot = &mut self.pages.entry(key).or_default()[rank];
        if slot.as_ref() == Some(&path) {
            return false;
        }
        *slot = Some(path);
        true
    }

    /// Whether the index holds the page at `path`, spelled as it is there.
    pub fn contains(&self, path: &Path) -> bool {
        let (Some(key), Some(rank)) = (stem_key(path), page_rank(path)) else {
            return false;
        };
        self.pages
            .get(&key)
            .is_some_and(|files| files[rank].as_deref() == Some(path))
    }

    /// Drops the page at `path`. Returns whether the index changed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let (Some(key), Some(rank)) = (stem_key(path), page_rank(path)) else {
            return false;
        };
        let Some(files) = self.pages.get_mut(&key) else {
            return false;
        };
        if files[rank].as_deref() != Some(path) {
            return false;
        }
        files[rank] = None;
        if files.iter().all(Option::is_none) {
            self.pages.remove(&key);
        }
        true
    }

    /// The file the page `page` names: of the files whose stem equals it ignoring case, the `.md` one, else the
    /// `.txt` one. A page that is not a plain file stem, one [`sanitize_file_stem`] would change (separators,
    /// `..`, drive letters, reserved device names, control characters...), names nothing.
    pub fn resolve(&self, page: &str) -> Option<&Path> {
        if !is_plain_stem(page) {
            return None;
        }
        self.pages
            .get(&page.to_lowercase())?
            .iter()
            .flatten()
            .next()
            .map(PathBuf::as_path)
    }

    /// How many page names the index holds.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

/// Whether `page` is safe as a file stem as it is: what the store would create for it is exactly `page`.
pub fn is_plain_stem(page: &str) -> bool {
    !page.contains("..") && sanitize_file_stem(page).as_deref() == Some(page)
}

fn stem_key(path: &Path) -> Option<String> {
    Some(path.file_stem()?.to_string_lossy().to_lowercase())
}

#[cfg(windows)]
fn is_hidden(entry: &fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn is_hidden(_entry: &fs::DirEntry) -> bool {
    false
}
