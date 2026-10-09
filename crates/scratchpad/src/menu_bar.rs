//! The menu bar under the title bar, as in Windows 11 Notepad: File, Edit and View. See ADR 0150.
//!
//! GPUI 0.2.2 keeps application menus (`cx.set_menus`) but does not show them on Windows, so the
//! bar is drawn in the window. Each command is an action: choosing it puts focus back where it
//! was before the menu opened and dispatches the action there, exactly as its key would, except
//! that Edit's editing commands always go to the note, as Notepad's edit the document. The keys
//! shown beside the commands are read from the keymap.
//!
//! Pressing a title opens its menu, moving the pointer to another title switches to it, clicking
//! an item runs it and clicking anywhere else only closes the menu. Alt+F, Alt+E, Alt+V and F10
//! open a menu from the keyboard; then Up and Down choose an item, Left and Right go to the
//! neighbouring menu or into and out of Zoom, Enter runs the item and Escape closes.

use gpui::{
    Action, AnyElement, App, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, Pixels, Stateful, Subscription, WeakFocusHandle, Window, actions, anchored,
    deferred, div, point, prelude::*, px, relative,
};

use crate::actions::editor::{Copy, Cut, Delete, Paste, Redo, SelectAll, ToggleSourceMode, Undo};
use crate::actions::view::{ResetZoom, ToggleWordWrap, ZoomIn, ZoomOut};
use crate::actions::{
    NewNote, OpenFile, OpenSettings, Quit, SaveAs, SaveNote, ToggleSidebar, ToggleStatusBar,
};
use crate::editor_view::EditorView;
use crate::find_bar::{FindInNote, FindNext, FindPrevious, ReplaceInNote};
use crate::go_to_line::GoToLine;
use crate::settings;
use crate::theme::{ActiveTheme, Theme, typography};

actions!(
    menu_bar,
    [
        /// Opens the File menu (Alt+F, F10).
        OpenFileMenu,
        /// Opens the Edit menu (Alt+E).
        OpenEditMenu,
        /// Opens the View menu (Alt+V).
        OpenViewMenu,
        SelectPreviousItem,
        SelectNextItem,
        /// Leaves the submenu, or opens the menu to the left.
        PreviousMenu,
        /// Enters the highlighted item's submenu, or opens the menu to the right.
        NextMenu,
        /// Runs the highlighted item, or enters its submenu.
        ActivateItem,
        /// Leaves the submenu, or closes the menu.
        CloseMenu,
    ]
);

const CONTEXT: &str = "MenuBar";

pub fn key_bindings() -> Vec<KeyBinding> {
    let menu = Some(CONTEXT);
    vec![
        KeyBinding::new("alt-f", OpenFileMenu, None),
        KeyBinding::new("f10", OpenFileMenu, None),
        KeyBinding::new("alt-e", OpenEditMenu, None),
        KeyBinding::new("alt-v", OpenViewMenu, None),
        KeyBinding::new("up", SelectPreviousItem, menu),
        KeyBinding::new("down", SelectNextItem, menu),
        KeyBinding::new("left", PreviousMenu, menu),
        KeyBinding::new("right", NextMenu, menu),
        KeyBinding::new("enter", ActivateItem, menu),
        KeyBinding::new("space", ActivateItem, menu),
        KeyBinding::new("escape", CloseMenu, menu),
    ]
}

const BAR_HEIGHT: Pixels = px(30.);
const TITLES: [&str; 3] = ["File", "Edit", "View"];
const ITEM_HEIGHT: Pixels = px(28.);
const MENU_MIN_WIDTH: Pixels = px(220.);
/// Segoe MDL2 Assets "CheckMark" and "ChevronRight"; plain characters elsewhere.
const CHECK_ICON: &str = if cfg!(windows) {
    "\u{E73E}"
} else {
    "\u{2713}"
};
const SUBMENU_ICON: &str = if cfg!(windows) {
    "\u{E76C}"
} else {
    "\u{203A}"
};

/// One line of a menu.
enum Entry {
    Item(Item),
    Separator,
}

struct Item {
    label: &'static str,
    command: Command,
    enabled: bool,
    /// `Some` for an item that is ticked while its setting is on.
    checked: Option<bool>,
    /// The action is the note's: it runs there, and the note gets focus.
    on_note: bool,
}

enum Command {
    Action(Box<dyn Action>),
    Submenu(Vec<Entry>),
}

fn item(label: &'static str, action: impl Action) -> Item {
    Item {
        label,
        command: Command::Action(Box::new(action)),
        enabled: true,
        checked: None,
        on_note: false,
    }
}

/// One of Edit's editing commands, which act on the note (see [`Item::on_note`]).
fn edit(label: &'static str, action: impl Action) -> Item {
    Item {
        on_note: true,
        ..item(label, action)
    }
}

impl Item {
    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    fn submenu(&self) -> Option<&[Entry]> {
        match &self.command {
            Command::Submenu(entries) => Some(entries),
            Command::Action(_) => None,
        }
    }
}

impl From<Item> for Entry {
    fn from(item: Item) -> Self {
        Entry::Item(item)
    }
}

fn enabled_item(entries: &[Entry], ix: usize) -> Option<&Item> {
    match entries.get(ix) {
        Some(Entry::Item(item)) if item.enabled => Some(item),
        _ => None,
    }
}

/// The enabled item after (or before) `from`, wrapping around; the first (or last) without one.
fn next_enabled(entries: &[Entry], from: Option<usize>, forward: bool) -> Option<usize> {
    let count = entries.len();
    (1..=count)
        .map(|step| match (from, forward) {
            (Some(from), true) => (from + step) % count,
            (Some(from), false) => (from + count - step) % count,
            (None, true) => step - 1,
            (None, false) => count - step,
        })
        .find(|&ix| enabled_item(entries, ix).is_some())
}

/// A menu item as shown, for tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownItem {
    pub label: &'static str,
    /// The key that runs the command, e.g. `Ctrl+Shift+S`.
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
}

/// The bar and, while one is open, its menu.
pub struct MenuBar {
    editor: Entity<EditorView>,
    sidebar_shown: bool,
    /// Held by the open menu, so its keys reach it.
    focus_handle: FocusHandle,
    open: Option<OpenMenu>,
}

struct OpenMenu {
    /// Index into [`TITLES`].
    index: usize,
    /// By the pointer or the arrow keys.
    highlighted: Option<usize>,
    /// The highlighted item's submenu is shown.
    submenu: Option<Submenu>,
    /// Where focus was when the menu opened, and where it goes back to unless that is gone (Go
    /// to line closes when the menu takes focus); then it goes to the note.
    restore_focus: Option<WeakFocusHandle>,
    _blur: Subscription,
}

#[derive(Default)]
struct Submenu {
    highlighted: Option<usize>,
}

impl MenuBar {
    pub fn new(editor: Entity<EditorView>, sidebar_shown: bool, cx: &mut Context<Self>) -> Self {
        Self {
            editor,
            sidebar_shown,
            focus_handle: cx.focus_handle(),
            open: None,
        }
    }

    /// Keeps the tick of View > Sidebar in step with the window.
    pub fn set_sidebar_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.sidebar_shown != shown {
            self.sidebar_shown = shown;
            cx.notify();
        }
    }

    /// The title of the open menu.
    pub fn open_menu(&self) -> Option<&'static str> {
        self.open.as_ref().map(|open| TITLES[open.index])
    }

    /// The items of the open menu, or of its open submenu, without separators.
    pub fn shown_items(&self, submenu: bool, window: &Window, cx: &App) -> Vec<ShownItem> {
        let entries = self.entries(cx);
        let shown = match &self.open {
            Some(open) if submenu && open.submenu.is_some() => open
                .highlighted
                .and_then(|ix| enabled_item(&entries, ix))
                .and_then(Item::submenu)
                .unwrap_or_default(),
            _ if submenu => &[],
            _ => &entries,
        };
        shown
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(item) => Some(ShownItem {
                    label: item.label,
                    shortcut: self.shortcut(item, window, cx),
                    enabled: item.enabled,
                    checked: item.checked,
                }),
                Entry::Separator => None,
            })
            .collect()
    }

    /// The open menu's entries, with what can be chosen and what is ticked right now.
    fn entries(&self, cx: &App) -> Vec<Entry> {
        let Some(open) = &self.open else {
            return Vec::new();
        };
        match open.index {
            0 => vec![
                item("New note", NewNote).into(),
                item("Open\u{2026}", OpenFile).into(),
                item("Save", SaveNote).into(),
                item("Save as\u{2026}", SaveAs).into(),
                Entry::Separator,
                item("Settings", OpenSettings).into(),
                Entry::Separator,
                item("Exit", Quit).into(),
            ],
            1 => {
                // The note's selection and history are kept while focus is elsewhere, e.g. in
                // the find bar after Replace all, so they can be used from there too.
                let editor = self.editor.read(cx);
                let engine = editor.editor();
                let writable = !editor.is_read_only();
                let selected = !engine.selection().is_empty();
                vec![
                    edit("Undo", Undo)
                        .enabled(writable && engine.can_undo())
                        .into(),
                    edit("Redo", Redo)
                        .enabled(writable && engine.can_redo())
                        .into(),
                    Entry::Separator,
                    edit("Cut", Cut).enabled(writable && selected).into(),
                    edit("Copy", Copy).enabled(selected).into(),
                    edit("Paste", Paste).enabled(writable).into(),
                    edit("Delete", Delete).enabled(writable && selected).into(),
                    Entry::Separator,
                    item("Find", FindInNote).into(),
                    item("Find next", FindNext).into(),
                    item("Find previous", FindPrevious).into(),
                    item("Replace", ReplaceInNote).into(),
                    item("Go to", GoToLine).into(),
                    Entry::Separator,
                    edit("Select all", SelectAll).into(),
                ]
            }
            _ => vec![
                Item {
                    label: "Zoom",
                    command: Command::Submenu(vec![
                        item("Zoom in", ZoomIn).into(),
                        item("Zoom out", ZoomOut).into(),
                        item("Restore default zoom", ResetZoom).into(),
                    ]),
                    enabled: true,
                    checked: None,
                    on_note: false,
                }
                .into(),
                item("Status bar", ToggleStatusBar)
                    .checked(!settings::get(cx).status_bar_hidden)
                    .into(),
                item("Word wrap", ToggleWordWrap)
                    .checked(self.editor.read(cx).soft_wrap())
                    .into(),
                edit("Source mode", ToggleSourceMode)
                    .checked(self.editor.read(cx).source_mode())
                    .into(),
                item("Sidebar", ToggleSidebar)
                    .checked(self.sidebar_shown)
                    .into(),
            ],
        }
    }

    /// The key bound to the item's action where the note has focus (window-wide keys work there
    /// too), as Windows writes it. Of several keys, the one with the fewest modifiers, then a
    /// character rather than a named key: Ctrl+Y over Ctrl+Shift+Z, Ctrl+C over Ctrl+Insert.
    fn shortcut(&self, item: &Item, window: &Window, cx: &App) -> Option<String> {
        let Command::Action(action) = &item.command else {
            return None;
        };
        let target = self.editor.focus_handle(cx);
        window
            .bindings_for_action_in(action.as_ref(), &target)
            .iter()
            .min_by_key(|binding| {
                let keystrokes = binding.keystrokes();
                let modifiers = keystrokes
                    .iter()
                    .map(|keystroke| keystroke.modifiers().number_of_modifiers())
                    .sum::<u8>();
                let named_key = keystrokes
                    .iter()
                    .any(|keystroke| keystroke.key().chars().count() > 1);
                (keystrokes.len(), modifiers, named_key)
            })
            .map(shortcut_text)
    }

    // --- Opening and closing ---

    /// Opens menu `index`, or switches to it if another is open. From the keyboard, its first
    /// item is highlighted.
    pub fn open(
        &mut self,
        index: usize,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.is_none() {
            self.open = Some(OpenMenu {
                index,
                highlighted: None,
                submenu: None,
                restore_focus: window.focused(cx).map(|focus| focus.downgrade()),
                // Another window or a shortcut (Ctrl+N) took focus. Focus is put back only when
                // the window was left, for when the user comes back.
                _blur: cx.on_blur(&self.focus_handle, window, |this, window, cx| {
                    this.close(!window.is_window_active(), window, cx)
                }),
            });
            window.focus(&self.focus_handle);
        }
        self.switch(index, from_keyboard, cx);
    }

    fn switch(&mut self, index: usize, from_keyboard: bool, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        open.index = index;
        open.highlighted = None;
        open.submenu = None;
        if from_keyboard {
            let first = next_enabled(&self.entries(cx), None, true);
            if let Some(open) = &mut self.open {
                open.highlighted = first;
            }
        }
        cx.notify();
    }

    /// Makes the open menu put focus in the note when it closes, rather than back where it was:
    /// that is going away. Its commands then run from the note too; from something no longer
    /// shown, they would reach none of the window's handlers.
    pub fn return_focus_to_note(&mut self) {
        if let Some(open) = &mut self.open {
            open.restore_focus = None;
        }
    }

    /// Closes the menu, putting focus back where it was if `restore_focus`.
    fn close(&mut self, restore_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.open.take() {
            if restore_focus {
                let focus = open.restore_focus.and_then(|focus| focus.upgrade());
                window.focus(&focus.unwrap_or_else(|| self.editor.focus_handle(cx)));
            }
            cx.notify();
        }
    }

    /// Runs item `ix` of the menu (`sub` of its submenu) if it is enabled.
    fn run(&mut self, ix: usize, sub: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        let item = enabled_item(&entries, ix).and_then(|item| match (sub, item.submenu()) {
            (None, _) => Some(item),
            (Some(sub), Some(entries)) => enabled_item(entries, sub),
            (Some(_), None) => None,
        });
        let Some((Command::Action(action), on_note)) =
            item.map(|item| (&item.command, item.on_note))
        else {
            return;
        };
        // Focus first: the action goes where its key would have gone, or to the note.
        self.close(!on_note, window, cx);
        if on_note {
            window.focus(&self.editor.focus_handle(cx));
        }
        window.dispatch_action(action.boxed_clone(), cx);
    }

    /// Shows the submenu of item `ix`, highlighting its first item when asked from the keyboard.
    fn enter_submenu(&mut self, ix: usize, from_keyboard: bool, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        let Some(submenu) = enabled_item(&entries, ix).and_then(Item::submenu) else {
            return;
        };
        let highlighted = from_keyboard
            .then(|| next_enabled(submenu, None, true))
            .flatten();
        if let Some(open) = &mut self.open {
            open.highlighted = Some(ix);
            open.submenu = Some(Submenu { highlighted });
            cx.notify();
        }
    }

    // --- Keyboard ---

    /// The keyboard is in the submenu: one of its items is highlighted.
    fn in_submenu(&self) -> bool {
        self.open
            .as_ref()
            .and_then(|open| open.submenu.as_ref())
            .is_some_and(|submenu| submenu.highlighted.is_some())
    }

    fn move_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        let in_submenu = self.in_submenu();
        let Some(open) = &mut self.open else {
            return;
        };
        if in_submenu {
            let submenu = open
                .highlighted
                .and_then(|ix| enabled_item(&entries, ix))
                .and_then(Item::submenu);
            if let (Some(entries), Some(state)) = (submenu, &mut open.submenu) {
                state.highlighted = next_enabled(entries, state.highlighted, forward);
            }
        } else {
            open.submenu = None;
            open.highlighted = next_enabled(&entries, open.highlighted, forward);
        }
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPreviousItem, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(false, cx);
    }

    fn select_next(&mut self, _: &SelectNextItem, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(true, cx);
    }

    fn previous_menu(&mut self, _: &PreviousMenu, _: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else {
            return;
        };
        if open.submenu.take().is_some() {
            cx.notify();
        } else {
            let index = (open.index + TITLES.len() - 1) % TITLES.len();
            self.switch(index, true, cx);
        }
    }

    fn next_menu(&mut self, _: &NextMenu, _: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        let (index, highlighted) = (open.index, open.highlighted);
        if !self.in_submenu()
            && let Some(ix) = highlighted
            && enabled_item(&self.entries(cx), ix).is_some_and(|item| item.submenu().is_some())
        {
            self.enter_submenu(ix, true, cx);
        } else {
            self.switch((index + 1) % TITLES.len(), true, cx);
        }
    }

    fn activate(&mut self, _: &ActivateItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.open else {
            return;
        };
        let Some(ix) = open.highlighted else {
            return;
        };
        if self.in_submenu() {
            let sub = open
                .submenu
                .as_ref()
                .and_then(|submenu| submenu.highlighted);
            self.run(ix, sub, window, cx);
        } else if enabled_item(&self.entries(cx), ix).is_some_and(|item| item.submenu().is_some()) {
            self.enter_submenu(ix, true, cx);
        } else {
            self.run(ix, None, window, cx);
        }
    }

    fn close_menu(&mut self, _: &CloseMenu, window: &mut Window, cx: &mut Context<Self>) {
        match &mut self.open {
            Some(open) if open.submenu.is_some() => {
                open.submenu = None;
                cx.notify();
            }
            _ => self.close(true, window, cx),
        }
    }

    // --- Pointer ---

    fn hover_item(&mut self, ix: usize, hovered: bool, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        let Some(open) = &mut self.open else {
            return;
        };
        let has_submenu = enabled_item(&entries, ix).is_some_and(|item| item.submenu().is_some());
        if hovered && open.highlighted != Some(ix) {
            open.highlighted = Some(ix);
            open.submenu = has_submenu.then(Submenu::default);
        } else if !hovered && open.highlighted == Some(ix) && open.submenu.is_none() {
            // A submenu stays while the pointer moves over to it.
            open.highlighted = None;
        }
        cx.notify();
    }

    fn hover_submenu_item(&mut self, sub: usize, hovered: bool, cx: &mut Context<Self>) {
        if let Some(submenu) = self.open.as_mut().and_then(|open| open.submenu.as_mut()) {
            if hovered {
                submenu.highlighted = Some(sub);
            } else if submenu.highlighted == Some(sub) {
                submenu.highlighted = None;
            }
            cx.notify();
        }
    }

    // --- Rendering ---

    fn render_title(
        &self,
        index: usize,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = TITLES[index];
        let is_open = self.open.as_ref().is_some_and(|open| open.index == index);
        let pressed = theme.foreground.opacity(0.08);
        let hovered = theme.foreground.opacity(0.05);
        div()
            .id(title)
            .debug_selector(move || format!("menu:{title}"))
            .relative()
            .h(px(24.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .map(|title| {
                if is_open {
                    title.bg(pressed)
                } else {
                    title.hover(move |style| style.bg(hovered))
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    // Not also focusing the window's root, or closing the menu just opened.
                    window.prevent_default();
                    cx.stop_propagation();
                    if this.open.as_ref().is_some_and(|open| open.index == index) {
                        this.close(true, window, cx);
                    } else {
                        this.open(index, false, window, cx);
                    }
                }),
            )
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered && this.open.as_ref().is_some_and(|open| open.index != index) {
                    this.switch(index, false, cx);
                }
            }))
            .child(title)
            .when(is_open, |title| {
                title.child(
                    div().absolute().left_0().top(relative(1.)).child(
                        deferred(
                            anchored()
                                .offset(point(px(0.), px(3.)))
                                .snap_to_window_with_margin(px(8.))
                                .child(self.render_menu(theme, window, cx)),
                        )
                        .with_priority(2),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<gpui::Div> {
        let Some(open) = &self.open else {
            return div().id("menu");
        };
        let entries = self.entries(cx);
        let submenu = open.submenu.as_ref();
        let check_column = has_checks(&entries);
        let rows = entries.iter().enumerate().map(|(ix, entry)| {
            let Entry::Item(item) = entry else {
                return separator(theme);
            };
            let highlighted = open.highlighted == Some(ix);
            self.render_item(item, highlighted, check_column, theme, window, cx)
                .when(item.enabled, |row| {
                    row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hover_item(ix, *hovered, cx)
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let has_submenu = enabled_item(&this.entries(cx), ix)
                            .is_some_and(|item| item.submenu().is_some());
                        if has_submenu {
                            this.enter_submenu(ix, false, cx);
                        } else {
                            this.run(ix, None, window, cx);
                        }
                    }))
                })
                .when_some(
                    item.submenu().filter(|_| highlighted).zip(submenu),
                    |row, (entries, state)| {
                        row.child(
                            div()
                                .absolute()
                                .top(px(-5.))
                                .left(relative(1.))
                                .ml(px(2.))
                                .child(self.render_submenu(ix, entries, state, theme, window, cx)),
                        )
                    },
                )
                .into_any_element()
        });
        let rows: Vec<_> = rows.collect();
        panel("menu", theme)
            .debug_selector(|| "menu-dropdown".into())
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::previous_menu))
            .on_action(cx.listener(Self::next_menu))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::close_menu))
            .children(rows)
    }

    fn render_submenu(
        &self,
        parent: usize,
        entries: &[Entry],
        state: &Submenu,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let check_column = has_checks(entries);
        let rows = entries.iter().enumerate().map(|(ix, entry)| {
            let Entry::Item(item) = entry else {
                return separator(theme);
            };
            let highlighted = state.highlighted == Some(ix);
            self.render_item(item, highlighted, check_column, theme, window, cx)
                .when(item.enabled, |row| {
                    row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hover_submenu_item(ix, *hovered, cx)
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(parent, Some(ix), window, cx)
                    }))
                })
                .into_any_element()
        });
        panel("submenu", theme).children(rows)
    }

    fn render_item(
        &self,
        item: &Item,
        highlighted: bool,
        check_column: bool,
        theme: &Theme,
        window: &Window,
        cx: &App,
    ) -> Stateful<gpui::Div> {
        let label = item.label;
        let shortcut = self.shortcut(item, window, cx);
        let (text, secondary) = if item.enabled {
            (theme.foreground, theme.muted)
        } else {
            let disabled = theme.muted.opacity(0.6);
            (disabled, disabled)
        };
        let icon = |glyph: &'static str| {
            div()
                .font_family(typography::ICON_FONT_FAMILY)
                .text_size(px(10.))
                .child(glyph)
        };
        div()
            .id(label)
            .debug_selector(move || format!("menu-item:{label}"))
            .relative()
            .flex_none()
            .h(ITEM_HEIGHT)
            .pl_3()
            .pr_2()
            .flex()
            .items_center()
            .rounded(px(4.))
            .text_color(text)
            .when(highlighted && item.enabled, |row| {
                row.bg(theme.foreground.opacity(0.06))
            })
            .when(check_column, |row| {
                row.child(
                    div()
                        .flex_none()
                        .w(px(22.))
                        .when(item.checked == Some(true), |column| {
                            column.child(icon(CHECK_ICON))
                        }),
                )
            })
            .child(div().flex_1().pr_2().child(label))
            .children(shortcut.map(|keys| div().pl_6().pr_1().text_color(secondary).child(keys)))
            .when(item.submenu().is_some(), |row| {
                row.child(icon(SUBMENU_ICON).pr_1().text_color(secondary))
            })
    }
}

/// The box a menu or submenu is drawn in.
fn panel(id: &'static str, theme: &Theme) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .occlude()
        .min_w(MENU_MIN_WIDTH)
        .p_1()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .shadow_lg()
        .text_color(theme.foreground)
}

fn separator(theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .mx_1()
        .my_1()
        .h(px(1.))
        .bg(theme.border)
        .into_any_element()
}

/// Menus with ticks keep a column for them, so that the labels line up.
fn has_checks(entries: &[Entry]) -> bool {
    entries
        .iter()
        .any(|entry| matches!(entry, Entry::Item(item) if item.checked.is_some()))
}

/// `Ctrl+Shift+S`, `Ctrl+Plus`, `Del`, `Shift+F3`: as Windows writes keys in menus.
fn shortcut_text(binding: &KeyBinding) -> String {
    let keystrokes = binding.keystrokes().iter().map(|keystroke| {
        let modifiers = keystroke.modifiers();
        let mut parts: Vec<String> = [
            (modifiers.control, "Ctrl"),
            (modifiers.alt, "Alt"),
            (modifiers.shift, "Shift"),
            (modifiers.platform, "Win"),
        ]
        .into_iter()
        .filter(|&(held, _)| held)
        .map(|(_, name)| name.to_owned())
        .collect();
        parts.push(key_name(keystroke.key()));
        parts.join("+")
    });
    keystrokes.collect::<Vec<_>>().join(", ")
}

fn key_name(key: &str) -> String {
    match key {
        // Windows calls the =/+ key "Plus" (VK_OEM_PLUS), as Notepad's Zoom menu shows.
        "=" | "+" => "Plus".into(),
        "-" => "Minus".into(),
        "delete" => "Del".into(),
        "insert" => "Ins".into(),
        "escape" => "Esc".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        _ => {
            // `a` -> `A`, `f3` -> `F3`, `home` -> `Home`.
            let mut chars = key.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let below_bar = window.viewport_size().height - BAR_HEIGHT;
        div()
            .debug_selector(|| "menu-bar".into())
            .relative()
            .flex_none()
            .h(BAR_HEIGHT)
            .px_1()
            .flex()
            .items_center()
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    // The bar is not a place to put focus.
                    window.prevent_default();
                    this.close(true, window, cx);
                }),
            )
            .children((0..TITLES.len()).map(|index| self.render_title(index, &theme, window, cx)))
            // While a menu is open, a click anywhere under the bar only closes it.
            .when(self.open.is_some(), |bar| {
                bar.child(
                    deferred(
                        div()
                            .id("menu-backdrop")
                            .debug_selector(|| "menu-backdrop".into())
                            .absolute()
                            .top(relative(1.))
                            .left_0()
                            .right_0()
                            .h(below_bar)
                            .occlude()
                            .on_any_mouse_down(cx.listener(
                                |this, _: &MouseDownEvent, window, cx| this.close(true, window, cx),
                            )),
                    )
                    .with_priority(1),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(keys: &str) -> String {
        shortcut_text(&KeyBinding::new(keys, SaveNote, None))
    }

    #[test]
    fn keys_are_written_as_in_windows_menus() {
        assert_eq!(text("secondary-shift-s"), "Ctrl+Shift+S");
        assert_eq!(text("secondary-="), "Ctrl+Plus");
        assert_eq!(text("secondary-+"), "Ctrl+Plus");
        assert_eq!(text("secondary--"), "Ctrl+Minus");
        assert_eq!(text("secondary-0"), "Ctrl+0");
        assert_eq!(text("secondary-,"), "Ctrl+,");
        assert_eq!(text("secondary-\\"), "Ctrl+\\");
        assert_eq!(text("delete"), "Del");
        assert_eq!(text("shift-f3"), "Shift+F3");
        assert_eq!(text("ctrl-alt-enter"), "Ctrl+Alt+Enter");
        assert_eq!(text("ctrl-k ctrl-s"), "Ctrl+K, Ctrl+S");
    }
}
