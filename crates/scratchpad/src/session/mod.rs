//! The open note's lifecycle: loading it into the editor, autosave, giving new notes a file,
//! keeping file names in step with titles (PLAN §7, §9, §44, §56). See ADRs 0060-0062 and 0066.
//!
//! [`Session`] follows the notes model's [`NotesEvent`]s and the editor's
//! [`EditorEvent::Changed`]. Every file operation goes through one ordered queue (see
//! [`writer`]), so the editor never waits for the disk and the disk never sees writes out of
//! order.

mod writer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Context, Entity, Focusable, Subscription, Task, Window};
use scratchpad_core::{Note, NoteText, UNTITLED, title_from_content};
use scratchpad_editor::Buffer;

use crate::editor_view::{EditorEvent, EditorView};
use crate::notes::{DraftId, Notes, NotesEvent, title_of};
use crate::toast;
use writer::{Job, Moved, Outcome, Running, Writer};

/// Quiet time after the last edit before the note is saved (PLAN §7).
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(300);
/// The longest a change waits for a save while the user keeps typing without pausing.
pub const MAX_AUTOSAVE_DELAY: Duration = Duration::from_secs(2);
/// Titles are looked for this far into a note, so a keystroke never scans a huge document.
const TITLE_SEARCH_LINES: usize = 200;

/// What the editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    None,
    /// Being read; the editor is read-only and still shows the previous note.
    Loading(PathBuf),
    /// A new note without a file.
    Draft(DraftId),
    Note(PathBuf),
}

/// The note in the editor and how it relates to its file.
struct Document {
    target: Target,
    /// Edits not yet handed to the writer (or whose save failed).
    dirty: bool,
    /// The title as loaded or as last renamed to. The file is renamed only when the title line
    /// is changed (ADR 0061).
    title: Option<String>,
    notice: Option<DocumentNotice>,
}

impl Document {
    fn new(target: Target) -> Self {
        Self {
            target,
            dirty: false,
            title: None,
            notice: None,
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
}

/// What the notice bar above the editor shows, most urgent first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    NotUtf8,
}

/// The buttons of the notice bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    EditAnyway,
}

pub struct Session {
    notes: Entity<Notes>,
    editor: Entity<EditorView>,
    doc: Document,
    writer: Writer,
    autosave: Option<Task<()>>,
    /// When the oldest unsaved edit must be saved by, however busy the typing.
    autosave_deadline: Option<Instant>,
    /// Called once the first note (or new note) is in the editor, to log startup timing.
    first_load: Option<Box<dyn FnOnce()>>,
    _subscriptions: Vec<Subscription>,
}

impl Session {
    pub fn new(
        notes: Entity<Notes>,
        editor: Entity<EditorView>,
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
        Self {
            notes,
            editor,
            doc: Document::new(Target::None),
            writer: Writer::default(),
            autosave: None,
            autosave_deadline: None,
            first_load: None,
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
        });
        document.into_iter().collect()
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
                self.flush(cx);
                self.open(path.clone(), cx);
            }
            NotesEvent::OpenDraft(id) => {
                self.flush(cx);
                self.start_draft(*id, window, cx);
            }
            NotesEvent::Renamed { from, to } => self.renamed(from, to, cx),
            NotesEvent::Deleted(path) => self.deleted(path, cx),
        }
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
                    notice: note.lossy.then_some(DocumentNotice::NotUtf8),
                    ..Document::new(Target::Note(path))
                };
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
        self.doc = Document::new(Target::Draft(id));
        self.editor.update(cx, |editor, cx| {
            editor.set_text("", cx);
            editor.set_read_only(false);
        });
        window.focus(&self.editor.focus_handle(cx));
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
        match self.doc.target.clone() {
            Target::Draft(id) => self.persist_draft(id, flush, cx),
            Target::Note(path) => self.persist_note(path, flush, cx),
            Target::None | Target::Loading(_) => {}
        }
    }

    fn persist_note(&mut self, path: PathBuf, flush: bool, cx: &mut Context<Self>) {
        // A note that needs a decision first is not saved.
        let blocked = self.doc.notice.is_some();
        if self.doc.dirty && !blocked {
            let text: Arc<str> = self.editor.read(cx).text().into();
            self.doc.dirty = false;
            self.enqueue(
                Job::Save {
                    path: path.clone(),
                    text,
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
    /// the user leaves it; until then it lives in memory (ADR 0061).
    fn persist_draft(&mut self, id: DraftId, flush: bool, cx: &mut Context<Self>) {
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
            return;
        }
        if !(flush || title_finished) {
            return;
        }
        match self
            .notes
            .update(cx, |notes, cx| notes.save_draft(id, title.as_deref(), cx))
        {
            Ok(note) => {
                self.doc = Document {
                    title,
                    ..Document::new(Target::Note(note.path.clone()))
                };
                self.notes
                    .update(cx, |notes, cx| notes.set_open_title(None, cx));
                self.enqueue(
                    Job::Save {
                        path: note.path,
                        text,
                    },
                    cx,
                );
            }
            Err(error) => {
                // Tried again at the next pause in typing or flush.
                let name = title.as_deref().unwrap_or(UNTITLED);
                toast::show_file_error(name, &error, cx);
            }
        }
    }

    fn saved(
        &mut self,
        path: PathBuf,
        text: Arc<str>,
        result: scratchpad_core::Result<Note>,
        moved: Option<Moved>,
        cx: &mut Context<Self>,
    ) {
        match (result, moved) {
            (Ok(_), Some(moved)) => {
                // The note was renamed or deleted while this save ran, which may have brought
                // the old file back. Remove it, and save to the new name.
                self.enqueue(Job::RemoveIfUnchanged { path, text }, cx);
                if let Moved::Renamed(to) = moved
                    && self.doc.is_note(&to)
                {
                    self.doc.dirty = true;
                    self.persist(false, cx);
                }
            }
            (Ok(note), None) => {
                self.notes
                    .update(cx, |notes, cx| notes.note_saved(note, cx));
            }
            (Err(error), _) => {
                toast::show_file_error(&title_of(&path), &error, cx);
                if self.doc.is_note(&path) {
                    // Saved again after the next edit or flush.
                    self.doc.dirty = true;
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

    // --- Decisions ---

    /// Acts on a button of the notice bar.
    pub fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        if let (Choice::EditAnyway, Some(DocumentNotice::NotUtf8)) = (choice, &self.doc.notice) {
            self.doc.notice = None;
            self.editor
                .update(cx, |editor, _| editor.set_read_only(false));
        }
        window.focus(&self.editor.focus_handle(cx));
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
        let mut jobs = self.writer.take_unfinished();
        let flushed = self.writer.flush_marker();
        // Waits for a background job that is writing right now.
        let mut flushed = flushed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
                Job::Load(_) => {}
                job => {
                    if let Outcome::Saved(Err(error)) = writer::run(&job, store.as_ref()) {
                        tracing::error!(%error, "could not save on exit");
                    }
                }
            }
        }
        self.doc.dirty = false;
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
            let running = job.clone();
            self.writer.running = Some(Running { job, moved: None });
            return Some(cx.background_spawn(async move {
                writer::run_in_order(&running, number, &flushed, store.as_ref())
            }));
        }
    }

    fn finish_job(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        let Some(Running { job, moved }) = self.writer.running.take() else {
            return;
        };
        match (job, outcome) {
            (Job::Load(path), Outcome::Loaded(result)) => self.loaded(path, result, cx),
            (Job::Save { path, text }, Outcome::Saved(result)) => {
                self.saved(path, text, result, moved, cx)
            }
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
            Target::Draft(_) => title,
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

/// On case-insensitive filesystems a case-only rename keeps the same file, which a save to the
/// old spelling then still updates in place.
fn same_name_ignoring_case(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}
