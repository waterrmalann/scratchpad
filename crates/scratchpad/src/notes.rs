//! The notes shown in the sidebar: the folder, the note list, which note is open and the new
//! note that has not been saved yet. See ADR 0050.
//!
//! Views and the editor integration react to [`NotesEvent`]s; they never touch the folder
//! directly, so the list always matches what is on disk.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext, Context, EventEmitter, Task};
use scratchpad_core::{Note, NoteStore};

use crate::toast;

/// Where notes live and how they are deleted.
#[derive(Clone, Debug)]
pub struct NotesLocation {
    pub dir: PathBuf,
    /// Replaces the move to the OS recycle bin. Tests use it so they never fill the real bin.
    pub deleter: Option<fn(&Path) -> io::Result<()>>,
}

impl NotesLocation {
    /// Notes in `dir`; deleted notes go to the OS recycle bin.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            deleter: None,
        }
    }

    fn open(&self) -> scratchpad_core::Result<NoteStore> {
        match self.deleter {
            Some(deleter) => NoteStore::with_deleter(&self.dir, deleter),
            None => NoteStore::open(&self.dir),
        }
    }
}

/// What the editor shows. Picking a note in the sidebar opens it, so selected and open are the
/// same thing (as in iCloud Notes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    None,
    /// A new note that exists only in memory until it has content (PLAN §9).
    Draft(DraftId),
    Note(PathBuf),
}

/// Identifies a new, unsaved note. Each Ctrl+N starts a new draft, so when the editor saves one
/// that the user has already left, [`Notes::save_draft`] can tell it is no longer the open one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftId(u64);

/// What the editor must do to stay in step with the notes. The selection has already changed
/// when an event arrives, so the editor tracks which note (or draft) its own buffer belongs to.
/// On `OpenNote` / `OpenDraft` it first saves the current buffer to where it belongs (a draft
/// with content through [`Notes::save_draft`] with its own id), then shows the new one. Only
/// the latest of several quick `OpenNote`s (e.g. holding Down) needs to be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesEvent {
    /// Load this note into the editor.
    OpenNote(PathBuf),
    /// Show an empty editor for a new note.
    OpenDraft(DraftId),
}

pub struct Notes {
    location: NotesLocation,
    /// `None` until the folder has been opened off the UI thread.
    store: Option<NoteStore>,
    /// Newest first, as listed by [`NoteStore::list`].
    notes: Arc<Vec<Note>>,
    loaded: bool,
    selection: Selection,
    /// The id of the most recent draft.
    last_draft: u64,
    refresh_task: Option<Task<()>>,
}

impl EventEmitter<NotesEvent> for Notes {}

impl Notes {
    /// Starts listing the folder in the background: the window renders first and the sidebar
    /// fills in when the listing arrives (PLAN §39).
    pub fn new(location: NotesLocation, cx: &mut Context<Self>) -> Self {
        let mut notes = Self {
            location,
            store: None,
            notes: Arc::default(),
            loaded: false,
            selection: Selection::None,
            last_draft: 0,
            refresh_task: None,
        };
        notes.refresh(cx);
        notes
    }

    /// Re-reads the note list from disk (metadata only). A newer refresh replaces one that is
    /// still running.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let location = self.location.clone();
        let listing = cx.background_spawn(async move {
            let store = match store {
                Some(store) => store,
                None => location.open()?,
            };
            let notes = store.list()?;
            scratchpad_core::Result::Ok((store, notes))
        });
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            let listed = listing.await;
            this.update(cx, |this, cx| {
                match listed {
                    Ok((store, notes)) => {
                        this.store = Some(store);
                        this.notes = Arc::new(notes);
                    }
                    Err(error) => toast::show_file_error(&this.folder_name(), &error, cx),
                }
                this.loaded = true;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Notes on disk, most recently modified first.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// Whether the first listing has finished (successfully or not).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn has_draft(&self) -> bool {
        matches!(self.selection, Selection::Draft(_))
    }

    pub fn note(&self, path: &Path) -> Option<&Note> {
        self.notes.iter().find(|note| note.path == path)
    }

    /// Opens `path` unless it is already open.
    pub fn select(&mut self, path: &Path, cx: &mut Context<Self>) {
        if matches!(&self.selection, Selection::Note(open) if open == path) {
            return;
        }
        self.selection = Selection::Note(path.to_owned());
        cx.emit(NotesEvent::OpenNote(path.to_owned()));
        cx.notify();
    }

    /// Starts a new note in memory and asks for it to be opened. Nothing is written until
    /// [`save_draft`](Self::save_draft), so pressing Ctrl+N and leaving leaves no empty file.
    pub fn new_note(&mut self, cx: &mut Context<Self>) {
        self.last_draft += 1;
        let draft = DraftId(self.last_draft);
        self.selection = Selection::Draft(draft);
        cx.emit(NotesEvent::OpenDraft(draft));
        cx.notify();
    }

    /// Creates the file for `draft`, named after `title` (or `Untitled`), and returns it. The
    /// caller then saves the text to [`Note::path`].
    ///
    /// If the draft is still open, the new note replaces it as the selection without another
    /// [`NotesEvent::OpenNote`]. If the user has already moved on (the editor saves the draft
    /// while switching), the note is only added to the list.
    pub fn save_draft(
        &mut self,
        draft: DraftId,
        title: Option<&str>,
        cx: &mut Context<Self>,
    ) -> scratchpad_core::Result<Note> {
        let note = self.store()?.create(title)?;
        Arc::make_mut(&mut self.notes).insert(0, note.clone());
        if self.selection == Selection::Draft(draft) {
            self.selection = Selection::Note(note.path.clone());
        }
        cx.notify();
        Ok(note)
    }

    fn store(&self) -> scratchpad_core::Result<&NoteStore> {
        self.store
            .as_ref()
            .ok_or_else(|| scratchpad_core::Error::Io {
                action: "open",
                path: self.location.dir.clone(),
                source: io::Error::other("the notes folder is not available yet"),
            })
    }

    fn folder_name(&self) -> String {
        self.location
            .dir
            .file_name()
            .unwrap_or(self.location.dir.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}
