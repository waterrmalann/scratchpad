//! Note navigation: the search field, the "New Note" button and the note list grouped by date
//! or, while searching, the matching notes (PLAN §8, §28, §42-43). See ADR 0051.
//!
//! Click opens a note, double-click renames it in place, right-click shows Rename / Delete /
//! Show in Folder. With the list focused, Up/Down open the previous/next note, Enter moves
//! into the note, F2 renames and Delete deletes it after asking. Dragging the right edge
//! resizes it; the button left of the search field hides it (ADR 0135).

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, NaiveDate};
use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, Div, DragMoveEvent, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, Point, PromptLevel, ScrollStrategy, Stateful, StyledText,
    Subscription, UniformListScrollHandle, Window, anchored, deferred, div, prelude::*, px,
    uniform_list,
};
use scratchpad_core::{DateGroup, Note, local_date};

use crate::actions::{
    DeleteNote, FocusOpenNote, RenameNote, SelectNextNote, SelectPreviousNote, ToggleSidebar,
};
use crate::notes::{Notes, NotesEvent, Selection, title_of};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::{ActiveTheme, Theme, typography};
use crate::toast;

pub const DEFAULT_SIDEBAR_WIDTH: Pixels = px(260.);
pub const MIN_SIDEBAR_WIDTH: Pixels = px(180.);
pub const MAX_SIDEBAR_WIDTH: Pixels = px(480.);
/// Width of the invisible strip along the right edge that resizes the sidebar.
const RESIZE_HANDLE_WIDTH: Pixels = px(5.);

/// Every list row has the same height so the list can be virtualized with `uniform_list`.
/// Section headers put their label at the bottom, so the space above it separates sections.
const ROW_HEIGHT: Pixels = px(46.);

/// Segoe MDL2 Assets glyphs; plain text elsewhere.
const ADD_ICON: &str = if cfg!(windows) { "\u{E710}" } else { "+" };
const SEARCH_ICON: &str = if cfg!(windows) { "\u{E721}" } else { "" };
/// The button Windows apps show and hide their navigation pane with (GlobalNavigationButton).
const SIDEBAR_ICON: &str = if cfg!(windows) { "\u{E700}" } else { "=" };

/// One line of the note list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Header(DateGroup),
    Draft,
    /// Index into [`Notes::notes`].
    Note(usize),
    /// Index into [`Notes::search_hits`].
    Hit(usize),
}

/// A note title being edited in place.
struct Rename {
    path: PathBuf,
    input: Entity<TextInput>,
    _subscriptions: [Subscription; 2],
}

/// The delete confirmation while it is open.
struct DeletePrompt {
    /// The note it asks about, under its current name. Forgotten once the note leaves the list
    /// (deleted here or by another program), so the answer can never delete another note that
    /// takes its name.
    note: Option<PathBuf>,
}

struct ContextMenu {
    path: PathBuf,
    position: Point<Pixels>,
    focus: FocusHandle,
    /// Closes it when focus moves elsewhere (Ctrl+P, the sidebar being hidden...).
    _blur: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuItem {
    Rename,
    Delete,
    ShowInFolder,
}

impl MenuItem {
    fn label(self) -> &'static str {
        match self {
            MenuItem::Rename => "Rename",
            MenuItem::Delete => "Delete",
            MenuItem::ShowInFolder => "Show in Folder",
        }
    }
}

/// Drag payload (and its invisible drag preview) while resizing the sidebar.
struct DraggedEdge;

impl Render for DraggedEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    /// Enter was pressed in the note list: put the caret in the open note.
    FocusEditor,
    /// A note was opened by clicking it or with Enter in the search field (not by browsing
    /// with the arrow keys).
    NoteChosen,
}

pub struct Sidebar {
    notes: Entity<Notes>,
    width: Pixels,
    search: Entity<TextInput>,
    /// Where Escape in the search field returns focus to.
    focus_before_search: Option<FocusHandle>,
    list_focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The list rows, rebuilt only when the notes change or the day does (the groups are
    /// relative to today), so frames (hover, caret, scrolling) cost the same with 10 or
    /// 10,000 notes.
    rows: Rc<[Row]>,
    rows_date: NaiveDate,
    rename: Option<Rename>,
    menu: Option<ContextMenu>,
    delete_prompt: Option<DeletePrompt>,
    _subscriptions: [Subscription; 3],
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(notes: Entity<Notes>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("Search", cx));
        let subscriptions = [
            cx.observe(&notes, |this, notes, cx| {
                if let Some(prompt) = &mut this.delete_prompt {
                    prompt
                        .note
                        .take_if(|path| notes.read(cx).note(path).is_none());
                }
                // The query can also be cleared by the model (e.g. by a new note).
                let query = notes.read(cx).query().to_owned();
                this.search
                    .update(cx, |search, cx| search.set_text(&query, cx));
                this.rebuild_rows(cx);
                cx.notify();
            }),
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.subscribe(&notes, Self::on_notes_event),
        ];
        let mut sidebar = Self {
            notes,
            width: DEFAULT_SIDEBAR_WIDTH,
            search,
            focus_before_search: None,
            list_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            rows: Rc::new([]),
            rows_date: local_date(SystemTime::now()),
            rename: None,
            menu: None,
            delete_prompt: None,
            _subscriptions: subscriptions,
        };
        sidebar.rebuild_rows(cx);
        sidebar
    }

    fn rebuild_rows(&mut self, cx: &App) {
        self.rows_date = local_date(SystemTime::now());
        self.rows = rows(self.notes.read(cx), self.rows_date).into();
    }

    /// The current width. The integration persists it in the config (PLAN §32).
    pub fn width(&self) -> Pixels {
        self.width
    }

    /// Sets the width, clamped to what keeps both the list and the editor usable.
    pub fn set_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.width = width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        cx.notify();
    }

    /// The text in the search field.
    pub fn search_text(&self, cx: &App) -> String {
        self.search.read(cx).text().to_owned()
    }

    /// Whether keyboard focus is anywhere in the sidebar.
    pub fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.list_focus.contains_focused(window, cx)
            || self.search.focus_handle(cx).is_focused(window)
            || self
                .menu
                .as_ref()
                .is_some_and(|menu| menu.focus.is_focused(window))
    }

    /// Moves focus to the note list, e.g. when the sidebar is shown from the keyboard.
    pub fn focus_list(&self, window: &mut Window) {
        window.focus(&self.list_focus);
    }

    /// Moves focus to the search field and selects its text (Ctrl+P, Ctrl+Shift+F).
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.search.focus_handle(cx);
        if !search.is_focused(window) {
            self.focus_before_search = window.focused(cx);
        }
        window.focus(&search);
        self.search.update(cx, |search, cx| search.select_all(cx));
    }

    fn on_search_event(
        &mut self,
        search: &Entity<TextInput>,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                let query = search.read(cx).text().to_owned();
                self.notes
                    .update(cx, |notes, cx| notes.set_query(&query, cx));
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            TextInputEvent::Confirmed => {
                let first_hit = self
                    .notes
                    .read(cx)
                    .search_hits()
                    .and_then(|hits| hits.first().map(|hit| hit.path.clone()));
                if let Some(path) = first_hit {
                    self.open(path, window, cx);
                    cx.emit(SidebarEvent::NoteChosen);
                }
            }
            TextInputEvent::Cancelled => {
                search.update(cx, |search, cx| search.set_text("", cx));
                match self.focus_before_search.take() {
                    Some(previous) => window.focus(&previous),
                    None => window.focus(&self.list_focus),
                }
            }
        }
    }

    fn on_notes_event(&mut self, _: Entity<Notes>, event: &NotesEvent, _: &mut Context<Self>) {
        if let NotesEvent::Renamed { from, to } = event
            && let Some(prompt) = &mut self.delete_prompt
            && prompt.note.as_ref() == Some(from)
        {
            prompt.note = Some(to.clone());
        }
    }

    /// Starts a new note and shows it at the top of the list.
    pub fn new_note(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.notes.update(cx, |notes, cx| notes.new_note(cx));
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.list_focus);
        self.notes.update(cx, |notes, cx| notes.select(&path, cx));
    }

    /// Opens the note `delta` rows below (or above) the open one and scrolls it into view.
    fn open_adjacent(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let notes = self.notes.read(cx);
        let visible = notes.visible_paths();
        let open = match notes.selection() {
            Selection::Note(open) => visible.iter().position(|path| path == open),
            Selection::Draft(_) | Selection::None => None,
        };
        // Without an open note (or from the new note, which sits above them all) Down
        // starts at the top.
        let target = match open {
            Some(ix) => ix.saturating_add_signed(delta).min(visible.len() - 1),
            None if delta > 0 && !visible.is_empty() => 0,
            None => return,
        };
        let path = visible[target].to_path_buf();
        self.open(path.clone(), window, cx);
        self.reveal(&path, delta > 0, cx);
    }

    /// Scrolls the minimum needed to show the note's row.
    fn reveal(&self, path: &Path, moving_down: bool, cx: &App) {
        let notes = self.notes.read(cx);
        let row_of = |row: &Row| match *row {
            Row::Note(ix) => notes.notes().get(ix).map(|note| note.path.as_path()),
            Row::Hit(ix) => notes.search_hits()?.get(ix).map(|hit| hit.path.as_path()),
            Row::Header(_) | Row::Draft => None,
        };
        if let Some(ix) = self.rows.iter().position(|row| row_of(row) == Some(path)) {
            let strategy = if moving_down {
                ScrollStrategy::Bottom
            } else {
                ScrollStrategy::Top
            };
            self.scroll.scroll_to_item(ix, strategy);
        }
    }

    fn open_path(&self, cx: &App) -> Option<PathBuf> {
        match self.notes.read(cx).selection() {
            Selection::Note(path) => Some(path.clone()),
            Selection::Draft(_) | Selection::None => None,
        }
    }

    fn select_previous(
        &mut self,
        _: &SelectPreviousNote,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_adjacent(-1, window, cx);
    }

    fn select_next(&mut self, _: &SelectNextNote, window: &mut Window, cx: &mut Context<Self>) {
        self.open_adjacent(1, window, cx);
    }

    fn focus_open_note(&mut self, _: &FocusOpenNote, _: &mut Window, cx: &mut Context<Self>) {
        if self.notes.read(cx).selection() != &Selection::None {
            cx.emit(SidebarEvent::FocusEditor);
        }
    }

    fn rename_open_note(&mut self, _: &RenameNote, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.open_path(cx) {
            self.reveal(&path, true, cx);
            self.start_rename(path, window, cx);
        }
    }

    fn delete_open_note(&mut self, _: &DeleteNote, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.open_path(cx) {
            self.confirm_delete(path, window, cx);
        }
    }

    /// Replaces the note's title in the list with a text field. Enter or clicking elsewhere
    /// renames the file, Escape cancels.
    fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // Never drop a rename in progress silently (e.g. F2 while already renaming).
        self.finish_rename(true, window, cx);
        let title = title_of(&path);
        let input = cx.new(|cx| {
            let mut input = TextInput::new("", cx);
            input.set_text(&title, cx);
            input.select_all(cx);
            input
        });
        let input_focus = input.focus_handle(cx);
        let subscriptions = [
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                TextInputEvent::Confirmed => this.finish_rename(true, window, cx),
                TextInputEvent::Cancelled => this.finish_rename(false, window, cx),
                TextInputEvent::Changed => {}
            }),
            cx.on_blur(&input_focus, window, |this, window, cx| {
                this.finish_rename(true, window, cx)
            }),
        ];
        window.focus(&input_focus);
        self.rename = Some(Rename {
            path,
            input,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let title = rename.input.read(cx).text().trim().to_owned();
        // An empty title would become "Untitled"; treat it as a change of mind instead.
        if commit && !title.is_empty() && title != title_of(&rename.path) {
            let renamed = self
                .notes
                .update(cx, |notes, cx| notes.rename(&rename.path, &title, cx));
            match renamed {
                // Right-clicking the row commits the rename (the title loses focus) after the
                // menu has opened for the old path.
                Ok(new_path) => {
                    if let Some(menu) = self.menu.as_mut().filter(|menu| menu.path == rename.path) {
                        menu.path = new_path;
                    }
                }
                Err(error) => toast::show_file_error(&title_of(&rename.path), &error, cx),
            }
        }
        if rename.input.focus_handle(cx).is_focused(window) {
            window.focus(&self.list_focus);
        }
        cx.notify();
    }

    fn renaming(&self, path: &Path) -> bool {
        self.rename
            .as_ref()
            .is_some_and(|rename| rename.path == path)
    }

    /// Asks before moving the note to the recycle bin, with the system's own dialog.
    fn confirm_delete(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.delete_prompt.is_some() {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete \"{}\"?", title_of(&path)),
            Some("The note will be moved to the Recycle Bin."),
            &["Delete", "Cancel"],
            cx,
        );
        self.delete_prompt = Some(DeletePrompt { note: Some(path) });
        cx.spawn(async move |this, cx| {
            // Escape and the close button answer Cancel; a dialog gone with its window, too.
            let confirmed = matches!(answer.await, Ok(0));
            this.update(cx, |this, cx| {
                let note = this.delete_prompt.take().and_then(|prompt| prompt.note);
                if let Some(path) = note
                    && confirmed
                {
                    this.delete(&path, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn delete(&mut self, path: &Path, cx: &mut Context<Self>) {
        let deleted = self.notes.update(cx, |notes, cx| notes.delete(path, cx));
        if let Err(error) = deleted {
            toast::show_file_error(&title_of(path), &error, cx);
        }
    }

    fn open_menu(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.menu = Some(ContextMenu {
            path,
            position,
            _blur: cx.on_blur(&focus, window, |this, window, cx| {
                this.close_menu(window, cx)
            }),
            focus,
        });
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            if menu.focus.is_focused(window) {
                window.focus(&self.list_focus);
            }
            cx.notify();
        }
    }

    fn choose(&mut self, item: MenuItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.menu.as_ref().map(|menu| menu.path.clone()) else {
            return;
        };
        self.close_menu(window, cx);
        match item {
            MenuItem::Rename => self.start_rename(path, window, cx),
            MenuItem::Delete => self.confirm_delete(path, window, cx),
            MenuItem::ShowInFolder => {
                if let Err(error) = show_in_folder(&path) {
                    let message = format!(
                        "Could not show \"{}\" in its folder. {}",
                        title_of(&path),
                        toast::describe(&error)
                    );
                    toast::show_error(message, cx);
                }
            }
        }
    }

    fn render_row(&self, row: Row, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let today = self.rows_date;
        match row {
            Row::Header(group) => div()
                .debug_selector(|| format!("group:{}", group.label()))
                .h(ROW_HEIGHT)
                .px_5()
                .pb_1()
                .flex()
                .items_end()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(group.label())
                .into_any_element(),
            // The draft only exists while it is open, so it is always selected.
            Row::Draft => {
                let highlight = Some(self.selection_color(window, cx));
                let title = self
                    .notes
                    .read(cx)
                    .open_title()
                    .unwrap_or("New Note")
                    .to_owned();
                list_row(
                    "draft",
                    title,
                    "No additional text",
                    highlight,
                    false,
                    theme,
                )
                .debug_selector(|| "note:draft".into())
                .into_any_element()
            }
            Row::Note(ix) => {
                let notes = self.notes.read(cx);
                let Some(note) = notes.notes().get(ix) else {
                    return div().into_any_element();
                };
                let (path, title) = (note.path.clone(), note.title.clone());
                let subtitle = StyledText::new(modified_label(note, today));
                // While the open note's title line is edited, its new title shows before the
                // file is renamed.
                let shown_title = match (notes.selection(), notes.open_title()) {
                    (Selection::Note(open), Some(open_title)) if *open == path => {
                        open_title.to_owned()
                    }
                    _ => title.clone(),
                };
                self.note_row(
                    ix,
                    &path,
                    StyledText::new(shown_title),
                    subtitle,
                    window,
                    cx,
                )
                .debug_selector(|| format!("note:{title}"))
                .into_any_element()
            }
            Row::Hit(ix) => {
                let notes = self.notes.read(cx);
                let Some(hit) = notes.search_hits().and_then(|hits| hits.get(ix)) else {
                    return div().into_any_element();
                };
                let title = highlighted(&hit.title, &hit.title_ranges, theme);
                // Notes that match only by title have no snippet; show the date instead.
                let subtitle = if hit.snippet.is_empty() {
                    let note = notes.note(&hit.path);
                    StyledText::new(note.map_or(String::new(), |n| modified_label(n, today)))
                } else {
                    highlighted(&hit.snippet, &hit.snippet_ranges, theme)
                };
                let selector = format!("hit:{}", hit.title);
                let path = hit.path.clone();
                self.note_row(("hit", ix), &path, title, subtitle, window, cx)
                    .debug_selector(|| selector)
                    .into_any_element()
            }
        }
    }

    /// A row for the note at `path`, with its mouse interactions.
    fn note_row(
        &self,
        id: impl Into<ElementId>,
        path: &Path,
        title: StyledText,
        subtitle: StyledText,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected =
            matches!(self.notes.read(cx).selection(), Selection::Note(open) if open == path);
        let highlight = selected.then(|| self.selection_color(window, cx));
        let menu_target = self.menu.as_ref().is_some_and(|menu| menu.path == path);
        let title = match &self.rename {
            Some(rename) if rename.path == path => {
                let theme = cx.theme();
                div()
                    // Line the edited text up with the titles around it.
                    .ml(px(-5.))
                    .px(px(4.))
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.accent)
                    .bg(theme.background)
                    .child(rename.input.clone())
                    .into_any_element()
            }
            _ => title.into_any_element(),
        };
        let click_path = path.to_owned();
        let menu_path = path.to_owned();
        list_row(id, title, subtitle, highlight, menu_target, cx.theme())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                // Clicks inside the title being edited only move its caret.
                if this.renaming(&click_path) {
                    return;
                }
                if event.click_count() >= 2 {
                    this.start_rename(click_path.clone(), window, cx);
                } else {
                    this.open(click_path.clone(), window, cx);
                    cx.emit(SidebarEvent::NoteChosen);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(menu_path.clone(), event.position, window, cx);
                    // Or the list (and an overlay around it) would take focus from the menu.
                    window.prevent_default();
                }),
            )
    }

    /// Background of the selected row: accent while the list has focus, neutral otherwise,
    /// as in native lists.
    fn selection_color(&self, window: &Window, cx: &App) -> Hsla {
        let theme = cx.theme();
        if self.list_focus.contains_focused(window, cx) {
            theme.selection
        } else {
            theme.foreground.opacity(0.08)
        }
    }

    fn render_search_field(
        &self,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("search-field")
            .debug_selector(|| "search-field".into())
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .bg(theme.background)
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.search.focus_handle(cx))
            }))
            .child(
                div()
                    .font_family(typography::ICON_FONT_FAMILY)
                    .text_xs()
                    .text_color(theme.muted)
                    .child(SEARCH_ICON),
            )
            .child(div().flex_1().min_w_0().child(self.search.clone()))
    }

    fn render_menu(&self, menu: &ContextMenu, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let item = |item: MenuItem, cx: &mut Context<Self>| {
            div()
                .id(item.label())
                .debug_selector(|| format!("menu:{}", item.label()))
                .px_3()
                .py_1()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(theme.selection))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.choose(item, window, cx)
                }))
                .child(item.label())
        };
        let menu_element = div()
            .id("context-menu")
            .track_focus(&menu.focus)
            .occlude()
            .min_w(px(170.))
            .p_1()
            .flex()
            .flex_col()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .on_mouse_down_out(
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.close_menu(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_menu(window, cx);
                    // Not also hiding the sidebar shown over the note.
                    cx.stop_propagation();
                }
            }))
            .child(item(MenuItem::Rename, cx))
            .child(item(MenuItem::Delete, cx))
            .child(div().my_1().h(px(1.)).bg(theme.border))
            .child(item(MenuItem::ShowInFolder, cx));
        deferred(
            anchored()
                .position(menu.position)
                .snap_to_window_with_margin(px(8.))
                .child(menu_element),
        )
        .with_priority(1)
    }
}

/// Opens the system file manager with the note selected.
fn show_in_folder(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Explorer parses its own command line and needs the quotes inside the switch, which
        // std's argument quoting cannot produce. Quoted, spaces and commas in the path are
        // safe, and paths cannot contain quotes. Built as an `OsString` so that names that are
        // not valid Unicode are passed unchanged.
        let mut select = std::ffi::OsString::from("/select,\"");
        select.push(path);
        select.push("\"");
        Command::new("explorer").raw_arg(select).spawn()?;
    }
    #[cfg(target_os = "macos")]
    Command::new("open").arg("-R").arg(path).spawn()?;
    #[cfg(not(any(windows, target_os = "macos")))]
    Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn()?;
    Ok(())
}

/// The button that shows and hides the sidebar ([`ToggleSidebar`]): in the sidebar's header,
/// and at the top left of the note while the sidebar is hidden.
pub fn sidebar_toggle_button(id: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .font_family(typography::ICON_FONT_FAMILY)
        .text_sm()
        .text_color(theme.muted)
        .hover(|style| {
            style
                .bg(theme.foreground.opacity(0.05))
                .text_color(theme.foreground)
        })
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSidebar), cx))
        .child(SIDEBAR_ICON)
}

/// A two-line list row: the title and a muted second line. `highlight` is the background of
/// the selected row; `outlined` marks the row a context menu is open for.
fn list_row(
    id: impl Into<ElementId>,
    title: impl IntoElement,
    subtitle: impl IntoElement,
    highlight: Option<Hsla>,
    outlined: bool,
    theme: &Theme,
) -> Stateful<Div> {
    div().id(id).h(ROW_HEIGHT).px_2().child(
        div()
            .size_full()
            .px_3()
            .flex()
            .flex_col()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(if outlined {
                theme.accent
            } else {
                gpui::transparent_black()
            })
            .map(|row| match highlight {
                Some(color) => row.bg(color),
                None => row.hover(|style| style.bg(theme.foreground.opacity(0.04))),
            })
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(subtitle),
            ),
    )
}

fn rows(notes: &Notes, today: NaiveDate) -> Vec<Row> {
    if notes.is_searching() {
        let hits = notes.search_hits().map_or(0, |hits| hits.len());
        return (0..hits).map(Row::Hit).collect();
    }
    let mut rows = Vec::with_capacity(notes.notes().len() + 6);
    let mut current = None;
    if notes.has_draft() {
        rows.extend([Row::Header(DateGroup::Today), Row::Draft]);
        current = Some(DateGroup::Today);
    }
    for (ix, note) in notes.notes().iter().enumerate() {
        let group = DateGroup::of(local_date(note.modified_at), today);
        if current != Some(group) {
            rows.push(Row::Header(group));
            current = Some(group);
        }
        rows.push(Row::Note(ix));
    }
    rows
}

/// `text` with the matched `ranges` marked like selected text.
fn highlighted(text: &str, ranges: &[Range<usize>], theme: &Theme) -> StyledText {
    let style = HighlightStyle {
        color: Some(theme.foreground),
        background_color: Some(theme.selection),
        ..Default::default()
    };
    StyledText::new(text.to_owned())
        .with_highlights(ranges.iter().map(|range| (range.clone(), style)))
}

/// Time for today and yesterday, weekday within a week, date otherwise.
fn modified_label(note: &Note, today: NaiveDate) -> String {
    let Some(modified) = local_time(note.modified_at) else {
        return String::new();
    };
    match DateGroup::of(modified.date_naive(), today) {
        DateGroup::Today | DateGroup::Yesterday => modified.format("%H:%M").to_string(),
        DateGroup::Previous7Days => modified.format("%A").to_string(),
        _ if modified.year() == today.year() => modified.format("%-d %B").to_string(),
        _ => modified.format("%-d %B %Y").to_string(),
    }
}

/// `None` for timestamps chrono cannot represent (corrupt file metadata).
fn local_time(time: SystemTime) -> Option<DateTime<Local>> {
    let seconds = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let utc = DateTime::from_timestamp(i64::try_from(seconds).ok()?, 0)?;
    Some(utc.with_timezone(&Local))
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if local_date(SystemTime::now()) != self.rows_date {
            self.rebuild_rows(cx);
        }
        let theme = cx.theme().clone();
        let notes = self.notes.read(cx);
        let rows = self.rows.clone();
        let empty_text = match (notes.is_searching(), notes.search_hits()) {
            (true, Some([])) => Some("No results"),
            (false, _) if notes.is_loaded() && rows.is_empty() => Some("No notes"),
            _ => None,
        };
        let search_focused = self.search.focus_handle(cx).is_focused(window);

        div()
            .debug_selector(|| "sidebar".into())
            .relative()
            .flex_none()
            .w(self.width)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.surface)
            .border_r_1()
            .border_color(theme.border)
            .child(
                div()
                    .ml_2()
                    .mr_3()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(sidebar_toggle_button("sidebar-toggle", &theme))
                    .child(self.render_search_field(search_focused, &theme, cx)),
            )
            .child(
                div()
                    .id("new-note")
                    .debug_selector(|| "new-note-button".into())
                    .mx_2()
                    .mt_2()
                    .mb_1()
                    .px_3()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.foreground.opacity(0.05)))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.new_note(window, cx)),
                    )
                    .child(
                        div()
                            .font_family(typography::ICON_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme.accent)
                            .child(ADD_ICON),
                    )
                    .child("New Note"),
            )
            .child(
                div()
                    .id("note-list")
                    .track_focus(&self.list_focus)
                    .key_context("NoteList")
                    .on_action(cx.listener(Self::select_previous))
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::focus_open_note))
                    .on_action(cx.listener(Self::rename_open_note))
                    .on_action(cx.listener(Self::delete_open_note))
                    .flex_1()
                    .min_h_0()
                    .when_some(empty_text, |list, text| {
                        list.flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.muted)
                            .child(
                                div()
                                    .debug_selector(|| text.to_lowercase().replace(' ', "-"))
                                    .child(text),
                            )
                    })
                    .when(empty_text.is_none(), |list| {
                        list.child(
                            uniform_list(
                                "notes",
                                rows.len(),
                                cx.processor(move |this, range: Range<usize>, window, cx| {
                                    range
                                        .map(|ix| this.render_row(rows[ix], window, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .size_full()
                            .pb_2()
                            .track_scroll(self.scroll.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .id("sidebar-resize-handle")
                    .debug_selector(|| "sidebar-resize-handle".into())
                    .absolute()
                    .top_0()
                    .right_0()
                    .h_full()
                    .w(RESIZE_HANDLE_WIDTH)
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_drag(DraggedEdge, |_, _, _, cx| cx.new(|_| DraggedEdge)),
            )
            // Moves are reported here even when the pointer leaves the sidebar mid-drag.
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DraggedEdge>, _, cx| {
                    this.set_width(event.event.position.x - event.bounds.left(), cx)
                }),
            )
            .when_some(self.menu.as_ref(), |sidebar, menu| {
                sidebar.child(self.render_menu(menu, cx))
            })
    }
}
