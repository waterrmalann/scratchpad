//! The pages wikilinks resolve to: the notes and plain text files of one folder, looked up by stem ignoring
//! case as NTFS does (ADR 0188).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;

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

/// `c` as NTFS compares it in names: upper-cased through a table of single UTF-16 units, so a char whose upper
/// case is several chars (`ß`) or outside the Basic Multilingual Plane keeps its own value.
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    let in_bmp = |c: char| u32::from(c) <= 0xFFFF;
    if !in_bmp(c) {
        return c;
    }
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(upper), None) if in_bmp(upper) => upper,
        _ => c,
    }
}

/// `name` with each char folded by [`fold_char`]: two names are the same file name on Windows when their
/// folds are equal. Unlike [`str::to_lowercase`] it maps char by char, so `ΟΔΟΣ` and `οδοσ` (and `οδος`) agree.
pub fn fold_case(name: &str) -> String {
    name.chars().map(fold_char).collect()
}

/// A page: the file name of its preferred file as spelled on disk, hashed and compared by its stem as
/// [`fold_case`] would. A page name looked up is a key with no extension.
#[derive(Debug, Clone)]
struct PageKey {
    name: Box<str>,
    stem_len: usize,
}

impl PageKey {
    fn page(page: &str) -> Self {
        Self {
            name: page.into(),
            stem_len: page.len(),
        }
    }

    fn stem(&self) -> &str {
        &self.name[..self.stem_len]
    }

    /// The extension as spelled, if the key has one.
    fn extension(&self) -> Option<&str> {
        self.name.get(self.stem_len + 1..)
    }
}

impl PartialEq for PageKey {
    fn eq(&self, other: &Self) -> bool {
        self.stem()
            .chars()
            .map(fold_char)
            .eq(other.stem().chars().map(fold_char))
    }
}

impl Eq for PageKey {}

impl Hash for PageKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for c in self.stem().chars() {
            state.write_u32(u32::from(fold_char(c)));
        }
        state.write_u8(0xFF);
    }
}

/// Which of [`PAGE_EXTENSIONS`] a page has, one bit each.
type Extensions = u8;

/// Changes kept apart from the shared map before they are merged into it: a few dozen, or more in a large
/// folder, so that merging (a copy of the map when it is shared) is rare.
const MIN_PENDING_CHANGES: usize = 32;

/// The pages of one folder by stem ignoring case, for checking whether a wikilink's page exists without going to
/// the disk.
///
/// It holds each page's file name once and which extensions it has, not its path. Cloning it is cheap: the map read
/// from the folder is shared, and insertions and removals made since are kept in a small map of their own until
/// there are enough of them to be merged in. So a model can keep it in an `Arc`, hand clones to views, and
/// update it with `Arc::make_mut` without copying every page.
#[derive(Debug, Clone, Default)]
pub struct PageIndex {
    dir: PathBuf,
    /// The pages as listed, or as of the last merge.
    listed: Arc<HashMap<PageKey, Extensions>>,
    /// Pages added or removed since, which win over `listed`; `0` for a page that is gone.
    changed: HashMap<PageKey, Extensions>,
}

impl PageIndex {
    /// An index of no pages of `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ..Self::default()
        }
    }

    /// The pages among `paths`, which are files of `dir`; other paths are left out.
    pub fn from_paths<P: AsRef<Path>>(
        dir: impl Into<PathBuf>,
        paths: impl IntoIterator<Item = P>,
    ) -> Self {
        let mut index = Self::new(dir);
        for path in paths {
            index.insert(path.as_ref());
        }
        index
    }

    /// Lists the pages directly in `dir` from its directory entries (no file is read). Hidden files, folders
    /// and files of other types are left out. Blocks on the disk: call it off the UI thread.
    pub fn read(dir: &Path) -> Result<Self> {
        crate::store::list_folder(dir, crate::store::Listing::Pages).map(|(_, pages)| pages)
    }

    /// The folder the pages are in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn extensions(&self, key: &PageKey) -> Extensions {
        match self.changed.get(key) {
            Some(&extensions) => extensions,
            None => self.listed.get(key).copied().unwrap_or(0),
        }
    }

    /// The page's stem as the index spells it.
    fn stored_key(&self, key: &PageKey) -> Option<PageKey> {
        match self.changed.get_key_value(key) {
            Some((stored, _)) => Some(stored.clone()),
            None => self
                .listed
                .get_key_value(key)
                .map(|(stored, _)| stored.clone()),
        }
    }

    /// Records the extensions of the page `key`, spelled as `key` when `respell` (its preferred file was just
    /// added), else as before.
    fn set_extensions(&mut self, key: PageKey, extensions: Extensions, respell: bool) {
        let key = match respell {
            true => key,
            false => self.stored_key(&key).unwrap_or(key),
        };
        if self.changed.is_empty()
            && let Some(listed) = Arc::get_mut(&mut self.listed)
        {
            // Nothing shares the map, as while it is being read: change it in place.
            listed.remove(&key);
            if extensions != 0 {
                listed.insert(key, extensions);
            }
            return;
        }
        // Removed first: `insert` would keep the old spelling.
        self.changed.remove(&key);
        let listed = self.listed.get_key_value(&key);
        let as_listed = listed
            .is_some_and(|(stored, &listed)| listed == extensions && stored.name == key.name)
            || (listed.is_none() && extensions == 0);
        if !as_listed {
            self.changed.insert(key, extensions);
        }
        if self.changed.len() > MIN_PENDING_CHANGES.max(self.listed.len() / 16) {
            self.merge();
        }
    }

    /// Moves the pending changes into the listed map, copying it if it is shared.
    fn merge(&mut self) {
        let listed = Arc::make_mut(&mut self.listed);
        for (key, extensions) in self.changed.drain() {
            listed.remove(&key);
            if extensions != 0 {
                listed.insert(key, extensions);
            }
        }
    }

    /// Adds the page at `path`, if it is named like one. Returns whether the index changed.
    pub fn insert(&mut self, path: &Path) -> bool {
        let (Some(key), Some(rank)) = (stem_key(path), page_rank(path)) else {
            return false;
        };
        if !is_page_path(path) {
            return false;
        }
        let extensions = self.extensions(&key);
        if extensions & (1 << rank) != 0 {
            return false;
        }
        let extensions = extensions | (1 << rank);
        // The preferred file names the page: `Ideas.md` rather than `ideas.txt`.
        let respell = extensions.trailing_zeros() as usize == rank;
        self.set_extensions(key, extensions, respell);
        true
    }

    /// Whether the index holds the page at `path` (its stem ignoring case, with its extension).
    pub fn contains(&self, path: &Path) -> bool {
        let (Some(key), Some(rank)) = (stem_key(path), page_rank(path)) else {
            return false;
        };
        is_page_path(path) && self.extensions(&key) & (1 << rank) != 0
    }

    /// Drops the page at `path`. Returns whether the index changed.
    pub fn remove(&mut self, path: &Path) -> bool {
        if !self.contains(path) {
            return false;
        }
        let (Some(key), Some(rank)) = (stem_key(path), page_rank(path)) else {
            return false;
        };
        let extensions = self.extensions(&key) & !(1 << rank);
        self.set_extensions(key, extensions, false);
        true
    }

    /// Whether a file of stem `page` ignoring case exists, as [`resolve`](Self::resolve) would find. A hash map
    /// lookup or two.
    pub fn has_page(&self, page: &str) -> bool {
        self.extensions(&PageKey::page(page)) != 0
    }

    /// The file the page `page` names: of the files whose stem equals it ignoring case, the `.md` one, else the
    /// `.txt` one, spelled as the page's newest file is. `page` is not checked: only stems of files in the
    /// folder are found, so a path or `..` names nothing, but the caller decides which names are links
    /// (`scratchpad_editor::markdown::wikilink_page`).
    pub fn resolve(&self, page: &str) -> Option<PathBuf> {
        let key = PageKey::page(page);
        let (stem, extensions) = match self.changed.get_key_value(&key) {
            Some((stem, &extensions)) => (stem, extensions),
            None => self
                .listed
                .get_key_value(&key)
                .map(|(stem, &extensions)| (stem, extensions))?,
        };
        let rank = (0..PAGE_EXTENSIONS.len()).find(|rank| extensions & (1 << rank) != 0)?;
        let extension = PAGE_EXTENSIONS[rank];
        if stem
            .extension()
            .is_some_and(|spelled| spelled.eq_ignore_ascii_case(extension))
        {
            return Some(self.dir.join(&*stem.name));
        }
        Some(self.dir.join(format!("{}.{extension}", stem.stem())))
    }

    /// How many page names the index holds.
    pub fn len(&self) -> usize {
        let listed = self.listed.len();
        self.changed.iter().fold(listed, |len, (key, &extensions)| {
            match (self.listed.contains_key(key), extensions != 0) {
                (false, true) => len + 1,
                (true, false) => len - 1,
                _ => len,
            }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn stem_key(path: &Path) -> Option<PageKey> {
    let name = path.file_name()?.to_string_lossy();
    let stem_len = name.rfind('.').unwrap_or(name.len());
    Some(PageKey {
        name: name.into(),
        stem_len,
    })
}
