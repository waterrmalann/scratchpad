#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use scratchpad_core::NoteStore;
use tempfile::TempDir;

/// Stands in for the recycle bin: moves the file into a hidden `.trash` folder next to it.
pub fn move_to_test_trash(path: &Path) -> io::Result<()> {
    let trash = path.parent().unwrap().join(".trash");
    fs::create_dir_all(&trash)?;
    fs::rename(path, trash.join(path.file_name().unwrap()))
}

/// A store in a fresh temp directory that never touches the real recycle bin.
pub fn temp_store() -> (TempDir, NoteStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = NoteStore::with_deleter(dir.path(), move_to_test_trash).unwrap();
    (dir, store)
}

/// A time `secs` after the Unix epoch, for deterministic modification times.
pub fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

pub fn set_modified(path: &Path, time: SystemTime) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// Names of everything in `dir`, sorted, including hidden files.
pub fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

pub fn path_in(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}
