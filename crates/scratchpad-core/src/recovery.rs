//! Crash recovery snapshots of unsaved note text (PLAN §40).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::error::{Result, io_context};
use crate::store::read_note;

/// Unsaved text of an open note, as last written by [`RecoveryStore::write`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub note_path: PathBuf,
    pub text: String,
    pub saved_at: SystemTime,
}

/// Keeps one snapshot file per note in a folder that is *not* the notes folder, so recovery data
/// never shows up next to the user's Markdown files.
///
/// Intended flow: the app writes a snapshot periodically while a note has unsaved changes and
/// removes it after a successful save (and when a note is renamed or deleted, under its old
/// path). Snapshots still present at startup are leftovers of a crash; the app compares each
/// with the note on disk and offers to restore the ones that differ.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    dir: PathBuf,
}

impl RecoveryStore {
    /// A store in `dir`, which is created on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        RecoveryStore { dir: dir.into() }
    }

    /// `<platform local data dir>/Scratchpad/recovery`, e.g.
    /// `%LOCALAPPDATA%\Scratchpad\recovery`. `None` when the platform has no such directory.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|dir| dir.join("Scratchpad").join("recovery"))
    }

    /// Stores `text` as the recovery snapshot of `note_path`, replacing any earlier one.
    pub fn write(&self, note_path: &Path, text: &str) -> Result<()> {
        let snapshot = Snapshot {
            note_path: note_path.to_owned(),
            text: text.to_owned(),
            saved_at: SystemTime::now(),
        };
        let file = self.file_for(note_path);
        let json = serde_json::to_vec(&snapshot)
            .map_err(io::Error::other)
            .map_err(io_context("encode recovery snapshot", &file))?;
        fs::create_dir_all(&self.dir).map_err(io_context("create recovery folder", &self.dir))?;
        write_atomic(&file, &json).map_err(io_context("write recovery snapshot", &file))?;
        tracing::debug!(note = %note_path.display(), "wrote recovery snapshot");
        Ok(())
    }

    /// Deletes the snapshot of `note_path`. Succeeds when there is none.
    pub fn remove(&self, note_path: &Path) -> Result<()> {
        let file = self.file_for(note_path);
        match fs::remove_file(&file) {
            Ok(()) => {
                tracing::debug!(note = %note_path.display(), "removed recovery snapshot");
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_context("remove recovery snapshot", &file)(error)),
        }
    }

    /// All readable snapshots, newest first. Corrupt files (a crash can interrupt anything) are
    /// skipped with a warning and left in place.
    pub fn list(&self) -> Vec<Snapshot> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(dir = %self.dir.display(), %error, "cannot list recovery snapshots");
                }
                return Vec::new();
            }
        };
        let mut snapshots: Vec<Snapshot> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .filter_map(|path| {
                let bytes = fs::read(&path).ok()?;
                match serde_json::from_slice(&bytes) {
                    Ok(snapshot) => Some(snapshot),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "ignoring corrupt recovery snapshot");
                        None
                    }
                }
            })
            .collect();
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.saved_at));
        snapshots
    }

    /// Snapshots written before `before` (the start of this run) that hold text the note on
    /// disk does not: the unsaved work a crash left behind, newest first. Snapshots whose text
    /// equals their note were saved after all and are removed; a note that is missing or
    /// unreadable counts as different.
    pub fn leftovers(&self, before: SystemTime) -> Vec<Snapshot> {
        self.list()
            .into_iter()
            .filter(|snapshot| snapshot.saved_at < before)
            .filter(|snapshot| {
                let saved = read_note(&snapshot.note_path)
                    .is_ok_and(|note| !note.lossy && note.text == snapshot.text);
                if saved {
                    let _ = self.remove(&snapshot.note_path);
                }
                !saved
            })
            .collect()
    }

    fn file_for(&self, note_path: &Path) -> PathBuf {
        self.dir.join(format!("{:016x}.json", fnv1a(note_path)))
    }
}

/// A stable hash of the note path for the snapshot file name. `DefaultHasher` makes no stability
/// promise across Rust versions, and a changed name would orphan existing snapshots.
fn fnv1a(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}
