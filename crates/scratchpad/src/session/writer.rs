//! The session's file operations, run one at a time in the order they were asked for (ADR 0060).
//!
//! Loading, saving, checking the open note against the disk, renaming and recovery snapshots
//! all go through one queue. Each job sees the effects of every job before it, so an older save
//! can never land after a newer one, a load never reads a file that a queued save is about to
//! replace, and a check of the disk always compares against the text we saved last.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use scratchpad_core::{Note, NoteStore, NoteText, RecoveryStore};
use scratchpad_editor::TextSnapshot;

#[derive(Clone, Debug)]
pub(super) enum Job {
    /// Read a note to show it.
    Load(PathBuf),
    /// Read the open note to see whether another program changed it.
    Check(PathBuf),
    /// Write `text` to the note, unless the file no longer holds `expected` (what we last read
    /// or wrote): then another program changed it since and nothing is written.
    Save {
        path: PathBuf,
        text: SaveText,
        expected: Option<SaveText>,
    },
    /// Rename the note after its title. Runs on the UI thread, through the notes model.
    Rename {
        path: PathBuf,
        title: String,
    },
    /// Keep unsaved text in the recovery folder; `key` is the note's path or a draft's key.
    Snapshot {
        key: PathBuf,
        text: TextSnapshot,
    },
    RemoveSnapshot(PathBuf),
    /// Delete a file a save recreated after the note was renamed or deleted, if it still holds
    /// exactly what that save wrote.
    RemoveIfUnchanged {
        path: PathBuf,
        text: Arc<str>,
    },
}

/// The text of a save, or what a save expects the file to hold.
///
/// The session takes the editor's text as a [`TextSnapshot`], which copies nothing, and the
/// writer turns it into a string on the background thread, once: a large note's save costs the
/// UI thread nothing (ADR 0111). What the session needs afterwards (the text on disk) has been
/// serialized by then.
#[derive(Clone, Debug)]
pub(super) enum SaveText {
    Text(Arc<str>),
    Snapshot(Arc<(TextSnapshot, OnceLock<Arc<str>>)>),
}

impl SaveText {
    pub fn snapshot(snapshot: TextSnapshot) -> Self {
        Self::Snapshot(Arc::new((snapshot, OnceLock::new())))
    }

    /// The text, serialized now if nothing needed it before.
    pub fn get(&self) -> &Arc<str> {
        match self {
            SaveText::Text(text) => text,
            SaveText::Snapshot(snapshot) => snapshot.1.get_or_init(|| snapshot.0.to_text().into()),
        }
    }
}

impl From<Arc<str>> for SaveText {
    fn from(text: Arc<str>) -> Self {
        Self::Text(text)
    }
}

impl Job {
    fn path(&self) -> &Path {
        match self {
            Job::Load(path)
            | Job::Check(path)
            | Job::Save { path, .. }
            | Job::Rename { path, .. }
            | Job::RemoveIfUnchanged { path, .. }
            | Job::Snapshot { key: path, .. }
            | Job::RemoveSnapshot(path) => path,
        }
    }

    fn path_mut(&mut self) -> &mut PathBuf {
        match self {
            Job::Load(path)
            | Job::Check(path)
            | Job::Save { path, .. }
            | Job::Rename { path, .. }
            | Job::RemoveIfUnchanged { path, .. }
            | Job::Snapshot { key: path, .. }
            | Job::RemoveSnapshot(path) => path,
        }
    }

    /// Whether this job can take the place of `queued`: it does the same thing to the same
    /// file with newer content.
    fn replaces(&self, queued: &Job) -> bool {
        let same_kind = matches!(
            (self, queued),
            (Job::Check(_), Job::Check(_))
                | (Job::Save { .. }, Job::Save { .. })
                | (Job::Snapshot { .. }, Job::Snapshot { .. })
        );
        same_kind && self.path() == queued.path()
    }
}

/// What a job found or did.
pub(super) enum Outcome {
    Loaded(scratchpad_core::Result<NoteText>),
    /// The note's content, or `None` if it no longer exists.
    Checked(io::Result<Option<NoteText>>),
    Saved(scratchpad_core::Result<Note>),
    /// Another program changed (`Some`) or deleted (`None`) the note since we last read or
    /// wrote it, so the save was not done. The text is in a recovery snapshot.
    ChangedOnDisk(Option<NoteText>),
    Done,
    /// A synchronous flush already wrote newer content (see [`Writer::flush_marker`]).
    Skipped,
}

/// A job that has been started and not finished yet.
pub(super) struct Running {
    pub job: Job,
    /// The note the job saved to was renamed (to this path) or deleted while it ran, so the
    /// save may have recreated the old file.
    pub moved: Option<Moved>,
    /// Its place in the order of started jobs (see [`Writer::flush_marker`]).
    pub number: u64,
}

impl Running {
    /// Where the note this job saves to is now: `None` if it is not a save, or the note was
    /// deleted.
    pub fn saving_to(&self) -> Option<&Path> {
        match (&self.job, &self.moved) {
            (Job::Save { path, .. }, None) => Some(path),
            (Job::Save { .. }, Some(Moved::Renamed(to))) => Some(to),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Moved {
    Renamed(PathBuf),
    Deleted,
}

#[derive(Default)]
pub(super) struct Writer {
    queue: VecDeque<Job>,
    pub running: Option<Running>,
    /// Whether a task is working through the queue.
    pub busy: bool,
    /// Counts started jobs; shared with background jobs through [`Self::flush_marker`].
    started: u64,
    /// The number of the last job whose effects are on disk. A synchronous flush at exit sets
    /// it past every job, so a background save that has not run yet will skip itself instead
    /// of overwriting newer text with older.
    flushed: Arc<Mutex<u64>>,
}

impl Writer {
    /// Queues `job`. A queued save, snapshot or check of the same file is updated in place
    /// (so a save stays before a rename queued after it); only the latest of several notes
    /// opened in quick succession is loaded, after everything queued before it.
    pub fn push(&mut self, mut job: Job) {
        // A newer snapshot must not take the place of one queued before this removal.
        if let Job::RemoveSnapshot(key) = &job {
            self.queue.retain(
                |queued| !matches!(queued, Job::Snapshot { key: snapshot, .. } if snapshot == key),
            );
        }
        if matches!(job, Job::Load(_)) {
            self.queue.retain(|queued| !matches!(queued, Job::Load(_)));
        } else if let Some(queued) = self.queue.iter_mut().find(|queued| job.replaces(queued)) {
            // The file still holds what it held before the save being replaced.
            if let (
                Job::Save { expected, .. },
                Job::Save {
                    expected: before, ..
                },
            ) = (&mut job, &*queued)
            {
                *expected = before.clone();
            }
            *queued = job;
            return;
        }
        self.queue.push_back(job);
    }

    /// The text of the latest save of `path` that has not finished: what the file will hold
    /// when a save queued now runs.
    pub fn pending_save(&self, path: &Path) -> Option<SaveText> {
        let running = self.running.as_ref().map(|running| &running.job);
        self.queue
            .iter()
            .rev()
            .chain(running)
            .find_map(|job| match job {
                Job::Save { path: p, text, .. } if p == path => Some(text.clone()),
                _ => None,
            })
    }

    /// Makes the queued saves of `path` expect the file to hold `holds` (`None`: write over
    /// whatever it holds), e.g. what a failed save before them expected.
    pub fn set_expected(&mut self, path: &Path, holds: Option<SaveText>) {
        for job in &mut self.queue {
            if let Job::Save {
                path: p, expected, ..
            } = job
                && p == path
            {
                *expected = holds.clone();
            }
        }
    }

    pub fn pop(&mut self) -> Option<Job> {
        self.queue.pop_front()
    }

    /// Removes the queued jobs for `path` that `keep` rejects.
    pub fn retain_for(&mut self, path: &Path, mut keep: impl FnMut(&Job) -> bool) {
        self.queue.retain(|job| job.path() != path || keep(job));
    }

    pub fn has_save_for(&self, path: &Path) -> bool {
        self.queue
            .iter()
            .any(|job| matches!(job, Job::Save { path: p, .. } if p == path))
    }

    /// Points queued jobs for `from` at `to` after a rename.
    pub fn retarget(&mut self, from: &Path, to: &Path) {
        retarget(self.queue.iter_mut(), from, to);
    }

    /// Every job that has not finished, in order, for a synchronous flush, which takes over the
    /// running job: its outcome is ignored when it arrives, as it may be older than what the
    /// flush did. The running job comes first, unless it would recreate a moved note or has
    /// already run (`flushed` is the flush marker's value): a save run twice would find its own
    /// text and take it for a change by another program.
    pub fn take_unfinished(&mut self, flushed: u64) -> Vec<Job> {
        let running = self
            .running
            .take()
            .filter(|running| running.moved.is_none() && running.number > flushed)
            .map(|running| running.job);
        running.into_iter().chain(self.queue.drain(..)).collect()
    }

    /// The number the next started job gets.
    pub fn next_number(&mut self) -> u64 {
        self.started += 1;
        self.started
    }

    pub fn flush_marker(&self) -> Arc<Mutex<u64>> {
        self.flushed.clone()
    }
}

pub(super) fn retarget<'a>(jobs: impl Iterator<Item = &'a mut Job>, from: &Path, to: &Path) {
    for job in jobs {
        if job.path() == from && !matches!(job, Job::RemoveIfUnchanged { .. }) {
            *job.path_mut() = to.to_owned();
        }
    }
}

/// Runs a job on a background thread, unless a synchronous flush got there first.
pub(super) fn run_in_order(
    job: &Job,
    number: u64,
    flushed: &Mutex<u64>,
    store: Option<&NoteStore>,
    recovery: Option<&RecoveryStore>,
) -> Outcome {
    let mut flushed = flushed.lock().unwrap_or_else(PoisonError::into_inner);
    if *flushed >= number {
        return Outcome::Skipped;
    }
    *flushed = number;
    run(job, store, recovery)
}

/// Does the file work of `job`. Renames are not file work here (see [`Job::Rename`]).
pub(super) fn run(
    job: &Job,
    store: Option<&NoteStore>,
    recovery: Option<&RecoveryStore>,
) -> Outcome {
    match job {
        Job::Load(path) => Outcome::Loaded(store_or_error(store, path).and_then(|s| s.read(path))),
        Job::Check(path) => Outcome::Checked(check(store, path)),
        Job::Save {
            path,
            text,
            expected,
        } => {
            let text = text.get();
            if let Some(expected) = expected {
                match check(store, path) {
                    Ok(Some(disk)) if *disk.text == **expected.get() => {}
                    Ok(disk) => {
                        if let Some(recovery) = recovery {
                            log_recovery_error(recovery.write(path, text));
                        }
                        return Outcome::ChangedOnDisk(disk);
                    }
                    // Unreadable: the save will most likely fail and report why.
                    Err(_) => {}
                }
            }
            let saved = store_or_error(store, path)
                .and_then(|store| store.save(path, text).and(store.note(path)));
            if let Some(recovery) = recovery {
                // Unsaved text must survive a crash; saved text needs no snapshot.
                let result = match &saved {
                    Ok(_) => recovery.remove(path),
                    Err(_) => recovery.write(path, text),
                };
                log_recovery_error(result);
            }
            Outcome::Saved(saved)
        }
        Job::Snapshot { key, text } => {
            if let Some(recovery) = recovery {
                log_recovery_error(recovery.write(key, &text.to_text()));
            }
            Outcome::Done
        }
        Job::RemoveSnapshot(key) => {
            if let Some(recovery) = recovery {
                log_recovery_error(recovery.remove(key));
            }
            Outcome::Done
        }
        Job::RemoveIfUnchanged { path, text } => {
            let recreated = fs::read(path).is_ok_and(|bytes| bytes == text.as_bytes());
            if recreated && let Err(error) = fs::remove_file(path) {
                tracing::warn!(path = %path.display(), %error, "could not remove a recreated note");
            }
            Outcome::Done
        }
        Job::Rename { .. } => Outcome::Done,
    }
}

fn check(store: Option<&NoteStore>, path: &Path) -> io::Result<Option<NoteText>> {
    match store_or_error(store, path).and_then(|store| store.read(path)) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.io_kind() == io::ErrorKind::NotFound => Ok(None),
        Err(scratchpad_core::Error::Io { source, .. }) => Err(source),
    }
}

/// The notes model opens its store before any note can be open, so this only fails if that
/// order is broken.
fn store_or_error<'a>(
    store: Option<&'a NoteStore>,
    path: &Path,
) -> scratchpad_core::Result<&'a NoteStore> {
    store.ok_or_else(|| scratchpad_core::Error::Io {
        action: "open",
        path: path.to_owned(),
        source: io::Error::other("the notes folder is not open"),
    })
}

fn log_recovery_error(result: scratchpad_core::Result<()>) {
    if let Err(error) = result {
        tracing::warn!(%error, "recovery snapshot failed");
    }
}

#[cfg(test)]
mod tests {
    use scratchpad_editor::Buffer;

    use super::*;

    fn snapshot(key: &str, text: &str) -> Job {
        Job::Snapshot {
            key: key.into(),
            text: Buffer::from_text(text).snapshot(),
        }
    }

    #[test]
    fn a_snapshot_queued_after_its_removal_is_written_after_it() {
        let mut writer = Writer::default();
        writer.push(snapshot("a", "old"));
        writer.push(Job::RemoveSnapshot("a".into()));
        writer.push(snapshot("a", "new"));

        let jobs: Vec<_> = std::iter::from_fn(|| writer.pop()).collect();
        assert!(
            matches!(&jobs[..], [Job::RemoveSnapshot(_), Job::Snapshot { text, .. }] if text.to_text() == "new"),
            "{jobs:?}"
        );
    }
}
