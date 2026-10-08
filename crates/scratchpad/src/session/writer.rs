//! The session's file operations, run one at a time in the order they were asked for (ADR 0060).
//!
//! Loading, saving and renaming all go through one queue. Each job sees the
//! effects of every job before it, so an older save can never land after a newer one and a load
//! never reads a file that a queued save is about to replace.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use scratchpad_core::{Note, NoteStore, NoteText};

#[derive(Clone, Debug)]
pub(super) enum Job {
    /// Read a note to show it.
    Load(PathBuf),
    Save {
        path: PathBuf,
        text: Arc<str>,
    },
    /// Rename the note after its title. Runs on the UI thread, through the notes model.
    Rename {
        path: PathBuf,
        title: String,
    },
    /// Delete a file a save recreated after the note was renamed or deleted, if it still holds
    /// exactly what that save wrote.
    RemoveIfUnchanged {
        path: PathBuf,
        text: Arc<str>,
    },
}

impl Job {
    fn path(&self) -> &Path {
        match self {
            Job::Load(path)
            | Job::Save { path, .. }
            | Job::Rename { path, .. }
            | Job::RemoveIfUnchanged { path, .. } => path,
        }
    }

    fn path_mut(&mut self) -> &mut PathBuf {
        match self {
            Job::Load(path)
            | Job::Save { path, .. }
            | Job::Rename { path, .. }
            | Job::RemoveIfUnchanged { path, .. } => path,
        }
    }

    /// Whether this job can take the place of `queued`: a save of the same file with newer
    /// content.
    fn replaces(&self, queued: &Job) -> bool {
        matches!((self, queued), (Job::Save { .. }, Job::Save { .. }))
            && self.path() == queued.path()
    }
}

/// What a job found or did.
pub(super) enum Outcome {
    Loaded(scratchpad_core::Result<NoteText>),
    Saved(scratchpad_core::Result<Note>),
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
    /// Queues `job`. A queued save of the same file is updated in place
    /// (so a save stays before a rename queued after it); only the latest of several notes
    /// opened in quick succession is loaded, after everything queued before it.
    pub fn push(&mut self, job: Job) {
        if matches!(job, Job::Load(_)) {
            self.queue.retain(|queued| !matches!(queued, Job::Load(_)));
        } else if let Some(queued) = self.queue.iter_mut().find(|queued| job.replaces(queued)) {
            *queued = job;
            return;
        }
        self.queue.push_back(job);
    }

    pub fn pop(&mut self) -> Option<Job> {
        self.queue.pop_front()
    }

    /// Removes the queued jobs for `path` that `keep` rejects.
    pub fn retain_for(&mut self, path: &Path, mut keep: impl FnMut(&Job) -> bool) {
        self.queue.retain(|job| job.path() != path || keep(job));
    }

    /// Points queued jobs for `from` at `to` after a rename.
    pub fn retarget(&mut self, from: &Path, to: &Path) {
        retarget(self.queue.iter_mut(), from, to);
    }

    /// Every job that has not finished, in order, for a synchronous flush: the running one
    /// first, unless it would recreate a moved note.
    pub fn take_unfinished(&mut self) -> Vec<Job> {
        let running = self
            .running
            .as_ref()
            .filter(|running| running.moved.is_none())
            .map(|running| running.job.clone());
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
) -> Outcome {
    let mut flushed = flushed.lock().unwrap_or_else(PoisonError::into_inner);
    if *flushed >= number {
        return Outcome::Skipped;
    }
    *flushed = number;
    run(job, store)
}

/// Does the file work of `job`. Renames are not file work here (see [`Job::Rename`]).
pub(super) fn run(job: &Job, store: Option<&NoteStore>) -> Outcome {
    match job {
        Job::Load(path) => Outcome::Loaded(store_or_error(store, path).and_then(|s| s.read(path))),
        Job::Save { path, text } => Outcome::Saved(
            store_or_error(store, path)
                .and_then(|store| store.save(path, text).and(store.note(path))),
        ),
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
