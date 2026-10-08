//! Filesystem watching of the notes folder (PLAN §30).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::{Result, io_context};
use crate::note::is_note_path;

/// A change to a `*.md` file in the watched folder.
///
/// Renames arrive as `Removed(old)` followed by `Created(new)`. Some writers (including our own
/// atomic save and many other editors) replace a file by renaming a temporary file over it, which
/// shows up as `Changed` and/or `Created` for a note that already existed. Treat `Created` and
/// `Changed` for an open note the same way. `Removed` is only reported for a note that no longer
/// exists when the event is processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteEvent {
    Created(PathBuf),
    Changed(PathBuf),
    Removed(PathBuf),
}

impl NoteEvent {
    pub fn path(&self) -> &Path {
        match self {
            NoteEvent::Created(path) | NoteEvent::Changed(path) | NoteEvent::Removed(path) => path,
        }
    }
}

/// Watches the notes folder (not recursively) and reports changes to notes. Dropping it stops
/// the watching.
///
/// Events are queued without blocking, so the app can poll [`try_recv`](Self::try_recv) from a
/// timer on its executor. Hidden files and our own temporary files are never reported.
///
/// The watcher also reports the app's own saves. The app is expected to tell them apart from
/// external edits by comparing the file's content on disk with the text it last saved; if they
/// are equal there is nothing to reload. Events can arrive several times for one write.
pub struct NoteWatcher {
    // Keeps the OS watch alive; its callback feeds `events`.
    _watcher: RecommendedWatcher,
    events: Receiver<NoteEvent>,
}

impl NoteWatcher {
    pub fn start(dir: &Path) -> Result<Self> {
        let (sender, events) = mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                match result {
                    Ok(event) => {
                        for note_event in translate(event) {
                            tracing::debug!(?note_event, "filesystem event");
                            // The receiver is gone when the NoteWatcher was dropped.
                            let _ = sender.send(note_event);
                        }
                    }
                    Err(error) => tracing::warn!(%error, "filesystem watcher error"),
                }
            })
            .map_err(into_io_error)
            .map_err(io_context("watch", dir))?;
        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(into_io_error)
            .map_err(io_context("watch", dir))?;
        tracing::debug!(dir = %dir.display(), "watching notes folder");
        Ok(NoteWatcher {
            _watcher: watcher,
            events,
        })
    }

    /// The next queued event, if any, without waiting.
    pub fn try_recv(&self) -> Option<NoteEvent> {
        self.events.try_recv().ok()
    }

    /// The next event, waiting up to `timeout` for one to arrive.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<NoteEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

fn into_io_error(error: notify::Error) -> io::Error {
    match error.kind {
        notify::ErrorKind::Io(error) => error,
        _ => io::Error::other(error),
    }
}

fn translate(event: notify::Event) -> Vec<NoteEvent> {
    let notes = |paths: Vec<PathBuf>| paths.into_iter().filter(|path| is_note_path(path));
    match event.kind {
        EventKind::Create(_) => notes(event.paths).map(NoteEvent::Created).collect(),
        EventKind::Remove(_) => notes(event.paths).map(removal).collect(),
        EventKind::Modify(ModifyKind::Name(mode)) => match (mode, event.paths.as_slice()) {
            // Both paths come in one event; keep their roles before filtering.
            (RenameMode::Both, [from, to]) => {
                let removed = is_note_path(from).then(|| removal(from.clone()));
                let created = is_note_path(to).then(|| NoteEvent::Created(to.clone()));
                removed.into_iter().chain(created).collect()
            }
            (RenameMode::From, _) => notes(event.paths).map(removal).collect(),
            (RenameMode::To, _) => notes(event.paths).map(NoteEvent::Created).collect(),
            // Backends that cannot say which side of the rename this is.
            _ => notes(event.paths)
                .map(|path| {
                    if path.exists() {
                        NoteEvent::Created(path)
                    } else {
                        NoteEvent::Removed(path)
                    }
                })
                .collect(),
        },
        EventKind::Modify(_) => notes(event.paths).map(NoteEvent::Changed).collect(),
        EventKind::Access(_) | EventKind::Any | EventKind::Other => Vec::new(),
    }
}

/// Windows reports a file replaced by a rename (every atomic save) as removed first, although
/// it never stops existing. Only a note that is really gone counts as removed.
fn removal(path: PathBuf) -> NoteEvent {
    if path.exists() {
        NoteEvent::Changed(path)
    } else {
        NoteEvent::Removed(path)
    }
}

#[cfg(test)]
mod tests {
    use notify::Event;
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind};

    use super::*;

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        paths.iter().fold(Event::new(kind), |event, path| {
            event.add_path(PathBuf::from(path))
        })
    }

    #[test]
    fn rename_both_keeps_the_role_of_each_side() {
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        assert_eq!(
            translate(event(both, &["dir/a.md", "dir/b.md"])),
            [
                NoteEvent::Removed("dir/a.md".into()),
                NoteEvent::Created("dir/b.md".into())
            ]
        );
        // Renaming a note to a non-note name is a removal; the other way round a creation.
        assert_eq!(
            translate(event(both, &["dir/a.md", "dir/a.md.bak"])),
            [NoteEvent::Removed("dir/a.md".into())]
        );
        assert_eq!(
            translate(event(both, &["dir/draft.txt", "dir/draft.md"])),
            [NoteEvent::Created("dir/draft.md".into())]
        );
    }

    #[test]
    fn plain_kinds_map_to_their_event_and_filter_non_notes() {
        let created = EventKind::Create(CreateKind::File);
        let removed = EventKind::Remove(RemoveKind::File);
        let changed = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        let paths = [
            "d/a.md",
            "d/b.txt",
            "d/.c.md",
            "d/.scratchpad-1-0.tmp",
            "d/E.MD",
        ];
        assert_eq!(
            translate(event(created, &paths)),
            [
                NoteEvent::Created("d/a.md".into()),
                NoteEvent::Created("d/E.MD".into())
            ]
        );
        assert_eq!(
            translate(event(removed, &["d/a.md"])),
            [NoteEvent::Removed("d/a.md".into())]
        );
        assert_eq!(
            translate(event(changed, &["d/a.md"])),
            [NoteEvent::Changed("d/a.md".into())]
        );
        assert!(
            translate(event(
                EventKind::Access(notify::event::AccessKind::Any),
                &["d/a.md"]
            ))
            .is_empty()
        );
    }
}
