//! Crash recovery snapshots of unsaved note text (PLAN §40).
//!
//! Several instances of the app may run at once and share the recovery folder (ADR 0155). Each
//! keeps its snapshots in a folder of its own, `<id>/`, next to a lock file `<id>.lock` that it
//! holds open while it runs. A folder whose lock nobody holds belongs to an instance that is
//! gone, however it ended, and only such snapshots are leftovers.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::atomic::{remove_stale_temp_files, write_atomic};
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
/// path). At startup, [`leftovers`](Self::leftovers) takes over the snapshots of instances that
/// are no longer running (a crash) and returns those that differ from their note, to offer.
///
/// Clones share this instance's folder and lock; the lock is released when the last is dropped.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    /// The recovery folder shared by every instance.
    root: PathBuf,
    /// This instance's folder, claimed when it is first needed.
    instance: Arc<Mutex<Option<Instance>>>,
}

#[derive(Debug)]
struct Instance {
    dir: PathBuf,
    /// Held open while the instance runs; the OS closes it when the process ends, crash or not.
    _lock: File,
}

impl RecoveryStore {
    /// A store in `dir`, which is created on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        RecoveryStore {
            root: dir.into(),
            instance: Arc::default(),
        }
    }

    /// `<platform local data dir>/Scratchpad/recovery`, e.g.
    /// `%LOCALAPPDATA%\Scratchpad\recovery`. `None` when the platform has no such directory.
    pub fn default_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|dir| dir.join("Scratchpad").join("recovery"))
    }

    /// Stores `text` as the recovery snapshot of `note_path`, replacing any earlier one.
    pub fn write(&self, note_path: &Path, text: &str) -> Result<()> {
        /// [`Snapshot`] with borrowed fields, so a large note's text is not copied once more.
        #[derive(Serialize)]
        struct SnapshotRef<'a> {
            note_path: &'a Path,
            text: &'a str,
            saved_at: SystemTime,
        }
        let snapshot = SnapshotRef {
            note_path,
            text,
            saved_at: SystemTime::now(),
        };
        let json = serde_json::to_vec(&snapshot)
            .map_err(io::Error::other)
            .map_err(io_context("encode recovery snapshot", note_path))?;
        let dir = self
            .own_dir()
            // Made again if something removed it: only its lock file is held open.
            .and_then(|dir| fs::create_dir_all(&dir).map(|()| dir))
            .map_err(io_context("create recovery folder", &self.root))?;
        let file = dir.join(file_name(note_path));
        write_atomic(&file, &json).map_err(io_context("write recovery snapshot", &file))?;
        tracing::debug!(note = %note_path.display(), "wrote recovery snapshot");
        Ok(())
    }

    /// Deletes this instance's snapshot of `note_path`. Succeeds when there is none.
    pub fn remove(&self, note_path: &Path) -> Result<()> {
        let instance = self.instance.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(instance) = &*instance else {
            return Ok(());
        };
        let file = instance.dir.join(file_name(note_path));
        match fs::remove_file(&file) {
            Ok(()) => {
                tracing::debug!(note = %note_path.display(), "removed recovery snapshot");
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_context("remove recovery snapshot", &file)(error)),
        }
    }

    /// Every readable snapshot in the recovery folder, of any instance, newest first. Corrupt
    /// files (a crash can interrupt anything) are skipped with a warning and left in place.
    pub fn list(&self) -> Vec<Snapshot> {
        let mut files = snapshot_files(&self.root);
        for entry in entries(&self.root) {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                files.extend(snapshot_files(&entry.path()));
            }
        }
        let mut snapshots: Vec<Snapshot> = files.iter().filter_map(|file| read(file)).collect();
        snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.saved_at));
        snapshots
    }

    /// The unsaved work of instances that are no longer running, newest first.
    ///
    /// Their snapshots move into this instance's folder, so [`remove`](Self::remove) discards
    /// them and no other instance offers them as well; their emptied folders go. Snapshots
    /// whose text equals their note were saved after all and are removed; a note that is
    /// missing or not valid UTF-8 counts as different. Snapshots in the recovery folder itself
    /// were left by versions without instance folders and count as left behind too.
    pub fn leftovers(&self) -> Vec<Snapshot> {
        let mut leftovers = Vec::new();
        let mut instances = BTreeSet::new();
        for entry in entries(&self.root) {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                instances.insert(path);
            } else if path.extension().is_some_and(|ext| ext == "lock") {
                instances.insert(path.with_extension(""));
            } else if is_snapshot_file(&path) {
                // No lock guards these. On Windows a rename follows the file it opened, so two
                // instances starting at the same moment may both take (and offer) one.
                leftovers.extend(self.take_over(&path));
            }
        }
        for dir in instances {
            leftovers.extend(self.take_over_instance(&dir));
        }
        leftovers.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.saved_at));
        leftovers
    }

    /// The snapshots in another instance's folder `dir`, unless that instance is running (this
    /// one included). The folder goes once it is empty, then its lock file.
    fn take_over_instance(&self, dir: &Path) -> Vec<Snapshot> {
        let lock_path = dir.with_extension("lock");
        // Creates the lock of a folder that has none (it can only be left over), to clean up.
        let lock = match open_lock(File::options().write(true).create(true), &lock_path) {
            Ok(Some(lock)) => lock,
            Ok(None) => return Vec::new(),
            Err(error) => {
                tracing::warn!(lock = %lock_path.display(), %error, "cannot check recovery folder");
                return Vec::new();
            }
        };
        let leftovers = snapshot_files(dir)
            .iter()
            .filter_map(|file| self.take_over(file))
            .collect();
        remove_stale_temp_files(dir);
        match fs::remove_dir(dir) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            // It still holds snapshots kept for a later run (see `take_over`), or corrupt ones.
            Err(_) => return leftovers,
        }
        drop(lock);
        // Fails harmlessly when another instance has just opened it; that one removes it.
        let _ = fs::remove_file(&lock_path);
        leftovers
    }

    /// The snapshot in `file`, moved into this instance's folder, unless its text was saved.
    fn take_over(&self, file: &Path) -> Option<Snapshot> {
        let snapshot = read(file)?;
        let saved = read_note(&snapshot.note_path)
            .is_ok_and(|note| !note.lossy && note.text == snapshot.text);
        if saved {
            let _ = fs::remove_file(file);
            return None;
        }
        let dir = self
            .own_dir()
            .map_err(|error| tracing::warn!(%error, "cannot create recovery folder"))
            .ok()?;
        let target = dir.join(file.file_name()?);
        // This instance already holds a snapshot of that note (its own, or one taken over just
        // now): this one stays where it is, to be offered on the next start.
        if target.exists() {
            return None;
        }
        match fs::rename(file, &target) {
            Ok(()) => Some(snapshot),
            // Another instance took it first.
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!(file = %file.display(), %error, "cannot take over recovery snapshot");
                None
            }
        }
    }

    /// This instance's folder, claimed with its lock on first use.
    fn own_dir(&self) -> io::Result<PathBuf> {
        let mut instance = self.instance.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(instance) = &*instance {
            return Ok(instance.dir.clone());
        }
        let claimed = Instance::claim(&self.root)?;
        let dir = claimed.dir.clone();
        *instance = Some(claimed);
        Ok(dir)
    }
}

impl Instance {
    /// A new folder in `root` with its lock held. The lock file comes first and is locked as it
    /// is created, so a folder whose lock is free never belongs to an instance setting up.
    fn claim(root: &Path) -> io::Result<Instance> {
        fs::create_dir_all(root)?;
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100 {
            let id = format!("{}-{started:x}-{attempt}", std::process::id());
            let lock_path = root.join(format!("{id}.lock"));
            match open_lock(File::options().write(true).create_new(true), &lock_path) {
                Ok(Some(lock)) => {
                    let dir = root.join(id);
                    fs::create_dir_all(&dir)?;
                    return Ok(Instance { dir, _lock: lock });
                }
                // Taken by a store made at the same moment, or (not on Windows) by a scan.
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("no free name for a recovery folder"))
    }
}

/// Opens the lock file at `path` so that no other instance can while it stays open, or `None`
/// when another (running) instance holds it.
#[cfg(windows)]
fn open_lock(options: &mut OpenOptions, path: &Path) -> io::Result<Option<File>> {
    use std::os::windows::fs::OpenOptionsExt;
    /// `ERROR_SHARING_VIOLATION`: someone else has the file open.
    const SHARING_VIOLATION: i32 = 32;
    match options.share_mode(0).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.raw_os_error() == Some(SHARING_VIOLATION) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Best effort elsewhere: an advisory lock taken just after a new lock file is created, so a
/// scan in that instant can take a starting instance for one that is gone.
#[cfg(not(windows))]
fn open_lock(options: &mut OpenOptions, path: &Path) -> io::Result<Option<File>> {
    let file = options.open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(error)) => Err(error),
    }
}

/// The entries of `dir`; none when it does not exist or cannot be read.
fn entries(dir: &Path) -> Vec<fs::DirEntry> {
    match fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|entry| entry.ok()).collect(),
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(dir = %dir.display(), %error, "cannot list recovery snapshots");
            }
            Vec::new()
        }
    }
}

fn snapshot_files(dir: &Path) -> Vec<PathBuf> {
    entries(dir)
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| is_snapshot_file(path))
        .collect()
}

fn is_snapshot_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json") && path.is_file()
}

/// The snapshot in `file`, or `None` when it cannot be read or is corrupt.
fn read(file: &Path) -> Option<Snapshot> {
    let bytes = fs::read(file).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            tracing::warn!(path = %file.display(), %error, "ignoring corrupt recovery snapshot");
            None
        }
    }
}

/// The snapshot file name of `note_path`: a stable hash of the path. `DefaultHasher` makes no
/// stability promise across Rust versions, and a changed name would orphan existing snapshots.
fn file_name(note_path: &Path) -> String {
    let hash = note_path
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{hash:016x}.json")
}
