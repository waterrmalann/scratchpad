//! The notes shown in the sidebar: the folder, the note list, which note (or file from outside
//! the folder) is open, the new note that has not been saved yet and the note search. See ADRs
//! 0050 and 0145.
//!
//! Views and the editor integration react to [`NotesEvent`]s; they never touch the folder
//! directly, so the list always matches what is on disk.

use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use gpui::{AppContext, Context, EventEmitter, Task};
use scratchpad_core::{Note, NoteSearch, NoteStore, SearchHit, is_note_path};

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

    /// Opens the folder (creating it if needed) and checks that notes can be written there, so
    /// the user hears about a folder that cannot hold notes before switching to it rather than
    /// with the first save. Blocks on the disk: call it off the UI thread.
    pub fn open_writable(&self) -> scratchpad_core::Result<NoteStore> {
        let store = self.open()?;
        // Hidden and named like the store's temporary files, which it removes if one is left.
        let probe = self
            .dir
            .join(format!(".scratchpad-check-{}.tmp", std::process::id()));
        let written = fs::write(&probe, b"").and_then(|()| fs::remove_file(&probe));
        written.map_err(|source| scratchpad_core::Error::Io {
            action: "write to",
            path: self.dir.clone(),
            source,
        })?;
        Ok(store)
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
    /// A file that is not a note of the folder, opened with File > Open: it is edited in place
    /// but not listed, and its name never follows its first line.
    File(PathBuf),
}

/// Identifies a new, unsaved note. Each Ctrl+N starts a new draft, so when the editor saves one
/// that the user has already left, [`Notes::save_draft`] can tell it is no longer the open one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftId(u64);

/// What the editor must do to stay in step with the notes. The selection has already changed
/// when an event arrives, so the editor tracks which note (or draft) its own buffer belongs to:
///
/// - `OpenNote` / `OpenDraft`: first save the current buffer to where it belongs (a draft with
///   content through [`Notes::save_draft`] with its own id), then show the new one. Only the
///   latest of several quick `OpenNote`s (e.g. holding Down) needs to be loaded.
/// - `Renamed`: if the buffer belongs to `from`, keep it and save to `to` from now on. A save
///   to `from` that is still running would recreate the old file.
/// - `Deleted`: if the buffer belongs to that note, drop it without saving; a save would bring
///   the file back. Edits not saved yet are not in the recycle bin copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesEvent {
    /// Load this note into the editor.
    OpenNote(PathBuf),
    /// Load this file, which is not a note of the folder, into the editor.
    OpenFile(PathBuf),
    /// Show an empty editor for a new note.
    OpenDraft(DraftId),
    /// A note's file was renamed.
    Renamed { from: PathBuf, to: PathBuf },
    /// A note was moved to the recycle bin. If it was open, an `OpenNote` for the next note
    /// follows if there is one.
    Deleted(PathBuf),
}

pub struct Notes {
    location: NotesLocation,
    /// `None` until the folder has been opened off the UI thread.
    store: Option<NoteStore>,
    /// Newest first, as listed by [`NoteStore::list`].
    notes: Arc<Vec<Note>>,
    loaded: bool,
    /// Counts changes made through this model, so a listing that started before one of them
    /// is not applied over it.
    changes: u64,
    selection: Selection,
    /// The id of the most recent draft.
    last_draft: u64,
    /// Shown instead of the open note's file name while its title line has been edited but
    /// the file not renamed yet, and as the title of the draft.
    open_title: Option<String>,
    /// Open something once the first listing arrives, unless a note was opened before.
    open_after_listing: bool,
    refresh_task: Option<Task<()>>,
    query: String,
    /// Results for `query`; `None` while not searching or until the first results arrive.
    hits: Option<Vec<SearchHit>>,
    /// Shared with the background search, which keeps note contents cached between searches.
    search: Arc<Mutex<NoteSearch>>,
    /// The running search. Replacing it drops (cancels) the previous one, so results of an
    /// older query can never overwrite newer ones.
    search_task: Option<Task<()>>,
}

impl EventEmitter<NotesEvent> for Notes {}

impl Notes {
    /// Opens the folder and lists it in the background: the window renders first and the
    /// sidebar fills in when the listing arrives (PLAN §39).
    ///
    /// The first of `reopen` (what was open when the app last closed) that still exists is
    /// opened as soon as the folder is, before the listing: a note of this folder, or any other
    /// file as with [`open_file`](Self::open_file). If none does, the newest note is opened once
    /// the list arrives, or a new note if there are none.
    pub fn new(location: NotesLocation, reopen: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut notes = Self {
            location,
            store: None,
            notes: Arc::default(),
            loaded: false,
            changes: 0,
            selection: Selection::None,
            last_draft: 0,
            open_title: None,
            open_after_listing: true,
            refresh_task: None,
            query: String::new(),
            hits: None,
            search: Arc::default(),
            search_task: None,
        };
        notes.open_folder(reopen, cx);
        notes
    }

    fn open_folder(&mut self, reopen: Vec<PathBuf>, cx: &mut Context<Self>) {
        let location = self.location.clone();
        let opening = cx.background_spawn(async move {
            let store = location.open()?;
            let reopen = reopen.into_iter().find(|path| path.is_file());
            scratchpad_core::Result::Ok((store, reopen))
        });
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            let opened = opening.await;
            this.update(cx, |this, cx| match opened {
                Ok((store, reopen)) => {
                    this.store = Some(store);
                    if let Some(path) = reopen {
                        this.open_file(&path, cx);
                    }
                    this.refresh(cx);
                }
                Err(error) => {
                    toast::show_file_error(&this.folder_name(), &error, cx);
                    this.loaded = true;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Re-reads the note list from disk (metadata only). A newer refresh replaces one that is
    /// still running.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let location = self.location.clone();
        let changes = self.changes;
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
                if this.changes != changes {
                    // A rename, delete or new note happened meanwhile; list again.
                    this.refresh(cx);
                    return;
                }
                match listed {
                    Ok((store, notes)) => {
                        this.store = Some(store);
                        this.notes = Arc::new(notes);
                        this.search(cx);
                        if mem::take(&mut this.open_after_listing)
                            && this.selection == Selection::None
                        {
                            this.open_newest(cx);
                        }
                    }
                    Err(error) => toast::show_file_error(&this.folder_name(), &error, cx),
                }
                this.loaded = true;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Opens the newest note, or a new note if there are none.
    fn open_newest(&mut self, cx: &mut Context<Self>) {
        match self.notes.first() {
            Some(newest) => self.select(&newest.path.clone(), cx),
            None => self.new_note(cx),
        }
    }

    /// Shows the notes of another folder, whose `store` the caller has opened (see
    /// [`NotesLocation::open_writable`]). The caller must have saved the open note first: the
    /// selection is cleared without an event. Once the folder is listed its newest note opens,
    /// or a new note if it has none. A file from outside the folder stays open, unless it is a
    /// note of the new folder: then it is left like a note, as the list's rename and delete
    /// would not follow it.
    pub fn change_folder(
        &mut self,
        location: NotesLocation,
        store: NoteStore,
        cx: &mut Context<Self>,
    ) {
        self.location = location;
        self.store = Some(store);
        self.notes = Arc::default();
        self.loaded = false;
        self.changes += 1;
        if !matches!(&self.selection, Selection::File(path) if self.note_path(path).is_none()) {
            self.selection = Selection::None;
        }
        self.open_title = None;
        self.open_after_listing = self.selection == Selection::None;
        self.query.clear();
        self.hits = None;
        // Its cache holds the texts of the other folder's notes.
        self.search = Arc::default();
        self.search_task = None;
        // Replaces (and so cancels) a listing of the old folder.
        self.refresh(cx);
        cx.notify();
    }

    pub fn location(&self) -> &NotesLocation {
        &self.location
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

    /// The folder's store, once it has been opened in the background. Always available while
    /// a note is open.
    pub fn store(&self) -> Option<&NoteStore> {
        self.store.as_ref()
    }

    /// The title the sidebar shows for the open note (or the draft) instead of its file name.
    pub fn open_title(&self) -> Option<&str> {
        self.open_title.as_deref()
    }

    /// Sets the title shown for the open note while its title line is edited but the file not
    /// renamed yet (ADR 0061), or for the draft. `None` shows the file name again.
    pub fn set_open_title(&mut self, title: Option<String>, cx: &mut Context<Self>) {
        if self.open_title != title {
            self.open_title = title;
            cx.notify();
        }
    }

    /// Records that the note was just saved: updates its metadata and moves it to the top,
    /// adding it back if it was missing (e.g. deleted by another program and saved again).
    pub fn note_saved(&mut self, note: Note, cx: &mut Context<Self>) {
        // A save of the previous folder that finished after the switch, or of a file that is
        // not a note.
        if note.path.parent() != Some(self.location.dir.as_path()) || !is_note_path(&note.path) {
            return;
        }
        self.changes += 1;
        let notes = Arc::make_mut(&mut self.notes);
        notes.retain(|listed| listed.path != note.path);
        notes.insert(0, note);
        cx.notify();
    }

    /// Drops a note that no longer exists from the list. If it was open, the next note is
    /// opened instead, as after a delete. A file from outside the folder that is gone or cannot
    /// be read is closed, and the newest note opens.
    pub fn forget(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.selection == Selection::File(path.to_owned()) {
            self.selection = Selection::None;
            // Before the first listing, it opens the newest note.
            self.open_after_listing = !self.loaded;
            if self.loaded {
                self.open_newest(cx);
            }
            cx.notify();
            return;
        }
        let neighbour = self.neighbour_of(path);
        self.changes += 1;
        Arc::make_mut(&mut self.notes).retain(|note| note.path != path);
        if let Some(hits) = &mut self.hits {
            hits.retain(|hit| hit.path != path);
        }
        if self.selection == Selection::Note(path.to_owned()) {
            match neighbour {
                Some(next) => self.select(&next, cx),
                None => self.selection = Selection::None,
            }
        }
        cx.notify();
    }

    pub fn has_draft(&self) -> bool {
        matches!(self.selection, Selection::Draft(_))
    }

    /// The search text as typed.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether the sidebar shows search results instead of the note list.
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Matches for [`query`](Self::query), title matches first. `None` until the first results
    /// for a new search arrive.
    pub fn search_hits(&self) -> Option<&[SearchHit]> {
        self.hits.as_deref()
    }

    /// Searches titles and contents for `query` in the background (PLAN §28). An empty query
    /// returns to the note list.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.query == query {
            return;
        }
        self.query = query.to_owned();
        if !self.is_searching() {
            self.hits = None;
            // The cache holds the text of every note searched (up to 64 MiB) and would stay for
            // the rest of the run. A search after this one reads the notes again, off the UI
            // thread (~130 ms for 1,000 notes).
            self.search = Arc::default();
        }
        self.search(cx);
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let (Some(store), true) = (self.store.clone(), self.is_searching()) else {
            self.search_task = None;
            return;
        };
        let notes = self.notes.clone();
        let query = self.query.clone();
        let search = self.search.clone();
        // The first search reads every note, so it must not block the UI thread.
        let searching = cx.background_spawn(async move {
            let mut search = search.lock().unwrap_or_else(PoisonError::into_inner);
            search.search(&store, &notes, &query)
        });
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let hits = searching.await;
            this.update(cx, |this, cx| {
                this.hits = Some(hits);
                cx.notify();
            })
            .ok();
        }));
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
        self.open_after_listing = false;
        self.open_title = None;
        cx.emit(NotesEvent::OpenNote(path.to_owned()));
        cx.notify();
    }

    /// Opens the file at `path`: a note of this folder is selected like a click in the list;
    /// any other file is opened in place without being listed.
    pub fn open_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(note) = self.note_path(path) {
            self.select(&note, cx);
            return;
        }
        if matches!(&self.selection, Selection::File(open) if open == path) {
            return;
        }
        self.selection = Selection::File(path.to_owned());
        self.open_after_listing = false;
        self.open_title = None;
        cx.emit(NotesEvent::OpenFile(path.to_owned()));
        cx.notify();
    }

    /// The path of the note `path` names if it is one of this folder: as listed, which may
    /// differ in case from how a file dialog spells it.
    pub fn note_path(&self, path: &Path) -> Option<PathBuf> {
        let name = path.file_name()?;
        let in_folder = path
            .parent()
            .is_some_and(|dir| same_file(dir, &self.location.dir));
        if !(in_folder && is_note_path(path)) {
            return None;
        }
        let path = self.location.dir.join(name);
        let listed = self.notes.iter().find(|note| same_file(&note.path, &path));
        Some(listed.map_or(path, |note| note.path.clone()))
    }

    /// Makes the file just written by Save As the open one, without an event: the editor
    /// already shows its text. A note of this folder is listed and selected; any other file
    /// is selected as with [`open_file`](Self::open_file).
    pub fn saved_as(&mut self, note: Note, cx: &mut Context<Self>) {
        self.open_title = None;
        self.open_after_listing = false;
        self.selection = if self.note_path(&note.path).is_some() {
            let path = note.path.clone();
            self.note_saved(note, cx);
            Selection::Note(path)
        } else {
            Selection::File(note.path)
        };
        cx.notify();
    }

    /// Starts a new note in memory and asks for it to be opened. Nothing is written until
    /// [`save_draft`](Self::save_draft), so pressing Ctrl+N and leaving leaves no empty file.
    pub fn new_note(&mut self, cx: &mut Context<Self>) {
        // The draft is shown in the note list, not among search results.
        self.set_query("", cx);
        self.last_draft += 1;
        let draft = DraftId(self.last_draft);
        self.selection = Selection::Draft(draft);
        self.open_after_listing = false;
        self.open_title = None;
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
        let note = self.opened_store()?.create(title)?;
        self.changes += 1;
        Arc::make_mut(&mut self.notes).insert(0, note.clone());
        if self.selection == Selection::Draft(draft) {
            self.selection = Selection::Note(note.path.clone());
        }
        self.search(cx);
        cx.notify();
        Ok(note)
    }

    /// Renames the note's file after `title` (sanitized and made unique by the store, so
    /// "Ideas" may become "Ideas 2") and returns the new path.
    pub fn rename(
        &mut self,
        path: &Path,
        title: &str,
        cx: &mut Context<Self>,
    ) -> scratchpad_core::Result<PathBuf> {
        let new_path = self.opened_store()?.rename(path, title)?;
        if new_path == path {
            return Ok(new_path);
        }
        self.changes += 1;
        // Renaming keeps the modification time, so the note keeps its place in the list.
        match self.opened_store()?.note(&new_path) {
            Ok(renamed) => {
                let notes = Arc::make_mut(&mut self.notes);
                if let Some(note) = notes.iter_mut().find(|note| note.path == path) {
                    *note = renamed;
                }
            }
            Err(_) => self.refresh(cx),
        }
        if let Some(hit) = self.hit_mut(path) {
            hit.path = new_path.clone();
            hit.title = title_of(&new_path);
            hit.title_ranges.clear();
        }
        if self.selection == Selection::Note(path.to_owned()) {
            self.selection = Selection::Note(new_path.clone());
        }
        cx.emit(NotesEvent::Renamed {
            from: path.to_owned(),
            to: new_path.clone(),
        });
        self.search(cx);
        cx.notify();
        Ok(new_path)
    }

    /// Moves the note to the recycle bin (PLAN §41). If it was open, the next note in the
    /// sidebar (or the previous one at the end) is opened instead.
    pub fn delete(&mut self, path: &Path, cx: &mut Context<Self>) -> scratchpad_core::Result<()> {
        let neighbour = self.neighbour_of(path);
        self.opened_store()?.delete(path)?;
        self.changes += 1;
        Arc::make_mut(&mut self.notes).retain(|note| note.path != path);
        if let Some(hits) = &mut self.hits {
            hits.retain(|hit| hit.path != path);
        }
        cx.emit(NotesEvent::Deleted(path.to_owned()));
        if self.selection == Selection::Note(path.to_owned()) {
            match neighbour {
                Some(next) => self.select(&next, cx),
                None => self.selection = Selection::None,
            }
        }
        cx.notify();
        Ok(())
    }

    /// The paths in the order the sidebar shows them.
    pub fn visible_paths(&self) -> Vec<&Path> {
        match (&self.hits, self.is_searching()) {
            (Some(hits), true) => hits.iter().map(|hit| hit.path.as_path()).collect(),
            (None, true) => Vec::new(),
            (_, false) => self.notes.iter().map(|note| note.path.as_path()).collect(),
        }
    }

    fn neighbour_of(&self, path: &Path) -> Option<PathBuf> {
        let visible = self.visible_paths();
        let ix = visible.iter().position(|visible| *visible == path)?;
        visible
            .get(ix + 1)
            .or_else(|| ix.checked_sub(1).and_then(|previous| visible.get(previous)))
            .map(|path| path.to_path_buf())
    }

    fn hit_mut(&mut self, path: &Path) -> Option<&mut SearchHit> {
        self.hits.as_mut()?.iter_mut().find(|hit| hit.path == path)
    }

    fn opened_store(&self) -> scratchpad_core::Result<&NoteStore> {
        self.store
            .as_ref()
            .ok_or_else(|| scratchpad_core::Error::Io {
                action: "open",
                path: self.location.dir.clone(),
                source: io::Error::other("the notes folder is not available yet"),
            })
    }

    fn folder_name(&self) -> String {
        folder_name(&self.location.dir)
    }
}

/// Whether two paths name the same file, as far as their spelling tells: Windows ignores case.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let lower = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    a == b || (cfg!(windows) && lower(a) == lower(b))
}

/// The name a folder is shown by in messages: its last component.
pub fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or(dir.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The title of the note at `path`: its file stem (ADR 0013).
pub fn title_of(path: &Path) -> String {
    path.file_stem()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
