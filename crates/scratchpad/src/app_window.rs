use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseDownEvent, Pixels,
    Subscription, Task, Window, div, prelude::*, px,
};
use scratchpad_core::{NoteStore, PAGE_EXTENSIONS, PageIndex, fold_case};
use scratchpad_editor::markdown::{MarkdownOptions, wikilink_page};

use crate::actions::view::ToggleWikilinks;
use crate::actions::view::{
    ResetZoom, ToggleAutoReplace, ToggleAutocorrect, ToggleCallouts, ToggleHeadingIds,
    ToggleSpellCheck, ToggleSubscript, ToggleSuperscript, ToggleWordWrap, ZoomIn, ZoomOut,
};
use crate::actions::{
    CloseWindow, NewNote, OpenFile, OpenSettings, SaveAs, SaveNote, SearchNotes, ToggleSidebar,
    ToggleStatusBar,
};
use crate::app::{Storage, WINDOW_TITLE};
use crate::editor_pane::EditorPane;
use crate::editor_view::{Direction, EditorView, OpenPage};
use crate::find_bar::{FindInNote, FindNext, FindPrevious, ReplaceInNote};
use crate::go_to_line::GoToLine;
use crate::menu_bar::{MenuBar, OpenEditMenu, OpenFileMenu, OpenViewMenu};
use crate::notes::{Notes, NotesLocation, Selection, folder_name, same_file};
use crate::session::Session;
use crate::settings_panel::{SettingsPanel, SettingsPanelEvent};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::theme::{self, ActiveTheme, typography};
use crate::{file_dialogs, settings, toast};

/// The open document and its folder if it is a file from outside the notes folder: its
/// wikilinks resolve there (ADR 0145, 0188).
fn outside_folder(notes: &Notes) -> Option<(PathBuf, PathBuf)> {
    let Selection::File(path) = notes.selection() else {
        return None;
    };
    let dir = path.parent()?;
    (!same_file(dir, &notes.location().dir)).then(|| (path.clone(), dir.to_owned()))
}

/// The file of `page` in `dir` if there is one, `.md` first, found by asking for each name
/// rather than listing the folder: on a case-insensitive file system (NTFS) that also finds it
/// spelled otherwise, and the name is then spelled as on disk. Blocks on the disk.
fn existing_page(dir: &Path, page: &str) -> Option<PathBuf> {
    let path = PAGE_EXTENSIONS
        .iter()
        .map(|extension| dir.join(format!("{page}.{extension}")))
        .find(|path| path.is_file())?;
    let name = path.file_name()?.to_string_lossy();
    let on_disk = fs::canonicalize(&path)
        .ok()
        .and_then(|real| {
            real.file_name()
                .map(|real| real.to_string_lossy().into_owned())
        })
        // A link's target, say, has another name: keep the one asked for.
        .filter(|real| fold_case(real) == fold_case(&name));
    Some(on_disk.map_or_else(|| path.clone(), |real| dir.join(real)))
}

/// How long the pages of another folder's file are trusted before the window becoming active
/// again reads them anew (ADR 0188): that folder is not watched, but going to another program
/// and back is when its files are likely to have changed.
const OUTSIDE_PAGES_MAX_AGE: Duration = Duration::from_secs(2);

/// Narrower windows do not dock the sidebar, so the note keeps a comfortable width: at this
/// width a sidebar of the default 260 px leaves
/// [`MIN_NOTE_WIDTH`](crate::editor_pane::MIN_NOTE_WIDTH) for it, and a wider one
/// narrows to leave as much (ADR 0135).
pub const AUTO_COLLAPSE_WIDTH: Pixels = px(720.);

/// How the sidebar is shown (ADR 0135).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    /// Beside the note.
    Docked,
    /// Not shown: the user collapsed it, or the window is too narrow to dock it.
    Hidden,
    /// Floating over the note until the user is done with it.
    Overlay,
}

/// Root view of the main window: sidebar on the left, editor pane filling the rest (PLAN §42).
pub struct AppWindow {
    focus_handle: FocusHandle,
    notes: Entity<Notes>,
    sidebar: Entity<Sidebar>,
    /// Tracked by the sidebar's container while it is shown over the note, to notice focus
    /// leaving it.
    sidebar_overlay_focus: FocusHandle,
    /// The sidebar was asked for while it is not docked.
    sidebar_overlay: bool,
    /// The window is narrower than [`AUTO_COLLAPSE_WIDTH`].
    narrow: bool,
    editor_pane: Entity<EditorPane>,
    menu_bar: Entity<MenuBar>,
    session: Entity<Session>,
    notes_dir_overridden: bool,
    settings: Option<OpenSettingsPanel>,
    /// Opening a newly picked notes folder; a newer pick replaces (cancels) it.
    folder_change: Option<Task<()>>,
    /// What the title bar says: the name of a file opened from outside the notes folder.
    window_title: String,
    _subscriptions: Vec<Subscription>,
    /// The file from outside the notes folder whose folder's pages the editor's wikilinks
    /// resolve against, while it is open, when those pages were last read, and their reading
    /// (ADR 0188).
    pages_file: Option<PathBuf>,
    pages_read_at: Option<Instant>,
    pages_task: Option<Task<()>>,
    /// The pages of Ctrl+clicked wikilinks being found or created, by [`fold_case`]d name: a
    /// second click on one before it is done is ignored, so it is never created twice.
    pending_pages: HashSet<String>,
    /// Counts Ctrl+clicks on wikilinks; only the page of the latest one opens.
    page_requests: u64,
}

/// The settings panel while it is shown.
struct OpenSettingsPanel {
    panel: Entity<SettingsPanel>,
    _subscriptions: [Subscription; 2],
}

impl AppWindow {
    pub fn new(storage: Storage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let config = settings::get(cx);
        // A remembered note of another folder is not opened (ADR 0081); a file from outside
        // the notes folder is (ADR 0145).
        let last_note = config
            .last_opened_note
            .clone()
            .filter(|note| note.parent() == Some(storage.notes.dir.as_path()));
        let reopen = config
            .last_opened_file
            .iter()
            .cloned()
            .chain(last_note)
            .collect();
        let sidebar_width = config.sidebar_width;
        let sidebar_collapsed = config.sidebar_collapsed;
        let (zoom, word_wrap) = (config.zoom_percent, config.word_wrap);
        let (spell_check, autocorrect) = (config.spell_check, config.autocorrect);
        let auto_replace = config.auto_replace;
        let markdown_options = settings::markdown_options(config);
        let notes = cx.new(|cx| {
            let mut notes = Notes::new(storage.notes.clone(), reopen, cx);
            // Before the folder is opened and listed: off, the listing skips the pages.
            notes.set_wikilinks(markdown_options.wikilinks, cx);
            notes
        });
        let editor = cx.new(|cx| {
            let mut editor = EditorView::new("", window, cx);
            editor.set_zoom_percent(zoom.unwrap_or(100), cx);
            editor.set_soft_wrap(word_wrap.unwrap_or(true), cx);
            editor.set_spell_check(spell_check.unwrap_or(true), cx);
            editor.set_autocorrect(autocorrect.unwrap_or(true), cx);
            editor.set_auto_replace(auto_replace.unwrap_or(true), cx);
            editor.set_markdown_options(markdown_options, cx);
            editor
        });
        let session =
            cx.new(|cx| Session::new(notes.clone(), editor.clone(), &storage, window, cx));
        let editor_pane = cx.new(|cx| EditorPane::new(editor.clone(), session.clone(), window, cx));
        window.focus(&editor.focus_handle(cx));
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(notes.clone(), window, cx);
            if let Some(width) = sidebar_width {
                sidebar.set_width(px(width), cx);
            }
            sidebar
        });
        let sidebar_overlay_focus = cx.focus_handle();
        let narrow = is_narrow(window);
        let sidebar_hidden = narrow || sidebar_collapsed;
        editor_pane.update(cx, |pane, cx| pane.set_sidebar_button(sidebar_hidden, cx));
        let menu_bar = cx.new(|cx| MenuBar::new(editor.clone(), !sidebar_hidden, cx));

        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::system_appearance_changed(window.appearance(), cx);
            }),
            cx.subscribe_in(&sidebar, window, |this, _, event, window, cx| match event {
                SidebarEvent::FocusEditor => window.focus(&this.editor_pane.focus_handle(cx)),
                SidebarEvent::NoteChosen => {
                    if this.sidebar_mode(cx) == SidebarMode::Overlay {
                        this.hide_sidebar_overlay(window, cx);
                    }
                }
            }),
            // Clicking the note, Escape in the search field, Ctrl+N...: the user is done with
            // the sidebar. Another window becoming active (e.g. the delete confirmation) is not.
            cx.on_focus_out(&sidebar_overlay_focus, window, |this, _, window, cx| {
                if window.is_window_active() {
                    // A menu that took focus from it cannot give focus back to it.
                    this.menu_bar
                        .update(cx, |bar, _| bar.return_focus_to_note());
                    this.hide_sidebar_overlay(window, cx);
                }
            }),
            // Focus was put back on something no longer shown, e.g. by a menu opened from the
            // sidebar, which the window then became too narrow for: the keys go to the note.
            // Deferred: this runs while the window is drawn, when focusing would not redraw it.
            cx.on_focus_lost(window, |_, window, cx| {
                cx.defer_in(window, |this, window, cx| {
                    window.focus(&this.editor_pane.focus_handle(cx))
                })
            }),
            // Another program may have changed the pages of a file's folder, which is not watched.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.update_pages(true, cx);
                }
            }),
            cx.subscribe(&editor, |this, _, OpenPage(page), cx| {
                this.open_page(page.clone(), cx)
            }),
            cx.observe_in(&notes, window, |this, notes, window, cx| {
                this.update_pages(false, cx);
                this.remember_pins(cx);
                let selection = notes.read(cx).selection().clone();
                this.show_file_name(&selection, window);
                match selection {
                    Selection::Note(path) => settings::update(cx, |config| {
                        config.last_opened_note = Some(path);
                        config.last_opened_file = None;
                    }),
                    Selection::File(path) => {
                        settings::update(cx, |config| config.last_opened_file = Some(path))
                    }
                    Selection::Draft(_) => {
                        settings::update(cx, |config| config.last_opened_file = None)
                    }
                    Selection::None => {}
                }
            }),
            cx.observe(&sidebar, |_, sidebar, cx| {
                let width = f32::from(sidebar.read(cx).width());
                settings::update(cx, |config| config.sidebar_width = Some(width));
            }),
            // Zoom, word wrap, spelling and Markdown options belong to the editor; checked on each
            // of its updates, written only when they change.
            cx.observe(&editor, |this, editor, cx| {
                let editor = editor.read(cx);
                let (zoom, wrap) = (editor.zoom_percent(), editor.soft_wrap());
                let (spell_check, autocorrect) = (editor.spell_check(), editor.autocorrect());
                let auto_replace = editor.auto_replace();
                let markdown_options = editor.markdown_options();
                // The notes model keeps the folder's pages only while wikilinks are on.
                this.notes.update(cx, |notes, cx| {
                    notes.set_wikilinks(markdown_options.wikilinks, cx)
                });
                let config = settings::get(cx);
                if config.zoom_percent.unwrap_or(100) != zoom
                    || config.word_wrap.unwrap_or(true) != wrap
                    || config.spell_check.unwrap_or(true) != spell_check
                    || config.autocorrect.unwrap_or(true) != autocorrect
                    || config.auto_replace.unwrap_or(true) != auto_replace
                    || settings::markdown_options(config) != markdown_options
                {
                    settings::update(cx, |config| {
                        config.zoom_percent = Some(zoom);
                        config.word_wrap = Some(wrap);
                        config.spell_check = Some(spell_check);
                        config.autocorrect = Some(autocorrect);
                        config.auto_replace = Some(auto_replace);
                        settings::store_markdown_options(config, markdown_options);
                    });
                }
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                let bounds = window_bounds(window);
                settings::update(cx, |config| config.window = Some(bounds));
                let narrow = is_narrow(window);
                if narrow != this.narrow {
                    this.narrow = narrow;
                    this.sidebar_changed(window, cx);
                }
            }),
            // Quitting (Ctrl+Q, or the last window closing) runs this before the app exits.
            cx.on_app_quit(|this, cx| {
                this.save_all(cx);
                async {}
            }),
        ];
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            this.update(cx, |this, cx| this.save_all(cx)).ok();
            true
        });

        Self {
            focus_handle,
            sidebar,
            sidebar_overlay_focus,
            sidebar_overlay: false,
            narrow,
            notes,
            editor_pane,
            menu_bar,
            session,
            notes_dir_overridden: storage.notes_dir_overridden,
            settings: None,
            folder_change: None,
            window_title: WINDOW_TITLE.to_owned(),
            _subscriptions: subscriptions,
            pages_file: None,
            pages_read_at: None,
            pages_task: None,
            pending_pages: HashSet::new(),
            page_requests: 0,
        }
    }

    /// Gives the editor the pages its wikilinks resolve against (ADR 0188): the notes model's
    /// index of the notes folder, kept in step with the note list, or for a file from another
    /// folder that folder's pages, read in the background when a file from it opens and, with
    /// `reread`, again if they are older than [`OUTSIDE_PAGES_MAX_AGE`] (the window became
    /// active). With wikilinks off, none.
    fn update_pages(&mut self, reread: bool, cx: &mut Context<Self>) {
        let editor = self.editor_pane.read(cx).editor().clone();
        let notes = self.notes.read(cx);
        let outside = outside_folder(notes).filter(|_| notes.wikilinks());
        let Some((file, dir)) = outside else {
            self.pages_file = None;
            self.pages_read_at = None;
            self.pages_task = None;
            // Until the folder is listed every page would look missing.
            let pages = (notes.wikilinks() && notes.is_loaded()).then(|| notes.pages().clone());
            editor.update(cx, |editor, cx| editor.set_pages(pages, cx));
            return;
        };
        let same_file = self.pages_file.as_ref() == Some(&file);
        // The executor's clock, which tests can advance.
        let now = cx.background_executor().now();
        let stale = self
            .pages_read_at
            .is_none_or(|read_at| now.duration_since(read_at) >= OUTSIDE_PAGES_MAX_AGE);
        if same_file && !(reread && stale) {
            return;
        }
        if !same_file {
            self.pages_file = Some(file);
            editor.update(cx, |editor, cx| editor.set_pages(None, cx));
        }
        self.pages_read_at = Some(now);
        let reading = cx.background_spawn(async move { PageIndex::read(&dir) });
        self.pages_task = Some(cx.spawn(async move |_, cx| match reading.await {
            Ok(pages) => {
                editor
                    .update(cx, |editor, cx| editor.set_pages(Some(Arc::new(pages)), cx))
                    .ok();
            }
            Err(error) => tracing::warn!(%error, "not resolving wikilinks"),
        }));
    }

    /// Ctrl+click on a wikilink: opens the page in the folder of the open document as choosing
    /// it in the sidebar or with File > Open would; a page that does not exist is created there
    /// first, as a note named after it starting with it as its heading. A page still being
    /// found or created is not asked for again; of several pages, the last one asked for opens.
    fn open_page(&mut self, page: String, cx: &mut Context<Self>) {
        // Only what the editor draws as a link: anything else could name a file elsewhere.
        if wikilink_page(&page) != Some(page.as_str()) {
            return;
        }
        let notes = self.notes.read(cx);
        let outside = outside_folder(notes);
        let known = match &outside {
            None => notes.is_loaded().then(|| notes.pages().clone()),
            Some(_) => self.editor_pane.read(cx).editor().read(cx).pages().cloned(),
        };
        if let Some(path) = known.as_ref().and_then(|pages| pages.resolve(&page)) {
            self.page_requests += 1;
            self.notes
                .update(cx, |notes, cx| notes.open_file(&path, cx));
            return;
        }
        let pending = fold_case(&page);
        if !self.pending_pages.insert(pending.clone()) {
            return;
        }
        self.page_requests += 1;
        // Not known to exist: look on the disk, which may have changed since it was listed, and
        // create the note only if it is still missing there.
        let dir = outside
            .clone()
            .map_or_else(|| notes.location().dir.clone(), |(_, dir)| dir);
        let store = outside.is_none().then(|| notes.store().cloned()).flatten();
        let name = page.clone();
        let finding = cx.background_spawn(async move {
            if let Some(path) = existing_page(&dir, &name) {
                return Ok((path, None));
            }
            let store = match store {
                Some(store) => store,
                None => NoteStore::open(&dir)?,
            };
            let note = store.create(Some(&name))?;
            store.save(&note.path, &format!("# {name}\n"))?;
            let note = store.note(&note.path)?;
            scratchpad_core::Result::Ok((note.path.clone(), Some(note)))
        });
        let request = self.page_requests;
        // Not cancelled by a later click: a note it creates is listed either way.
        cx.spawn(async move |this, cx| {
            let found = finding.await;
            this.update(cx, |this, cx| {
                this.pending_pages.remove(&pending);
                let latest = this.page_requests == request;
                match found {
                    Ok((path, created)) => this.notes.update(cx, |notes, cx| {
                        if let Some(note) = created {
                            // Lists a note of the notes folder; ignores any other file.
                            notes.note_saved(note, cx);
                        }
                        if latest {
                            notes.open_file(&path, cx);
                        }
                    }),
                    Err(error) => toast::show_file_error(&page, &error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the page of a Ctrl+clicked wikilink is still being found or created.
    pub fn is_opening_page(&self) -> bool {
        !self.pending_pages.is_empty()
    }

    /// Writes the pinned notes to the settings, keeping the pins of other folders.
    fn remember_pins(&self, cx: &mut Context<Self>) {
        let notes = self.notes.read(cx);
        let dir = notes.location().dir.clone();
        let pinned = notes.pinned().to_vec();
        settings::update(cx, |config| {
            config
                .pinned_notes
                .retain(|path| path.parent() != Some(dir.as_path()));
            config.pinned_notes.extend(pinned);
        });
    }

    /// The settings panel, while it is open.
    pub fn settings_panel(&self) -> Option<&Entity<SettingsPanel>> {
        self.settings.as_ref().map(|settings| &settings.panel)
    }

    pub fn menu_bar(&self) -> &Entity<MenuBar> {
        &self.menu_bar
    }

    pub fn editor_pane(&self) -> &Entity<EditorPane> {
        &self.editor_pane
    }

    /// The note list and the open note. The editor integration subscribes to its events.
    pub fn notes(&self) -> &Entity<Notes> {
        &self.notes
    }

    pub fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    /// The open note's loading, saving and recovery.
    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    /// Calls `callback` once, when the first note opened has been loaded into the editor.
    pub fn on_first_note_shown(&self, cx: &mut Context<Self>, callback: impl FnOnce() + 'static) {
        self.session
            .update(cx, |session, _| session.on_first_load(callback));
    }

    /// Writes the open note, pending saves and the settings before the window goes away.
    fn save_all(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.flush_sync(cx));
        settings::save_now(cx);
    }

    // Window-wide commands live on the root so they work wherever focus is.
    fn new_note(&mut self, _: &NewNote, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.new_note(window, cx));
    }

    fn save_note(&mut self, _: &SaveNote, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| session.flush(cx));
    }

    /// Ctrl+O: a note of the notes folder opens as a note, any other file in place.
    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        // The dialog leaves focus where it is: the settings would stay over the file.
        self.close_settings(window, cx);
        let picked = file_dialogs::pick_file(cx);
        let notes = self.notes.clone();
        cx.spawn(async move |_, cx| {
            if let Some(path) = picked.await {
                notes
                    .update(cx, |notes, cx| notes.open_file(&path, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Ctrl+Shift+S: writes the open note or file to a file the user picks and edits that.
    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.close_settings(window, cx);
        let session = self.session.clone();
        let Some((dir, name)) = session.update(cx, |session, cx| session.begin_save_as(cx)) else {
            return;
        };
        let picked = file_dialogs::pick_new_path(&dir, &name, cx);
        cx.spawn(async move |_, cx| {
            let path = picked.await;
            session
                .update(cx, |session, cx| session.finish_save_as(path, cx))
                .ok();
        })
        .detach();
    }

    /// Names a file from outside the notes folder in the title bar, where the sidebar's list
    /// does not show it.
    fn show_file_name(&mut self, selection: &Selection, window: &mut Window) {
        let title = match selection {
            Selection::File(path) => match path.file_name() {
                Some(name) => format!("{} - {WINDOW_TITLE}", name.to_string_lossy()),
                None => WINDOW_TITLE.to_owned(),
            },
            Selection::Note(_) | Selection::Draft(_) | Selection::None => WINDOW_TITLE.to_owned(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    fn search_notes(&mut self, _: &SearchNotes, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_mode(cx) == SidebarMode::Hidden {
            self.sidebar_overlay = true;
            self.sidebar_changed(window, cx);
        }
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.focus_search(window, cx));
    }

    fn find_in_note(&mut self, _: &FindInNote, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane
            .update(cx, |pane, cx| pane.find(window, cx));
    }

    /// Zoom and word wrap act on the note wherever focus is.
    fn update_editor(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut EditorView, &mut Context<EditorView>),
    ) {
        let editor = self.editor_pane.read(cx).editor().clone();
        editor.update(cx, update);
    }

    /// Turns one of the Markdown options (View > Markdown features) on or off in the note.
    fn toggle_markdown_option(
        &self,
        cx: &mut Context<Self>,
        option: fn(&mut MarkdownOptions) -> &mut bool,
    ) {
        self.update_editor(cx, |editor, cx| {
            let mut options = editor.markdown_options();
            let on = option(&mut options);
            *on = !*on;
            editor.set_markdown_options(options, cx);
        })
    }

    // On the root, not the editor pane, so that F3 (or a menu) also works from the sidebar.
    fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane.update(cx, |pane, cx| {
            pane.select_match(Direction::Next, window, cx)
        });
    }

    fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane.update(cx, |pane, cx| {
            pane.select_match(Direction::Previous, window, cx)
        });
    }

    fn replace_in_note(&mut self, _: &ReplaceInNote, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane
            .update(cx, |pane, cx| pane.replace(window, cx));
    }

    fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_pane
            .update(cx, |pane, cx| pane.go_to_line(window, cx));
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        window.remove_window();
    }

    // --- Sidebar (ADR 0135) ---

    /// How the sidebar is shown right now.
    pub fn sidebar_mode(&self, cx: &App) -> SidebarMode {
        if !self.narrow && !settings::get(cx).sidebar_collapsed {
            SidebarMode::Docked
        } else if self.sidebar_overlay {
            SidebarMode::Overlay
        } else {
            SidebarMode::Hidden
        }
    }

    /// Ctrl+\ and the sidebar buttons. Collapsing or expanding the docked sidebar is remembered;
    /// in a narrow window the sidebar is only shown over the note for a while.
    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        match self.sidebar_mode(cx) {
            SidebarMode::Docked => settings::update(cx, |config| config.sidebar_collapsed = true),
            SidebarMode::Overlay => self.sidebar_overlay = false,
            SidebarMode::Hidden if self.narrow => {
                self.sidebar_overlay = true;
                self.sidebar.read(cx).focus_list(window);
            }
            SidebarMode::Hidden => settings::update(cx, |config| config.sidebar_collapsed = false),
        }
        self.sidebar_changed(window, cx);
    }

    fn hide_sidebar_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_overlay {
            self.sidebar_overlay = false;
            self.sidebar_changed(window, cx);
        }
    }

    /// Brings the rest of the window in line with a change of [`sidebar_mode`](Self::sidebar_mode).
    fn sidebar_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.sidebar_mode(cx);
        if mode == SidebarMode::Docked {
            // E.g. the window was widened while the sidebar was shown over the note.
            self.sidebar_overlay = false;
        }
        // The note moves from under its context menu.
        let editor = self.editor_pane.read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.close_context_menu(cx));
        let hidden = mode == SidebarMode::Hidden;
        if hidden && self.sidebar.read(cx).contains_focus(window, cx) {
            window.focus(&self.editor_pane.focus_handle(cx));
        }
        self.editor_pane
            .update(cx, |pane, cx| pane.set_sidebar_button(hidden, cx));
        self.menu_bar
            .update(cx, |bar, cx| bar.set_sidebar_shown(!hidden, cx));
        cx.notify();
    }

    fn render_sidebar_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .size_full()
            // Clicking anywhere else only closes it, as with any flyout.
            .child(
                div()
                    .id("sidebar-overlay-backdrop")
                    .debug_selector(|| "sidebar-overlay-backdrop".into())
                    .absolute()
                    .size_full()
                    .occlude()
                    .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.hide_sidebar_overlay(window, cx)
                    })),
            )
            .child(
                div()
                    .track_focus(&self.sidebar_overlay_focus)
                    .absolute()
                    .h_full()
                    .occlude()
                    .shadow_lg()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.hide_sidebar_overlay(window, cx);
                        }
                    }))
                    .child(self.sidebar.clone()),
            )
    }

    /// Shows or hides the status bar under the note (ADR 0137).
    fn toggle_status_bar(&mut self, _: &ToggleStatusBar, _: &mut Window, cx: &mut Context<Self>) {
        settings::update(cx, |config| {
            config.status_bar_hidden = !config.status_bar_hidden;
        });
        cx.notify();
    }

    /// Alt+F, Alt+E, Alt+V and F10 (ADR 0150).
    fn open_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_bar
            .update(cx, |bar, cx| bar.open(index, true, window, cx));
    }

    // --- Settings ---

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_some() {
            return;
        }
        let panel = cx.new(|cx| {
            SettingsPanel::new(self.notes.clone(), self.notes_dir_overridden, window, cx)
        });
        let panel_focus = panel.focus_handle(cx);
        let subscriptions = [
            cx.subscribe_in(&panel, window, |this, _, event, window, cx| match event {
                SettingsPanelEvent::Close => this.close_settings(window, cx),
                SettingsPanelEvent::NotesFolderPicked(dir) => {
                    this.change_notes_folder(dir.clone(), window, cx)
                }
            }),
            // E.g. Ctrl+N or Ctrl+P while it is open: the panel goes and focus stays there.
            // Another window becoming active (such as the folder dialog, whose answer the
            // panel waits for) leaves it open.
            cx.on_focus_out(&panel_focus, window, |this, _, window, cx| {
                if window.is_window_active() {
                    this.settings = None;
                    cx.notify();
                }
            }),
        ];
        self.settings = Some(OpenSettingsPanel {
            panel,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Closes the settings panel, if open, and puts the caret back in the note.
    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            window.focus(&self.editor_pane.focus_handle(cx));
            cx.notify();
        }
    }

    /// Switches to the notes in `dir` if notes can be written there; otherwise shows why and
    /// stays on the current folder. The folder is opened off the UI thread: it may be slow,
    /// e.g. on a network drive.
    fn change_notes_folder(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.notes.read(cx).location();
        if dir == current.dir {
            // Also the last word on a slow switch to another folder that is still running.
            self.folder_change = None;
            self.close_settings(window, cx);
            return;
        }
        let location = NotesLocation {
            dir,
            ..current.clone()
        };
        let opening = cx.background_spawn({
            let location = location.clone();
            async move { location.open_writable() }
        });
        self.folder_change = Some(cx.spawn_in(window, async move |this, cx| {
            let opened = opening.await;
            this.update_in(cx, |this, window, cx| match opened {
                Ok(store) => this.switch_notes_folder(location, store, window, cx),
                Err(error) => toast::show_file_error(&folder_name(&location.dir), &error, cx),
            })
            .ok();
        }));
    }

    fn switch_notes_folder(
        &mut self,
        location: NotesLocation,
        store: NoteStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = location.dir.clone();
        tracing::info!(dir = %dir.display(), "changing the notes folder");
        // The open note, new notes and pending renames go to the old folder first.
        self.session
            .update(cx, |session, cx| session.change_folder(dir.clone(), cx));
        self.notes
            .update(cx, |notes, cx| notes.change_folder(location, store, cx));
        settings::update(cx, |config| config.notes_dir = Some(dir));
        self.close_settings(window, cx);
    }
}

fn is_narrow(window: &Window) -> bool {
    window.viewport_size().width < AUTO_COLLAPSE_WIDTH
}

/// The window's restorable bounds (the normal size and position even while maximized).
fn window_bounds(window: &Window) -> scratchpad_core::WindowBounds {
    let (bounds, maximized) = match window.window_bounds() {
        gpui::WindowBounds::Windowed(bounds) => (bounds, false),
        gpui::WindowBounds::Maximized(bounds) | gpui::WindowBounds::Fullscreen(bounds) => {
            (bounds, true)
        }
    };
    scratchpad_core::WindowBounds {
        x: bounds.origin.x.into(),
        y: bounds.origin.y.into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
        maximized,
    }
}

impl Focusable for AppWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.sidebar_mode(cx);
        let theme = cx.theme();
        div()
            .key_context("AppWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::new_note))
            .on_action(cx.listener(Self::save_note))
            .on_action(cx.listener(Self::search_notes))
            .on_action(cx.listener(Self::find_in_note))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::replace_in_note))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::toggle_status_bar))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.update_editor(cx, |editor, cx| editor.zoom_by(1, cx))
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.update_editor(cx, |editor, cx| editor.zoom_by(-1, cx))
            }))
            .on_action(cx.listener(|this, _: &ResetZoom, _, cx| {
                this.update_editor(cx, |editor, cx| editor.set_zoom_percent(100, cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleWordWrap, _, cx| {
                this.update_editor(cx, |editor, cx| {
                    editor.set_soft_wrap(!editor.soft_wrap(), cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleSpellCheck, _, cx| {
                this.update_editor(cx, |editor, cx| {
                    editor.set_spell_check(!editor.spell_check(), cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleAutocorrect, _, cx| {
                this.update_editor(cx, |editor, cx| {
                    editor.set_autocorrect(!editor.autocorrect(), cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleAutoReplace, _, cx| {
                this.update_editor(cx, |editor, cx| {
                    editor.set_auto_replace(!editor.auto_replace(), cx)
                })
            }))
            .on_action(cx.listener(|this, _: &ToggleCallouts, _, cx| {
                this.toggle_markdown_option(cx, |options| &mut options.callouts)
            }))
            .on_action(cx.listener(|this, _: &ToggleSuperscript, _, cx| {
                this.toggle_markdown_option(cx, |options| &mut options.superscript)
            }))
            .on_action(cx.listener(|this, _: &ToggleSubscript, _, cx| {
                this.toggle_markdown_option(cx, |options| &mut options.subscript)
            }))
            .on_action(cx.listener(|this, _: &ToggleHeadingIds, _, cx| {
                this.toggle_markdown_option(cx, |options| &mut options.heading_ids)
            }))
            .on_action(cx.listener(|this, _: &ToggleWikilinks, _, cx| {
                this.toggle_markdown_option(cx, |options| &mut options.wikilinks)
            }))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::save_as))
            .on_action(
                cx.listener(|this, _: &OpenFileMenu, window, cx| this.open_menu(0, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenEditMenu, window, cx| this.open_menu(1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenViewMenu, window, cx| this.open_menu(2, window, cx)),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(typography::UI_FONT_FAMILY)
            .text_size(typography::UI_FONT_SIZE)
            .child(self.menu_bar.clone())
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .when(mode == SidebarMode::Docked, |row| {
                        row.child(self.sidebar.clone())
                    })
                    .child(self.editor_pane.clone())
                    .when(mode == SidebarMode::Overlay, |row| {
                        row.child(self.render_sidebar_overlay(cx))
                    }),
            )
            .children(self.settings_panel().cloned())
            .children(toast::render(cx))
    }
}
