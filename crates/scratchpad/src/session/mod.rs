//! The open note's lifecycle: loading it into the editor, autosave, giving new notes a file,
//! keeping file names in step with titles, changes made by other programs and crash recovery
//! (PLAN §7, §9, §30, §40, §44, §56). See ADRs 0060-0064 and 0066.
//!
//! [`Session`] follows the notes model's [`NotesEvent`]s and the editor's
//! [`EditorEvent::Changed`]. Every file operation goes through one ordered queue (see
//! [`writer`]), so the editor never waits for the disk and the disk never sees writes out of
//! order.

mod watch;
mod writer;

use std::collections::VecDeque;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{AppContext, Context, Entity, Focusable, Subscription, Task, Window};
use scratchpad_core::{
    Note, NoteEvent, NoteText, RecoveryStore, Snapshot, UNTITLED, title_from_content,
};
use scratchpad_editor::{Buffer, TextSnapshot};

use crate::app::Storage;
use crate::editor_view::{EditorEvent, EditorView};
use crate::notes::{DraftId, Notes, NotesEvent, title_of};
use crate::toast;
pub use watch::POLL_INTERVAL;
use writer::{Job, Moved, Outcome, Running, SaveText, Writer};

/// Quiet time after the last edit before the note is saved (PLAN §7).
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(300);
/// The longest a change waits for a save while the user keeps typing without pausing.
pub const MAX_AUTOSAVE_DELAY: Duration = Duration::from_secs(2);
/// While the user keeps typing, unsaved text goes to a recovery snapshot this often, so a crash
/// loses at most about this much typing; a pause saves it sooner (PLAN §40, ADR 0064).
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(500);
/// Quiet time after changes by other programs before the note list is re-read.
const REFRESH_DELAY: Duration = Duration::from_millis(250);
/// Titles are looked for this far into a note, so a keystroke never scans a huge document.
const TITLE_SEARCH_LINES: usize = 200;
/// Recovery snapshots of new notes that have no file yet are keyed by a path in the notes
/// folder starting with this; it is hidden and not `.md`, so it can never be a note.
const DRAFT_KEY_PREFIX: &str = ".draft-";

/// What the editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    None,
    /// Being read; the editor is read-only and still shows the previous note.
    Loading(PathBuf),
    /// A new note without a file; `key` names its recovery snapshot.
    Draft {
        id: DraftId,
        key: PathBuf,
    },
    Note(PathBuf),
}

/// The note in the editor and how it relates to its file.
struct Document {
    target: Target,
    /// Edits not yet handed to the writer (or whose save failed).
    dirty: bool,
    /// The note's content as we last read or wrote it, to tell other programs' changes from
    /// our own saves.
    disk_text: Arc<str>,
    /// The title as loaded or as last renamed to. The file is renamed only when the title line
    /// is changed (ADR 0061).
    title: Option<String>,
    notice: Option<DocumentNotice>,
    /// A recovery snapshot of this note may exist.
    snapshot: bool,
    /// The next save writes even if the file changed since we last read or wrote it: the user
    /// chose their version.
    overwrite: bool,
}

impl Document {
    fn new(target: Target) -> Self {
        Self {
            target,
            dirty: false,
            disk_text: Arc::from(""),
            title: None,
            notice: None,
            snapshot: false,
            overwrite: false,
        }
    }

    fn is_note(&self, path: &Path) -> bool {
        matches!(&self.target, Target::Note(open) if open == path)
    }
}

/// Something about the open note that needs a decision before it is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DocumentNotice {
    /// Not valid UTF-8: read-only until the user agrees to replace the unreadable bytes.
    NotUtf8,
    /// Another program changed the file while there were unsaved edits. `lossy`: their
    /// version is not valid UTF-8.
    Conflict { disk: Arc<str>, lossy: bool },
    /// Another program deleted the file.
    Deleted,
}

/// What the notice bar above the editor shows, most urgent first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    NotUtf8,
    ChangedOnDisk,
    DeletedOnDisk,
    /// Unsaved text from a run that crashed. `title` is the note's, or the new note's first
    /// line.
    Recovered {
        title: String,
        new_note: bool,
    },
}

/// The buttons of the notice bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    EditAnyway,
    KeepMine,
    LoadDisk,
    KeepDeleted,
    CloseDeleted,
    Restore,
    Discard,
}

/// Recovered text waiting for its note (or a new note) to open.
struct Restore {
    /// `None` restores into a new note.
    path: Option<PathBuf>,
    text: String,
    /// The snapshot it came from.
    key: PathBuf,
}

pub struct Session {
    notes: Entity<Notes>,
    editor: Entity<EditorView>,
    notes_dir: PathBuf,
    recovery: Option<RecoveryStore>,
    doc: Document,
    writer: Writer,
    autosave: Option<Task<()>>,
    /// When the oldest unsaved edit must be saved by, however busy the typing.
    autosave_deadline: Option<Instant>,
    snapshot_timer: Option<Task<()>>,
    refresh: Option<Task<()>>,
    /// Unsaved text from crashed runs, or that could not be saved to a note the user left,
    /// offered one at a time.
    recovered: VecDeque<Snapshot>,
    restore: Option<Restore>,
    /// Called once the first note (or new note) is in the editor, to log startup timing.
    first_load: Option<Box<dyn FnOnce()>>,
    /// Watches the notes folder; `None` when [`Storage::watch`] is off.
    watcher: Option<Task<()>>,
    _find_recovered: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Session {
    pub fn new(
        notes: Entity<Notes>,
        editor: Entity<EditorView>,
        storage: &Storage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Nothing is open until the notes model opens something.
        editor.update(cx, |editor, _| editor.set_read_only(true));
        let subscriptions = vec![
            cx.subscribe_in(&notes, window, Self::on_notes_event),
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| match event {
                EditorEvent::Changed => this.edited(cx),
            }),
            // GPUI reports switching to another window as the window losing activation.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.flush(cx);
                }
            }),
        ];
        let recovery = storage.recovery_dir.clone().map(RecoveryStore::new);
        let find_recovered = recovery
            .clone()
            .map(|recovery| Self::find_recovered(recovery, cx));
        let watcher = storage
            .watch
            .then(|| watch::start(storage.notes.dir.clone(), cx));
        Self {
            notes,
            editor,
            notes_dir: storage.notes.dir.clone(),
            recovery,
            doc: Document::new(Target::None),
            writer: Writer::default(),
            autosave: None,
            autosave_deadline: None,
            snapshot_timer: None,
            refresh: None,
            recovered: VecDeque::new(),
            restore: None,
            first_load: None,
            watcher,
            _find_recovered: find_recovered,
            _subscriptions: subscriptions,
        }
    }

    /// Whether the open note has edits that are not on disk yet (or not handed to the writer).
    pub fn is_dirty(&self) -> bool {
        self.doc.dirty
    }

    /// Whether a file operation is running on a background thread, for tests that interleave
    /// with one.
    #[doc(hidden)]
    pub fn is_writing(&self) -> bool {
        self.writer.running.is_some()
    }

    /// Calls `callback` once, when the first note opened is in the editor.
    pub fn on_first_load(&mut self, callback: impl FnOnce() + 'static) {
        self.first_load = Some(Box::new(callback));
    }

    /// The decisions waiting for the user.
    pub fn notices(&self) -> Vec<Notice> {
        let document = self.doc.notice.as_ref().map(|notice| match notice {
            DocumentNotice::NotUtf8 => Notice::NotUtf8,
            DocumentNotice::Conflict { .. } => Notice::ChangedOnDisk,
            DocumentNotice::Deleted => Notice::DeletedOnDisk,
        });
        let recovered = self.recovered.front().map(|snapshot| {
            let new_note = is_draft_key(&snapshot.note_path);
            let title = if new_note {
                title_from_content(&snapshot.text).unwrap_or_else(|| UNTITLED.to_owned())
            } else {
                title_of(&snapshot.note_path)
            };
            Notice::Recovered { title, new_note }
        });
        document.into_iter().chain(recovered).collect()
    }

    fn on_notes_event(
        &mut self,
        _: &Entity<Notes>,
        event: &NotesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            NotesEvent::OpenNote(path) => {
                self.leave(cx);
                self.open(path.clone(), cx);
            }
            NotesEvent::OpenDraft(id) => {
                self.leave(cx);
                self.start_draft(*id, window, cx);
            }
            NotesEvent::Renamed { from, to } => self.renamed(from, to, cx),
            NotesEvent::Deleted(path) => self.deleted(path, cx),
        }
    }

    /// Saves the open note before another one opens. Text that cannot go to a file is offered
    /// like recovered text: edits waiting for the user's decision about a change by another
    /// program (the question goes with the note, and their snapshot alone would be removed by
    /// the note's next save), and a new note whose file could not be created (its snapshot
    /// alone would only be offered after a restart).
    fn leave(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        if !self.doc.dirty {
            return;
        }
        let undecided = matches!(
            self.doc.notice,
            Some(DocumentNotice::Conflict { .. } | DocumentNotice::Deleted)
        );
        let key = match &self.doc.target {
            Target::Note(path) if undecided => path.clone(),
            // Still a draft after the flush: creating its file failed.
            Target::Draft { key, .. } => key.clone(),
            _ => return,
        };
        let text = self.editor.read(cx).text();
        self.offer_unsaved(key, text, cx);
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // The previous note stays visible, read-only, until this one is read.
        self.doc = Document::new(Target::Loading(path.clone()));
        self.editor
            .update(cx, |editor, _| editor.set_read_only(true));
        self.enqueue(Job::Load(path), cx);
        cx.notify();
    }

    fn loaded(
        &mut self,
        path: PathBuf,
        result: scratchpad_core::Result<NoteText>,
        cx: &mut Context<Self>,
    ) {
        if self.doc.target != Target::Loading(path.clone()) {
            // Another note was opened meanwhile.
            return;
        }
        match result {
            Ok(note) => {
                self.editor.update(cx, |editor, cx| {
                    editor.set_text(&note.text, cx);
                    editor.set_read_only(note.lossy);
                });
                self.doc = Document {
                    title: self.current_title(cx),
                    disk_text: note.text.into(),
                    notice: note.lossy.then_some(DocumentNotice::NotUtf8),
                    ..Document::new(Target::Note(path.clone()))
                };
                if let Some(restore) = self.restore.take_if(|r| r.path.as_ref() == Some(&path)) {
                    self.apply_restore(restore, cx);
                }
                if let Some(first_load) = self.first_load.take() {
                    first_load();
                }
            }
            Err(error) => {
                toast::show_file_error(&title_of(&path), &error, cx);
                self.close_document(cx);
                self.notes.update(cx, |notes, cx| notes.refresh(cx));
            }
        }
        cx.notify();
    }

    fn start_draft(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let key = self.notes_dir.join(format!("{DRAFT_KEY_PREFIX}{nanos}"));
        self.doc = Document::new(Target::Draft { id, key });
        self.editor.update(cx, |editor, cx| {
            editor.set_text("", cx);
            editor.set_read_only(false);
        });
        window.focus(&self.editor.focus_handle(cx));
        if let Some(restore) = self.restore.take_if(|restore| restore.path.is_none()) {
            self.apply_restore(restore, cx);
        }
        if let Some(first_load) = self.first_load.take() {
            first_load();
        }
        cx.notify();
    }

    /// Shows nothing: the open note was deleted or could not be read.
    fn close_document(&mut self, cx: &mut Context<Self>) {
        self.doc = Document::new(Target::None);
        self.autosave = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_text("", cx);
            editor.set_read_only(true);
        });
        self.notes
            .update(cx, |notes, cx| notes.set_open_title(None, cx));
    }

    // --- Saving ---

    fn edited(&mut self, cx: &mut Context<Self>) {
        if matches!(self.doc.target, Target::None | Target::Loading(_)) {
            return;
        }
        self.doc.dirty = true;
        self.show_title(cx);
        self.schedule_autosave(cx);
        if self.snapshot_timer.is_none() {
            let timer = cx.background_executor().timer(SNAPSHOT_INTERVAL);
            self.snapshot_timer = Some(cx.spawn(async move |this, cx| {
                timer.await;
                this.update(cx, |this, cx| this.snapshot_unsaved(cx)).ok();
            }));
        }
    }

    /// Keeps edits that are not saved yet in a recovery snapshot: release builds abort on a
    /// panic, so only what is on disk survives a crash (ADR 0064).
    fn snapshot_unsaved(&mut self, cx: &mut Context<Self>) {
        self.snapshot_timer = None;
        let key = match &self.doc.target {
            Target::Note(path) => path.clone(),
            Target::Draft { key, .. } => key.clone(),
            Target::None | Target::Loading(_) => return,
        };
        if self.doc.dirty {
            self.doc.snapshot = true;
            let text = self.text_snapshot(cx);
            self.enqueue(Job::Snapshot { key, text }, cx);
        }
    }

    /// The editor's text for a save or a recovery snapshot. These run while the user types, so
    /// the text is not copied here (megabytes for a large note) but serialized by the writer,
    /// off the UI thread.
    fn text_snapshot(&self, cx: &Context<Self>) -> TextSnapshot {
        self.editor.read(cx).editor().buffer().snapshot()
    }

    /// Saves after [`AUTOSAVE_DELAY`] without edits, or [`MAX_AUTOSAVE_DELAY`] after the
    /// first unsaved edit, whichever comes first.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let deadline = *self
            .autosave_deadline
            .get_or_insert(now + MAX_AUTOSAVE_DELAY);
        let timer = cx
            .background_executor()
            .timer(AUTOSAVE_DELAY.min(deadline.saturating_duration_since(now)));
        self.autosave = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.persist(false, cx)).ok();
        }));
    }

    /// Saves now, e.g. on Ctrl+S, when switching notes or when the window loses focus. Also
    /// renames the file if the title line changed.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.persist(true, cx);
    }

    /// Hands unsaved edits to the writer. `flush` marks the moments the user leaves the note
    /// (or asks to save): a new note then gets its file even if its title line is unfinished,
    /// and a changed title renames the file.
    fn persist(&mut self, flush: bool, cx: &mut Context<Self>) {
        self.autosave = None;
        self.autosave_deadline = None;
        self.snapshot_timer = None;
        match self.doc.target.clone() {
            Target::Draft { id, key } => self.persist_draft(id, key, flush, cx),
            Target::Note(path) => self.persist_note(path, flush, cx),
            Target::None | Target::Loading(_) => {}
        }
    }

    fn persist_note(&mut self, path: PathBuf, flush: bool, cx: &mut Context<Self>) {
        let blocked = self.doc.notice.is_some();
        if self.doc.dirty {
            if blocked {
                // Keep the edits safe until the user decides.
                self.doc.snapshot = true;
                let text = self.text_snapshot(cx);
                self.enqueue(Job::Snapshot { key: path, text }, cx);
                return;
            }
            let text = SaveText::snapshot(self.text_snapshot(cx));
            self.doc.dirty = false;
            let expected = if mem::take(&mut self.doc.overwrite) {
                None
            } else {
                // What the file holds once the saves before this one are done.
                let pending = self.writer.pending_save(&path);
                Some(pending.unwrap_or_else(|| self.doc.disk_text.clone().into()))
            };
            self.enqueue(
                Job::Save {
                    path: path.clone(),
                    text,
                    expected,
                },
                cx,
            );
        }
        if flush && !blocked {
            let title = self.current_title(cx);
            if title.is_some() && title != self.doc.title {
                self.doc.title = title.clone();
                self.enqueue(
                    Job::Rename {
                        path,
                        title: title.unwrap_or_default(),
                    },
                    cx,
                );
            }
        }
    }

    /// A new note gets its file once its title line is finished (followed by another line) or
    /// the user leaves it; until then it lives in memory and in a recovery snapshot (ADR 0061).
    fn persist_draft(&mut self, id: DraftId, key: PathBuf, flush: bool, cx: &mut Context<Self>) {
        if !self.doc.dirty {
            return;
        }
        let editor = self.editor.read(cx);
        let text: Arc<str> = editor.text().into();
        let title = title_in(editor.editor().buffer());
        let title_finished = title_line_finished(editor.editor().buffer());
        if text.trim().is_empty() {
            // Nothing worth a file.
            self.doc.dirty = false;
            if self.doc.snapshot {
                self.doc.snapshot = false;
                self.enqueue(Job::RemoveSnapshot(key), cx);
            }
            return;
        }
        if !(flush || title_finished) {
            self.doc.snapshot = true;
            let text = self.text_snapshot(cx);
            self.enqueue(Job::Snapshot { key, text }, cx);
            return;
        }
        match self
            .notes
            .update(cx, |notes, cx| notes.save_draft(id, title.as_deref(), cx))
        {
            Ok(note) => {
                let had_snapshot = self.doc.snapshot;
                self.doc = Document {
                    title,
                    ..Document::new(Target::Note(note.path.clone()))
                };
                self.notes
                    .update(cx, |notes, cx| notes.set_open_title(None, cx));
                self.enqueue(
                    Job::Save {
                        path: note.path,
                        text: text.into(),
                        expected: Some(Arc::<str>::from("").into()),
                    },
                    cx,
                );
                if had_snapshot {
                    self.enqueue(Job::RemoveSnapshot(key), cx);
                }
            }
            Err(error) => {
                let name = title.as_deref().unwrap_or(UNTITLED);
                toast::show_file_error(name, &error, cx);
                self.doc.snapshot = true;
                let text = self.text_snapshot(cx);
                self.enqueue(Job::Snapshot { key, text }, cx);
            }
        }
    }

    fn saved(
        &mut self,
        path: PathBuf,
        text: SaveText,
        expected: Option<SaveText>,
        result: scratchpad_core::Result<Note>,
        moved: Option<Moved>,
        cx: &mut Context<Self>,
    ) {
        match (result, moved) {
            (Ok(_), Some(moved)) => {
                // The note was renamed or deleted while this save ran, which may have brought
                // the old file back. Remove it, and save to the new name.
                let text = text.get().clone();
                self.enqueue(Job::RemoveIfUnchanged { path, text }, cx);
                if let Moved::Renamed(to) = moved
                    && self.doc.is_note(&to)
                {
                    self.doc.dirty = true;
                    // The renamed file holds the old or the new text, depending on which won.
                    self.doc.overwrite = true;
                    self.persist(false, cx);
                }
            }
            (Ok(note), None) => {
                if self.doc.is_note(&path) {
                    self.doc.disk_text = text.get().clone();
                    self.doc.snapshot = false;
                }
                self.notes
                    .update(cx, |notes, cx| notes.note_saved(note, cx));
            }
            (Err(error), moved) => {
                toast::show_file_error(&title_of(&path), &error, cx);
                self.writer.save_failed(&path, expected);
                let open = match &moved {
                    Some(Moved::Renamed(to)) => self.doc.is_note(to),
                    // Dropped, as the delete asked.
                    Some(Moved::Deleted) => false,
                    None => self.doc.is_note(&path),
                };
                if open {
                    // Saved again after the next edit or flush; the text is in a snapshot.
                    self.doc.dirty = true;
                    self.doc.snapshot = true;
                } else if moved.is_none() {
                    self.offer_unsaved(path, text.get().to_string(), cx);
                }
            }
        }
    }

    // --- Renames and deletes from the sidebar ---

    fn renamed(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        self.writer.retarget(from, to);
        if let Some(running) = &mut self.writer.running
            && matches!(&running.job, Job::Save { path, .. } if path == from)
            && !same_name_ignoring_case(from, to)
        {
            running.moved = Some(Moved::Renamed(to.to_owned()));
        }
        // A load running now reads the old name, so its result would not be taken.
        let load_again = matches!(
            &self.writer.running,
            Some(Running { job: Job::Load(path), .. }) if path == from
        );
        match &mut self.doc.target {
            Target::Note(path) | Target::Loading(path) if path == from => *path = to.to_owned(),
            _ => return,
        }
        if load_again {
            self.enqueue(Job::Load(to.to_owned()), cx);
        }
        if self.doc.snapshot {
            self.enqueue(Job::RemoveSnapshot(from.to_owned()), cx);
            // Written again under the new name with the next save or snapshot.
            self.doc.dirty = true;
            self.schedule_autosave(cx);
        }
        self.show_title(cx);
    }

    /// The note is gone (to the recycle bin): its buffer is dropped unsaved, as the notes
    /// model's contract asks; a save would bring the file back.
    fn deleted(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.writer
            .retain_for(path, |job| matches!(job, Job::RemoveIfUnchanged { .. }));
        if let Some(running) = &mut self.writer.running
            && matches!(&running.job, Job::Save { path: saving, .. } if saving == path)
        {
            running.moved = Some(Moved::Deleted);
        }
        self.enqueue(Job::RemoveSnapshot(path.to_owned()), cx);
        if matches!(&self.doc.target, Target::Note(open) | Target::Loading(open) if open == path) {
            self.close_document(cx);
            cx.notify();
        }
    }

    fn rename_now(&mut self, path: PathBuf, title: String, cx: &mut Context<Self>) {
        match self
            .notes
            .update(cx, |notes, cx| notes.rename(&path, &title, cx))
        {
            // Retarget now: jobs after this one may already be queued for the old name. The
            // `Renamed` event that follows then finds nothing left to do.
            Ok(to) => self.renamed(&path, &to, cx),
            Err(error) => toast::show_file_error(&title_of(&path), &error, cx),
        }
    }

    // --- Changes by other programs ---

    /// Filesystem events from the watcher (tests inject them here). Changes to the open note
    /// are checked against the disk; others re-read the note list.
    pub fn disk_events(&mut self, events: Vec<NoteEvent>, cx: &mut Context<Self>) {
        let open = match &self.doc.target {
            Target::Note(path) => Some(path.clone()),
            _ => None,
        };
        let (open_changed, others): (Vec<_>, Vec<_>) = events
            .iter()
            .partition(|event| Some(event.path()) == open.as_deref());
        if let (Some(open), false) = (open, open_changed.is_empty()) {
            self.enqueue(Job::Check(open), cx);
        }
        if !others.is_empty() {
            self.refresh_list(cx);
        }
    }

    fn refresh_list(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(REFRESH_DELAY);
        let notes = self.notes.clone();
        self.refresh = Some(cx.spawn(async move |_, cx| {
            timer.await;
            notes.update(cx, |notes, cx| notes.refresh(cx)).ok();
        }));
    }

    fn checked(
        &mut self,
        path: PathBuf,
        result: std::io::Result<Option<NoteText>>,
        cx: &mut Context<Self>,
    ) {
        if !self.doc.is_note(&path) {
            return;
        }
        let disk = match result {
            Ok(Some(disk)) => disk,
            Ok(None) => {
                self.doc.notice = Some(DocumentNotice::Deleted);
                self.refresh_list(cx);
                cx.notify();
                return;
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not check the open note");
                return;
            }
        };
        if self.doc.notice == Some(DocumentNotice::Deleted) {
            // It came back.
            self.doc.notice = None;
        }
        let disk_text: Arc<str> = disk.text.into();
        if disk_text == self.doc.disk_text {
            // Our own save, or a change that was undone: nothing to reload.
            if matches!(self.doc.notice, Some(DocumentNotice::Conflict { .. })) {
                self.doc.notice = None;
            }
            if self.doc.dirty {
                self.schedule_autosave(cx);
            }
        } else if self.doc.dirty || self.writer.has_save_for(&path) {
            // Never overwrite either version without asking (PLAN §30).
            self.writer
                .retain_for(&path, |job| !matches!(job, Job::Save { .. }));
            self.doc.dirty = true;
            self.doc.notice = Some(DocumentNotice::Conflict {
                disk: disk_text,
                lossy: disk.lossy,
            });
            self.persist(false, cx);
            self.refresh_list(cx);
        } else {
            self.editor.update(cx, |editor, cx| {
                editor.reload_text(&disk_text, cx);
                editor.set_read_only(disk.lossy);
            });
            self.doc.disk_text = disk_text;
            self.doc.title = self.current_title(cx);
            self.doc.notice = disk.lossy.then_some(DocumentNotice::NotUtf8);
            self.refresh_list(cx);
        }
        cx.notify();
    }

    /// A save found the file changed or deleted by another program since we last read or wrote
    /// it, so it did not write (and kept the text in a snapshot instead).
    fn changed_before_save(
        &mut self,
        path: PathBuf,
        text: &SaveText,
        disk: Option<NoteText>,
        cx: &mut Context<Self>,
    ) {
        if self.doc.is_note(&path) {
            self.writer
                .retain_for(&path, |job| !matches!(job, Job::Save { .. }));
            self.doc.dirty = true;
            self.doc.snapshot = true;
            self.doc.notice = Some(match disk {
                Some(disk) => DocumentNotice::Conflict {
                    disk: disk.text.into(),
                    lossy: disk.lossy,
                },
                None => DocumentNotice::Deleted,
            });
        } else {
            self.offer_unsaved(path, text.get().to_string(), cx);
        }
        self.refresh_list(cx);
        cx.notify();
    }

    // --- Decisions ---

    /// Acts on a button of the notice bar.
    pub fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let notice = self.doc.notice.clone();
        match (choice, notice) {
            (Choice::EditAnyway, Some(DocumentNotice::NotUtf8)) => {
                self.doc.notice = None;
                self.editor
                    .update(cx, |editor, _| editor.set_read_only(false));
            }
            (Choice::KeepMine, Some(DocumentNotice::Conflict { disk, .. })) => {
                // Their version stays one undo away.
                let mine = self.editor.read(cx).text();
                self.editor.update(cx, |editor, cx| {
                    editor.replace_text(&disk, cx);
                    editor.replace_text(&mine, cx);
                });
                self.save_over_disk(cx);
            }
            (Choice::KeepDeleted, Some(DocumentNotice::Deleted)) => self.save_over_disk(cx),
            (Choice::LoadDisk, Some(DocumentNotice::Conflict { disk, lossy })) => {
                // Like opening it: their version is read-only if not valid UTF-8 (ADR 0066).
                self.doc.notice = lossy.then_some(DocumentNotice::NotUtf8);
                self.doc.dirty = false;
                // My version stays one undo away.
                self.editor.update(cx, |editor, cx| {
                    editor.replace_text(&disk, cx);
                    editor.set_read_only(lossy);
                });
                self.doc.disk_text = disk;
                self.doc.title = self.current_title(cx);
                if let (Target::Note(path), true) = (&self.doc.target, self.doc.snapshot) {
                    self.doc.snapshot = false;
                    self.enqueue(Job::RemoveSnapshot(path.clone()), cx);
                }
                self.show_title(cx);
            }
            (Choice::CloseDeleted, Some(DocumentNotice::Deleted)) => {
                if let Target::Note(path) = self.doc.target.clone() {
                    self.enqueue(Job::RemoveSnapshot(path.clone()), cx);
                    self.close_document(cx);
                    self.notes.update(cx, |notes, cx| notes.forget(&path, cx));
                }
            }
            (Choice::Restore, _) => {
                if let Some(snapshot) = self.recovered.pop_front() {
                    self.restore(snapshot, cx);
                }
            }
            (Choice::Discard, _) => {
                if let Some(snapshot) = self.recovered.pop_front()
                    // Then the snapshot holds the open note's unsaved text instead.
                    && !(self.doc.is_note(&snapshot.note_path) && self.doc.snapshot)
                {
                    self.enqueue(Job::RemoveSnapshot(snapshot.note_path), cx);
                }
            }
            _ => {}
        }
        window.focus(&self.editor.focus_handle(cx));
        cx.notify();
    }

    /// Saves the editor's text over whatever the file holds now, or recreates it.
    fn save_over_disk(&mut self, cx: &mut Context<Self>) {
        self.doc.notice = None;
        self.doc.dirty = true;
        self.doc.overwrite = true;
        self.persist(false, cx);
    }

    // --- Crash recovery ---

    fn find_recovered(recovery: RecoveryStore, cx: &mut Context<Self>) -> Task<()> {
        let started = SystemTime::now();
        let scan = cx.background_spawn(async move { recovery.leftovers(started) });
        cx.spawn(async move |this, cx| {
            let leftovers = scan.await;
            if leftovers.is_empty() {
                return;
            }
            tracing::info!(count = leftovers.len(), "found unsaved text from a crash");
            this.update(cx, |this, cx| {
                this.recovered.extend(leftovers);
                cx.notify();
            })
            .ok();
        })
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        let path = snapshot.note_path;
        // A note of a folder used before the notes folder was changed is not opened from
        // there: renaming it would move it into this folder. Its text becomes a new note here.
        let into_note = !is_draft_key(&path)
            && path.parent() == Some(self.notes_dir.as_path())
            && path.is_file();
        let restore = Restore {
            path: into_note.then(|| path.clone()),
            text: snapshot.text,
            key: path.clone(),
        };
        if self.doc.is_note(&path) {
            self.apply_restore(restore, cx);
            return;
        }
        // Applied once the note (or the new note) is open.
        self.restore = Some(restore);
        self.notes.update(cx, |notes, cx| {
            if into_note {
                notes.select(&path, cx);
            } else {
                notes.new_note(cx);
            }
        });
    }

    fn apply_restore(&mut self, restore: Restore, cx: &mut Context<Self>) {
        // What it replaces stays one undo away.
        self.editor.update(cx, |editor, cx| {
            editor.replace_text(&restore.text, cx);
            editor.set_read_only(false);
        });
        self.doc.notice = None;
        self.doc.dirty = true;
        self.show_title(cx);
        // Saved (or snapshotted under its new key) before the old snapshot goes.
        self.persist(false, cx);
        if restore.path.is_none() {
            self.enqueue(Job::RemoveSnapshot(restore.key), cx);
        }
    }

    /// Offers text that could not be saved to a note the user has left like recovered text:
    /// its snapshot alone would be removed by the note's next save.
    fn offer_unsaved(&mut self, note_path: PathBuf, text: String, cx: &mut Context<Self>) {
        self.recovered
            .retain(|offered| offered.note_path != note_path);
        self.recovered.push_back(Snapshot {
            note_path,
            text,
            saved_at: SystemTime::now(),
        });
        cx.notify();
    }

    // --- Closing ---

    /// Writes everything still pending, on this thread, before the window closes or the app
    /// quits: tasks do not run after that.
    pub fn flush_sync(&mut self, cx: &mut Context<Self>) {
        let renamed_while_saving = self
            .writer
            .running
            .as_ref()
            .and_then(|running| match &running.moved {
                Some(Moved::Renamed(to)) => Some(to.clone()),
                _ => None,
            });
        if renamed_while_saving.is_some_and(|to| self.doc.is_note(&to)) {
            self.doc.dirty = true;
        }
        self.persist(true, cx);
        let flushed = self.writer.flush_marker();
        // Waits for a background job that is writing right now.
        let mut flushed = flushed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut jobs = self.writer.take_unfinished(*flushed);
        // Background jobs started before now will skip themselves.
        *flushed = self.writer.next_number();
        let store = self.notes.read(cx).store().cloned();
        for ix in 0..jobs.len() {
            match jobs[ix].clone() {
                Job::Rename { path, title } => {
                    match self
                        .notes
                        .update(cx, |notes, cx| notes.rename(&path, &title, cx))
                    {
                        Ok(to) => writer::retarget(jobs[ix + 1..].iter_mut(), &path, &to),
                        Err(error) => tracing::warn!(%error, "could not rename on exit"),
                    }
                }
                Job::Load(_) | Job::Check(_) => {}
                job => {
                    let outcome = writer::run(&job, store.as_ref(), self.recovery.as_ref());
                    // The text is in a recovery snapshot, so it is offered on the next start;
                    // and now, in case the app goes on with another notes folder.
                    if let Job::Save { path, text, .. } = job {
                        match outcome {
                            Outcome::Saved(Err(error)) => {
                                tracing::error!(%error, "could not save while flushing");
                                toast::show_file_error(&title_of(&path), &error, cx);
                            }
                            Outcome::ChangedOnDisk(_) => {}
                            _ => continue,
                        }
                        self.offer_unsaved(path, text.get().to_string(), cx);
                    }
                }
            }
        }
        self.doc.dirty = false;
    }

    // --- Changing the notes folder ---

    /// Writes everything for the current folder, on this thread, and closes the open note before
    /// the notes model switches to `dir`: saves, renames and new notes' files must land in the
    /// folder they belong to. Edits waiting for a decision about a change by another program
    /// are offered like recovered text, as when switching notes. See ADR 0081.
    pub fn change_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.leave(cx);
        self.flush_sync(cx);
        self.close_document(cx);
        self.refresh = None;
        if self.watcher.is_some() {
            // Replacing the task stops the old watcher.
            self.watcher = Some(watch::start(dir.clone(), cx));
        }
        self.notes_dir = dir;
        cx.notify();
    }

    // --- Writer ---

    fn enqueue(&mut self, job: Job, cx: &mut Context<Self>) {
        self.writer.push(job);
        if self.writer.busy {
            return;
        }
        self.writer.busy = true;
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(started)) = this.update(cx, |this, cx| this.start_next_job(cx)) else {
                    break;
                };
                let outcome = started.await;
                if this
                    .update(cx, |this, cx| this.finish_job(outcome, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Starts the next job on a background thread, running renames (UI-thread work) inline.
    /// `None` when the queue is empty.
    fn start_next_job(&mut self, cx: &mut Context<Self>) -> Option<Task<Outcome>> {
        loop {
            let Some(job) = self.writer.pop() else {
                self.writer.busy = false;
                return None;
            };
            if let Job::Rename { path, title } = job {
                self.rename_now(path, title, cx);
                continue;
            }
            let number = self.writer.next_number();
            let flushed = self.writer.flush_marker();
            let store = self.notes.read(cx).store().cloned();
            let recovery = self.recovery.clone();
            let running = job.clone();
            self.writer.running = Some(Running {
                job,
                moved: None,
                number,
            });
            return Some(cx.background_spawn(async move {
                writer::run_in_order(
                    &running,
                    number,
                    &flushed,
                    store.as_ref(),
                    recovery.as_ref(),
                )
            }));
        }
    }

    fn finish_job(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let Some(Running { job, moved, .. }) = self.writer.running.take() else {
            return;
        };
        match (job, outcome) {
            (Job::Load(path), Outcome::Loaded(result)) => self.loaded(path, result, cx),
            (Job::Check(path), Outcome::Checked(result)) => self.checked(path, result, cx),
            (
                Job::Save {
                    path,
                    text,
                    expected,
                },
                Outcome::Saved(result),
            ) => self.saved(path, text, expected, result, moved, cx),
            (Job::Save { path, text, .. }, Outcome::ChangedOnDisk(disk)) => match moved {
                // Renamed or deleted from the sidebar meanwhile, so nothing was written; the
                // text goes to the new name with the next save.
                Some(moved) => {
                    self.enqueue(Job::RemoveSnapshot(path), cx);
                    if let Moved::Renamed(to) = moved
                        && self.doc.is_note(&to)
                    {
                        self.doc.dirty = true;
                        self.persist(false, cx);
                    }
                }
                None => self.changed_before_save(path, &text, disk, cx),
            },
            _ => {}
        }
    }

    // --- Titles ---

    fn current_title(&self, cx: &Context<Self>) -> Option<String> {
        title_in(self.editor.read(cx).editor().buffer())
    }

    /// The sidebar shows the draft's title, and a note's new title until its file is renamed.
    fn show_title(&mut self, cx: &mut Context<Self>) {
        let title = self.current_title(cx);
        let shown = match &self.doc.target {
            Target::Draft { .. } => title,
            Target::Note(_) if title.is_some() && title != self.doc.title => title,
            _ => None,
        };
        self.notes
            .update(cx, |notes, cx| notes.set_open_title(shown, cx));
    }
}

/// The note's title: its first meaningful line (see [`title_from_content`]).
fn title_in(buffer: &Buffer) -> Option<String> {
    title_line(buffer).map(|(_, title)| title)
}

/// Whether the title line has a line after it, i.e. the user moved on from typing it.
fn title_line_finished(buffer: &Buffer) -> bool {
    match title_line(buffer) {
        Some((line, _)) => line + 1 < buffer.line_count(),
        None => buffer.line_count() > 1,
    }
}

fn title_line(buffer: &Buffer) -> Option<(usize, String)> {
    (0..buffer.line_count().min(TITLE_SEARCH_LINES))
        .find_map(|line| title_from_content(&buffer.line_text(line)).map(|title| (line, title)))
}

fn is_draft_key(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(DRAFT_KEY_PREFIX))
}

/// On case-insensitive filesystems a case-only rename keeps the same file, which a save to the
/// old spelling then still updates in place.
fn same_name_ignoring_case(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}
