use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::atomic::{remove_stale_temp_files, write_atomic};
use crate::error::{Result, io_context};
use crate::naming::{UNTITLED, sanitize_file_stem, unique_file_name};
use crate::note::{NOTE_EXTENSION, Note, is_note_path};

/// How often `create` retries when another process grabs the chosen name first.
const CREATE_ATTEMPTS: usize = 5;

/// The text of a note as read from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteText {
    pub text: String,
    /// The file was not valid UTF-8 and invalid bytes were replaced by U+FFFD. Saving this text
    /// back would destroy the original bytes, so the app should ask before overwriting.
    pub lossy: bool,
}

/// A folder of Markdown notes. The filesystem is the only source of truth: nothing is cached
/// here and every call goes to disk.
///
/// All `path` arguments are paths of notes in this store, as returned in [`Note::path`].
#[derive(Debug, Clone)]
pub struct NoteStore {
    dir: PathBuf,
    deleter: fn(&Path) -> io::Result<()>,
}

impl NoteStore {
    /// Opens the store rooted at `dir`, creating the folder if it does not exist, and removes
    /// temporary files an interrupted save left behind. Deleted notes go to the OS recycle bin.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        Self::with_deleter(dir, move_to_recycle_bin)
    }

    /// Like [`open`](Self::open) but with a custom way to delete files. Tests (including the
    /// app's) use this so they never put files into the real recycle bin.
    pub fn with_deleter(
        dir: impl Into<PathBuf>,
        deleter: fn(&Path) -> io::Result<()>,
    ) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(io_context("create notes folder", &dir))?;
        remove_stale_temp_files(&dir);
        Ok(NoteStore { dir, deleter })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// All notes, most recently modified first. Reads directory metadata only, never file
    /// contents, so it stays fast with thousands of notes (PLAN §39).
    ///
    /// Only `*.md` files directly in the folder are listed; hidden files and our own temporary
    /// files are skipped. Entries that cannot be inspected are skipped with a warning.
    pub fn list(&self) -> Result<Vec<Note>> {
        let entries = fs::read_dir(&self.dir).map_err(io_context("list notes in", &self.dir))?;
        let mut notes = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(%error, "skipping unreadable directory entry");
                    continue;
                }
            };
            let path = entry.path();
            if !is_note_path(&path) {
                continue;
            }
            match entry.metadata() {
                Ok(metadata) if metadata.is_file() && !is_hidden(&metadata) => {
                    notes.push(Note::new(path, &metadata));
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "skipping note without metadata");
                }
            }
        }
        notes.sort_by(|a, b| {
            b.modified_at
                .cmp(&a.modified_at)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.path.cmp(&b.path))
        });
        tracing::debug!(count = notes.len(), dir = %self.dir.display(), "listed notes");
        Ok(notes)
    }

    /// Metadata of a single note, e.g. to refresh its modification time after a save.
    pub fn note(&self, path: &Path) -> Result<Note> {
        let metadata = fs::metadata(path).map_err(io_context("inspect", path))?;
        Ok(Note::new(path.to_owned(), &metadata))
    }

    /// Reads a note. Never fails on malformed UTF-8: invalid bytes are replaced and
    /// [`NoteText::lossy`] is set. A leading UTF-8 byte order mark is dropped, so a note written
    /// by Notepad and saved again loses its BOM; Markdown tools do not need it.
    pub fn read(&self, path: &Path) -> Result<NoteText> {
        let note = read_note(path).map_err(io_context("read", path))?;
        if note.lossy {
            tracing::warn!(path = %path.display(), "note is not valid UTF-8, decoding lossily");
        }
        tracing::debug!(path = %path.display(), bytes = note.text.len(), "read note");
        Ok(note)
    }

    /// Creates an empty note named after `title` (sanitized), or `Untitled`. A name that is
    /// already taken, ignoring case, gets a numeric suffix: `Untitled 2.md`, `Untitled 3.md`...
    ///
    /// The file exists as soon as this returns, so the app should keep a new note in memory and
    /// call this once there is something to save (PLAN §9); otherwise every Ctrl+N leaves an
    /// empty `Untitled N.md` behind.
    pub fn create(&self, title: Option<&str>) -> Result<Note> {
        let stem = title
            .and_then(sanitize_file_stem)
            .unwrap_or_else(|| UNTITLED.to_owned());
        let mut attempts = 0;
        loop {
            let taken = self.taken_names(None)?;
            let path = self
                .dir
                .join(unique_file_name(&stem, NOTE_EXTENSION, &taken));
            // `create_new` makes the name reservation atomic, so two writers never share a file.
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => {
                    tracing::debug!(path = %path.display(), "created note");
                    return self.note(&path);
                }
                Err(e)
                    if e.kind() == io::ErrorKind::AlreadyExists && attempts < CREATE_ATTEMPTS =>
                {
                    attempts += 1;
                }
                Err(e) => return Err(io_context("create", &path)(e)),
            }
        }
    }

    /// Atomically replaces the note's contents (see ADR 0010). On error the previous file is
    /// untouched and no temporary file is left behind.
    ///
    /// A file watcher will report this write. The app suppresses its own writes by comparing
    /// the file's content with the text it last saved.
    pub fn save(&self, path: &Path, text: &str) -> Result<()> {
        write_atomic(path, text.as_bytes()).map_err(io_context("save", path))?;
        tracing::debug!(path = %path.display(), bytes = text.len(), "saved note");
        Ok(())
    }

    /// Renames the note after `new_title`, keeping its extension, and returns the new path. The
    /// title is sanitized and made unique like in [`create`](Self::create); a title that
    /// sanitizes to nothing becomes `Untitled`. Renaming a note to its own name is a no-op, and
    /// changing only the case works on case-insensitive filesystems.
    ///
    /// Another process creating the same name between the check and the rename would be
    /// overwritten; that window is accepted for a single-user notes folder.
    pub fn rename(&self, path: &Path, new_title: &str) -> Result<PathBuf> {
        let stem = sanitize_file_stem(new_title).unwrap_or_else(|| UNTITLED.to_owned());
        let extension = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        // The note itself must not count as a collision, or a case-only rename would be
        // pushed to "name 2".
        let taken = self.taken_names(path.file_name())?;
        let new_path = self.dir.join(unique_file_name(&stem, &extension, &taken));
        if new_path != path {
            fs::rename(path, &new_path).map_err(io_context("rename", path))?;
            tracing::debug!(from = %path.display(), to = %new_path.display(), "renamed note");
        }
        Ok(new_path)
    }

    /// Moves the note to the OS recycle bin (PLAN §41).
    pub fn delete(&self, path: &Path) -> Result<()> {
        (self.deleter)(path).map_err(io_context("delete", path))?;
        tracing::debug!(path = %path.display(), "deleted note");
        Ok(())
    }

    /// Lower-cased names of everything in the folder except `except`.
    fn taken_names(&self, except: Option<&OsStr>) -> Result<HashSet<String>> {
        let entries = fs::read_dir(&self.dir).map_err(io_context("list notes in", &self.dir))?;
        Ok(entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| Some(name.as_os_str()) != except)
            .map(|name| name.to_string_lossy().to_lowercase())
            .collect())
    }
}

/// Reads and decodes a note file as [`NoteStore::read`] does, without logging.
pub(crate) fn read_note(path: &Path) -> io::Result<NoteText> {
    let bytes = fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Ok(match std::str::from_utf8(bytes) {
        Ok(text) => NoteText {
            text: text.to_owned(),
            lossy: false,
        },
        Err(_) => NoteText {
            text: String::from_utf8_lossy(bytes).into_owned(),
            lossy: true,
        },
    })
}

fn move_to_recycle_bin(path: &Path) -> io::Result<()> {
    trash::delete(path).map_err(io::Error::other)
}

#[cfg(windows)]
fn is_hidden(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden(_metadata: &Metadata) -> bool {
    false
}
