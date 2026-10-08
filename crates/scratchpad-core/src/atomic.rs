//! Crash-safe file replacement. See ADR 0010.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

/// Temporary files are `.scratchpad-<pid>-<n>.tmp`: hidden and not `.md`, so neither the sidebar
/// nor the watcher reports them, and short, so even a target name near the 255 character limit
/// leaves room for them.
const TEMP_PREFIX: &str = ".scratchpad-";
const TEMP_SUFFIX: &str = ".tmp";
/// A temporary file untouched for this long belongs to a crashed save, not a running one.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60);

/// Delays between attempts to rename over a target that another process briefly holds open.
const RENAME_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
];

/// Replaces the contents of `path` with `contents`, or leaves it untouched on failure.
///
/// The data goes to a temporary file in the same directory (same volume, so the rename cannot
/// degrade to copy + delete), is flushed to disk, and is then renamed over the target. A crash
/// or error at any point leaves either the old or the new file, never a truncated one.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "target is a directory",
        ));
    }
    let (mut file, temp_path) = create_temp_file(path)?;
    let result = (|| {
        file.write_all(contents)?;
        #[cfg(windows)]
        keep_creation_time(&file, path);
        file.sync_all()?;
        // Windows cannot rename an open file.
        drop(file);
        rename_over(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn create_temp_file(target: &Path) -> io::Result<(File, PathBuf)> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let dir = target.parent().unwrap_or(Path::new("."));
    // `create_new` turns a leftover from a crashed run with the same pid and counter into a
    // retry instead of an overwrite.
    for _ in 0..8 {
        let temp_path = dir.join(format!(
            "{TEMP_PREFIX}{}-{}{TEMP_SUFFIX}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

/// Replacing the file would otherwise reset its creation date ("Date created" in Explorer) on
/// every save. Best effort: failing to copy it is no reason to fail the save.
#[cfg(windows)]
fn keep_creation_time(file: &File, target: &Path) {
    use std::os::windows::fs::FileTimesExt;
    if let Ok(created) = fs::metadata(target).and_then(|metadata| metadata.created()) {
        let _ = file.set_times(fs::FileTimes::new().set_created(created));
    }
}

/// Deletes temporary files in `dir` that a crash left behind between creating and renaming
/// them. Recent ones are kept: another running instance may be in the middle of a save.
pub(crate) fn remove_stale_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX)) {
            continue;
        }
        let is_stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > STALE_TEMP_AGE));
        if is_stale {
            match fs::remove_file(entry.path()) {
                Ok(()) => tracing::info!(file = %name, "removed temporary file left by a crash"),
                Err(error) => {
                    tracing::warn!(file = %name, %error, "cannot remove stale temporary file")
                }
            }
        }
    }
}

fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    let mut delays = RENAME_RETRY_DELAYS.iter();
    loop {
        match fs::rename(from, to) {
            Err(e) if is_transient_rename_error(&e) => match delays.next() {
                Some(delay) => thread::sleep(*delay),
                None => return Err(e),
            },
            result => return result,
        }
    }
}

/// Antivirus scanners, search indexers and cloud-sync clients open files without
/// `FILE_SHARE_DELETE` for a few milliseconds, which fails the rename with one of these codes.
fn is_transient_rename_error(error: &io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    cfg!(windows)
        && matches!(
            error.raw_os_error(),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
        )
}
