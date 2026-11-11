//! The note's context menu (ADR 0175), as in Notepad: right-click in the text, Shift+F10 or the
//! Menu key. It offers Undo, Cut, Copy, Paste, Delete, Select all and a Formatting submenu, with
//! the corrections of a misspelled word at the top (ADR 0170).
//!
//! The commands are the note's actions: choosing one gives the note its focus back and
//! dispatches the action there, exactly as its key would, and the keys shown are read from the
//! keymap. The rows are the menu bar's (see [`crate::menu`]). While the menu is open it has the
//! keys and a click anywhere else only closes it.

use gpui::{
    Action, AnyElement, AnyWindowHandle, Bounds, Context, Corner, EntityInputHandler, FocusHandle,
    Pixels, Point, SharedString, Subscription, Window, anchored, canvas, deferred, div, point,
    prelude::*, px, relative,
};

use super::spell::{Choice, SpellingSection, menu_choices};
use super::{Autoscroll, EditorView};
use crate::actions::editor::{
    Copy, Cut, Delete, InsertLink, Paste, SelectAll, ToggleBold, ToggleInlineCode, ToggleItalic,
    ToggleStrikethrough, Undo,
};
use crate::menu::{
    self, ActivateItem, CONTEXT_MENU_CONTEXT, CloseMenu, NextMenu, PreviousMenu, SelectNextItem,
    SelectPreviousItem, ShownItem,
};
use crate::theme::{ActiveTheme, Theme};

/// The gap between the menu and its submenu, and between a menu and the window's edges.
const SUBMENU_GAP: Pixels = px(2.);
const WINDOW_MARGIN: Pixels = px(8.);

pub(super) struct ContextMenu {
    /// Where it opened, in window coordinates: the pointer, or below the cursor.
    position: Point<Pixels>,
    /// The corrections of the misspelled word it opened on.
    pub(super) spelling: Option<SpellingSection>,
    /// By the pointer or the arrow keys: an item, not a place in the list, which changes while
    /// the menu is open (the suggestions come in).
    highlighted: Option<Key>,
    /// The highlighted item's submenu is shown, with this item of it highlighted.
    submenu: Option<Option<usize>>,
    /// The menu's and the submenu's bounds in the last frame, to open the submenu on the side
    /// with room for it.
    panel_bounds: Option<Bounds<Pixels>>,
    submenu_width: Option<Pixels>,
    /// The menu's window and focus, to give the focus back to the note when it closes.
    window: AnyWindowHandle,
    focus: FocusHandle,
    _blur: Subscription,
}

enum Command {
    Spelling(Choice),
    Action(Box<dyn Action>),
    /// The Formatting submenu.
    Formatting,
}

struct Item {
    label: SharedString,
    command: Command,
    enabled: bool,
}

/// Which item is highlighted. Spelling suggestions are words, which may be any label.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    spelling: bool,
    label: SharedString,
}

impl Item {
    fn key(&self) -> Key {
        Key {
            spelling: matches!(self.command, Command::Spelling(_)),
            label: self.label.clone(),
        }
    }
}

fn key_at(entries: &[Entry], ix: usize) -> Option<Key> {
    match entries.get(ix) {
        Some(Entry::Item(item)) => Some(item.key()),
        _ => None,
    }
}

fn index_of(entries: &[Entry], key: &Key) -> Option<usize> {
    entries
        .iter()
        .position(|entry| matches!(entry, Entry::Item(item) if item.key() == *key))
}

enum Entry {
    Item(Item),
    Separator,
    /// A line that cannot be chosen, such as "No suggestions".
    Note(&'static str),
}

fn action(label: &'static str, action: impl Action, enabled: bool) -> Entry {
    Entry::Item(Item {
        label: label.into(),
        command: Command::Action(Box::new(action)),
        enabled,
    })
}

fn enabled_item(entries: &[Entry], ix: usize) -> Option<&Item> {
    match entries.get(ix) {
        Some(Entry::Item(item)) if item.enabled => Some(item),
        _ => None,
    }
}

/// The command of item `ix`, if it can be chosen.
fn enabled_command(mut entries: Vec<Entry>, ix: usize) -> Option<Command> {
    enabled_item(&entries, ix)?;
    match entries.swap_remove(ix) {
        Entry::Item(item) => Some(item.command),
        Entry::Separator | Entry::Note(_) => None,
    }
}

fn next_enabled(entries: &[Entry], from: Option<usize>, forward: bool) -> Option<usize> {
    menu::next_enabled(entries.len(), from, forward, |ix| {
        enabled_item(entries, ix).is_some()
    })
}

/// The Formatting submenu: what Ctrl+B, Ctrl+I and the other Markdown keys do.
fn formatting_entries() -> Vec<Entry> {
    vec![
        action("Bold", ToggleBold, true),
        action("Italic", ToggleItalic, true),
        action("Strikethrough", ToggleStrikethrough, true),
        action("Inline code", ToggleInlineCode, true),
        action("Link", InsertLink, true),
    ]
}

impl EditorView {
    /// The context menu's items while it is open (`submenu`: those of its open submenu), for
    /// tests.
    #[doc(hidden)]
    pub fn context_menu_items(&self, submenu: bool, window: &Window) -> Option<Vec<ShownItem>> {
        let menu = self.context_menu.as_ref()?;
        let entries = match submenu {
            false => self.context_entries(),
            true if menu.submenu.is_some() => formatting_entries(),
            true => Vec::new(),
        };
        let items = entries.iter().filter_map(|entry| match entry {
            Entry::Item(item) => Some(ShownItem {
                label: item.label.to_string(),
                shortcut: self.context_shortcut(item, window),
                enabled: item.enabled,
                checked: None,
            }),
            Entry::Separator | Entry::Note(_) => None,
        });
        Some(items.collect())
    }

    /// The menu's entries, with what can be chosen right now: as in the Edit menu (ADR 0150).
    fn context_entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        if let Some(section) = self.context_menu.as_ref().and_then(|m| m.spelling.as_ref()) {
            entries.extend(section.note().map(Entry::Note));
            let mut separated = false;
            for (choice, label) in menu_choices(section, self.read_only) {
                if !choice.is_correction() && !separated {
                    separated = true;
                    if !entries.is_empty() {
                        entries.push(Entry::Separator);
                    }
                }
                entries.push(Entry::Item(Item {
                    label: label.into(),
                    command: Command::Spelling(choice),
                    enabled: true,
                }));
            }
            entries.push(Entry::Separator);
        }
        let writable = !self.read_only;
        let selected = !self.editor.selection().is_empty();
        entries.extend([
            action("Undo", Undo, writable && self.editor.can_undo()),
            Entry::Separator,
            action("Cut", Cut, writable && selected),
            action("Copy", Copy, selected),
            action("Paste", Paste, writable),
            action("Delete", Delete, writable && selected),
            Entry::Separator,
            action("Select all", SelectAll, true),
            Entry::Separator,
            Entry::Item(Item {
                label: "Formatting".into(),
                command: Command::Formatting,
                // Plain text has no Markdown to write.
                enabled: writable && !self.plain,
            }),
        ]);
        entries
    }

    fn context_shortcut(&self, item: &Item, window: &Window) -> Option<String> {
        match &item.command {
            Command::Action(action) => menu::shortcut(action.as_ref(), &self.focus_handle, window),
            Command::Spelling(_) | Command::Formatting => None,
        }
    }

    // --- Opening and closing ---

    /// A right-click in the text: the cursor moves there unless it is inside the selection,
    /// which then stays, and the menu opens at the pointer, with the corrections of the word
    /// there if it is misspelled.
    pub(super) fn context_menu_mouse_down(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.offset_at(position, window);
        let spelling = self.spelling_section_at(offset, cx);
        let selection = self.editor.selection();
        let in_selection =
            !selection.is_empty() && selection.start() <= offset && offset <= selection.end();
        if !in_selection {
            self.editor.move_to(offset, false);
            self.autoscroll = Some(Autoscroll::Click);
            self.selection_changed(cx);
        }
        self.open_context_menu(position, spelling, false, window, cx);
    }

    /// Shift+F10 or the Menu key: the menu opens below the cursor, with its first item
    /// highlighted.
    pub(super) fn open_context_menu_at_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let head = self.editor.selection().head;
        let spelling = self.spelling_section_at(head, cx);
        let utf16 = self.editor.buffer().offset_to_utf16(head).0;
        let position = match self.bounds_for_range(utf16..utf16, Bounds::default(), window, cx) {
            Some(caret) => caret.bottom_left(),
            // Scrolled out of view: it comes back into view, and the menu opens at the top left.
            None => {
                self.autoscroll = Some(Autoscroll::Cursor);
                cx.notify();
                self.bounds.map_or_else(Point::default, |bounds| {
                    bounds.origin + point(px(24.), px(24.))
                })
            }
        };
        self.open_context_menu(position, spelling, true, window, cx);
    }

    fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        spelling: Option<SpellingSection>,
        from_keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.context_menu = Some(ContextMenu {
            position,
            spelling,
            highlighted: None,
            submenu: None,
            panel_bounds: None,
            submenu_width: None,
            window: window.window_handle(),
            // Another window or a shortcut (Ctrl+N, Alt+E) took the focus.
            _blur: cx.on_blur(&focus, window, |this, _, cx| this.close_context_menu(cx)),
            focus,
        });
        if from_keyboard {
            let entries = self.context_entries();
            let first = next_enabled(&entries, None, true).and_then(|ix| key_at(&entries, ix));
            if let Some(menu) = &mut self.context_menu {
                menu.highlighted = first;
            }
        }
        cx.notify();
    }

    /// Closes the context menu. If it had the focus, the note gets it back once the current
    /// update is done, so this works where the window is not at hand (an edit, a setting).
    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        let (window, focus, note) = (menu.window, menu.focus.clone(), self.focus_handle.clone());
        drop(menu);
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, _| {
                    if focus.is_focused(window) {
                        window.focus(&note);
                    }
                })
                .ok();
        });
        cx.notify();
    }

    /// Closes the menu and gives the note its focus back at once: Escape, a click elsewhere or
    /// a command.
    fn dismiss_context_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ContextMenu> {
        let menu = self.context_menu.take()?;
        window.focus(&self.focus_handle);
        cx.notify();
        Some(menu)
    }

    /// Where the highlighted item is in `entries`.
    fn highlighted_index(&self, entries: &[Entry]) -> Option<usize> {
        let key = self.context_menu.as_ref()?.highlighted.as_ref()?;
        index_of(entries, key)
    }

    /// Runs the item `key` of the menu (`sub` of its submenu) if it is enabled.
    fn run_context_key(
        &mut self,
        key: &Key,
        sub: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = index_of(&self.context_entries(), key) {
            self.run_context_item(ix, sub, window, cx);
        }
    }

    /// Runs item `ix` of the menu (`sub` of its submenu) if it is enabled.
    fn run_context_item(
        &mut self,
        ix: usize,
        sub: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = enabled_command(self.context_entries(), ix) else {
            return;
        };
        let command = match (command, sub) {
            (Command::Formatting, None) => {
                self.enter_context_submenu(ix, false, cx);
                return;
            }
            (Command::Formatting, Some(sub)) => {
                let Some(command) = enabled_command(formatting_entries(), sub) else {
                    return;
                };
                command
            }
            (command, None) => command,
            (_, Some(_)) => return,
        };
        let Some(menu) = self.dismiss_context_menu(window, cx) else {
            return;
        };
        match command {
            // The focus is the note's again: the action goes where its key would.
            Command::Action(action) => window.dispatch_action(action, cx),
            Command::Spelling(choice) => {
                if let Some(section) = menu.spelling {
                    self.choose_spelling(section, choice, cx);
                }
            }
            Command::Formatting => {}
        }
    }

    /// Shows the submenu of item `ix`, highlighting its first item when asked from the keyboard.
    fn enter_context_submenu(&mut self, ix: usize, from_keyboard: bool, cx: &mut Context<Self>) {
        let entries = self.context_entries();
        let has_submenu = enabled_item(&entries, ix)
            .is_some_and(|item| matches!(item.command, Command::Formatting));
        let Some(menu) = self.context_menu.as_mut().filter(|_| has_submenu) else {
            return;
        };
        menu.highlighted = key_at(&entries, ix);
        menu.submenu = Some(
            from_keyboard
                .then(|| next_enabled(&formatting_entries(), None, true))
                .flatten(),
        );
        cx.notify();
    }

    // --- Keyboard ---

    /// The keyboard is in the submenu: one of its items is highlighted.
    fn in_context_submenu(&self) -> bool {
        self.context_menu
            .as_ref()
            .is_some_and(|menu| matches!(menu.submenu, Some(Some(_))))
    }

    fn move_context_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        let entries = self.context_entries();
        let in_submenu = self.in_context_submenu();
        let from = self.highlighted_index(&entries);
        let Some(menu) = &mut self.context_menu else {
            return;
        };
        if in_submenu {
            if let Some(highlighted) = &mut menu.submenu {
                *highlighted = next_enabled(&formatting_entries(), *highlighted, forward);
            }
        } else {
            menu.submenu = None;
            menu.highlighted =
                next_enabled(&entries, from, forward).and_then(|ix| key_at(&entries, ix));
        }
        cx.notify();
    }

    fn context_previous(&mut self, _: &SelectPreviousItem, _: &mut Window, cx: &mut Context<Self>) {
        self.move_context_highlight(false, cx);
    }

    fn context_next(&mut self, _: &SelectNextItem, _: &mut Window, cx: &mut Context<Self>) {
        self.move_context_highlight(true, cx);
    }

    /// Left leaves the submenu; the context menu has no menu beside it.
    fn context_left(&mut self, _: &PreviousMenu, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.context_menu
            && menu.submenu.take().is_some()
        {
            cx.notify();
        }
    }

    /// Right enters the highlighted item's submenu.
    fn context_right(&mut self, _: &NextMenu, _: &mut Window, cx: &mut Context<Self>) {
        let highlighted = self.highlighted_index(&self.context_entries());
        if let Some(ix) = highlighted.filter(|_| !self.in_context_submenu()) {
            self.enter_context_submenu(ix, true, cx);
        }
    }

    fn context_activate(&mut self, _: &ActivateItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let sub = menu.submenu.flatten();
        let Some(ix) = self.highlighted_index(&self.context_entries()) else {
            return;
        };
        if self.in_context_submenu() {
            self.run_context_item(ix, sub, window, cx);
        } else if enabled_item(&self.context_entries(), ix)
            .is_some_and(|item| matches!(item.command, Command::Formatting))
        {
            self.enter_context_submenu(ix, true, cx);
        } else {
            self.run_context_item(ix, None, window, cx);
        }
    }

    fn context_close(&mut self, _: &CloseMenu, window: &mut Window, cx: &mut Context<Self>) {
        match &mut self.context_menu {
            Some(menu) if menu.submenu.is_some() => {
                menu.submenu = None;
                cx.notify();
            }
            _ => _ = self.dismiss_context_menu(window, cx),
        }
    }

    // --- Pointer ---

    fn hover_context_item(&mut self, key: &Key, hovered: bool, cx: &mut Context<Self>) {
        let entries = self.context_entries();
        let has_submenu = index_of(&entries, key)
            .and_then(|ix| enabled_item(&entries, ix))
            .is_some_and(|item| matches!(item.command, Command::Formatting));
        let Some(menu) = &mut self.context_menu else {
            return;
        };
        let is_highlighted = menu.highlighted.as_ref() == Some(key);
        if hovered && !is_highlighted {
            menu.highlighted = Some(key.clone());
            menu.submenu = has_submenu.then_some(None);
        } else if !hovered && is_highlighted && menu.submenu.is_none() {
            // A submenu stays while the pointer moves over to it.
            menu.highlighted = None;
        }
        cx.notify();
    }

    fn hover_context_submenu_item(&mut self, sub: usize, hovered: bool, cx: &mut Context<Self>) {
        if let Some(highlighted) = self.context_menu.as_mut().and_then(|m| m.submenu.as_mut()) {
            if hovered {
                *highlighted = Some(sub);
            } else if *highlighted == Some(sub) {
                *highlighted = None;
            }
            cx.notify();
        }
    }

    // --- Rendering ---

    /// The menu, and under it a backdrop over the window that takes the clicks elsewhere.
    pub(super) fn render_context_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(menu) = &self.context_menu else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        let entries = self.context_entries();
        let submenu_left = self.submenu_on_the_left(window);
        let this = cx.entity().downgrade();
        let rows: Vec<AnyElement> = entries
            .iter()
            .map(|entry| {
                let item = match entry {
                    Entry::Item(item) => item,
                    Entry::Separator => return menu::separator(&theme).into_any_element(),
                    Entry::Note(text) => return menu::note(text, &theme).into_any_element(),
                };
                let key = item.key();
                let highlighted = menu.highlighted.as_ref() == Some(&key);
                let formatting = matches!(item.command, Command::Formatting);
                let (hover_key, click_key) = (key.clone(), key.clone());
                self.context_row(item, highlighted, &theme, window)
                    .when(item.enabled, |row| {
                        row.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            this.hover_context_item(&hover_key, *hovered, cx)
                        }))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.run_context_key(&click_key, None, window, cx)
                            },
                        ))
                    })
                    .when_some(
                        menu.submenu.filter(|_| highlighted && formatting),
                        |row, sub| {
                            // Its own anchor, so that it moves up from the window's bottom edge
                            // as the menu does.
                            let (side, corner) = if submenu_left {
                                (div().left(-SUBMENU_GAP), Corner::TopRight)
                            } else {
                                (div().left(relative(1.)).ml(SUBMENU_GAP), Corner::TopLeft)
                            };
                            let submenu = self.render_formatting(&key, sub, &theme, window, cx);
                            row.child(
                                side.absolute().top(px(-5.)).child(
                                    anchored()
                                        .anchor(corner)
                                        .snap_to_window_with_margin(WINDOW_MARGIN)
                                        .child(submenu),
                                ),
                            )
                        },
                    )
                    .into_any_element()
            })
            .collect();
        let panel = menu::panel("context-menu", &theme)
            .debug_selector(|| "context-menu".into())
            .track_focus(&menu.focus)
            .key_context(CONTEXT_MENU_CONTEXT)
            .on_action(cx.listener(Self::context_previous))
            .on_action(cx.listener(Self::context_next))
            .on_action(cx.listener(Self::context_left))
            .on_action(cx.listener(Self::context_right))
            .on_action(cx.listener(Self::context_activate))
            .on_action(cx.listener(Self::context_close))
            .child(measure(this, |menu, bounds| {
                menu.panel_bounds = Some(bounds)
            }))
            .children(rows);
        let backdrop = div()
            .id("context-menu-backdrop")
            .debug_selector(|| "context-menu-backdrop".into())
            .w(window.viewport_size().width)
            .h(window.viewport_size().height)
            .occlude()
            .on_any_mouse_down(cx.listener(|this, _, window, cx| {
                // Not focusing what was clicked: the note gets the focus back.
                window.prevent_default();
                this.dismiss_context_menu(window, cx);
            }));
        vec![
            deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
                .with_priority(1)
                .into_any_element(),
            deferred(
                anchored()
                    .position(menu.position)
                    .snap_to_window_with_margin(WINDOW_MARGIN)
                    .child(panel),
            )
            .with_priority(2)
            .into_any_element(),
        ]
    }

    /// The submenu opens on the right of the menu, unless it does not fit there and there is
    /// more room on the left.
    fn submenu_on_the_left(&self, window: &Window) -> bool {
        let Some(menu) = &self.context_menu else {
            return false;
        };
        let Some(panel) = menu.panel_bounds else {
            return false;
        };
        let width = menu.submenu_width.unwrap_or(menu::MENU_MIN_WIDTH) + SUBMENU_GAP;
        let right = window.viewport_size().width - WINDOW_MARGIN - panel.right();
        let left = panel.left() - WINDOW_MARGIN;
        right < width && left > right
    }

    fn render_formatting(
        &self,
        parent: &Key,
        highlighted: Option<usize>,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let rows: Vec<AnyElement> = formatting_entries()
            .iter()
            .enumerate()
            .filter_map(|(ix, entry)| {
                let Entry::Item(item) = entry else {
                    return None;
                };
                let row = self
                    .context_row(item, highlighted == Some(ix), theme, window)
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.hover_context_submenu_item(ix, *hovered, cx)
                    }))
                    .on_click({
                        let parent = parent.clone();
                        cx.listener(move |this, _, window, cx| {
                            this.run_context_key(&parent, Some(ix), window, cx)
                        })
                    });
                Some(row.into_any_element())
            })
            .collect();
        menu::panel("context-submenu", theme)
            .debug_selector(|| "context-submenu".into())
            .child(measure(this, |menu, bounds| {
                menu.submenu_width = Some(bounds.size.width)
            }))
            .children(rows)
    }

    fn context_row(
        &self,
        item: &Item,
        highlighted: bool,
        theme: &Theme,
        window: &Window,
    ) -> gpui::Stateful<gpui::Div> {
        let label = &item.label;
        let (selector, strong) = match &item.command {
            Command::Spelling(choice) => (format!("spelling:{label}"), choice.is_correction()),
            _ => (format!("context-menu:{label}"), false),
        };
        let row = menu::Row {
            id: SharedString::from(selector.clone()).into(),
            selector,
            label: label.clone(),
            shortcut: self.context_shortcut(item, window),
            enabled: item.enabled,
            checked: None,
            submenu: matches!(item.command, Command::Formatting),
            strong,
        };
        menu::row(row, highlighted, false, theme)
    }
}

/// An empty element filling its parent that hands the parent's bounds to `record` as it is laid
/// out, for the next frame.
fn measure(
    this: gpui::WeakEntity<EditorView>,
    record: impl Fn(&mut ContextMenu, Bounds<Pixels>) + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, _, cx| {
            this.update(cx, |this, _| {
                if let Some(menu) = &mut this.context_menu {
                    record(menu, bounds);
                }
            })
            .ok();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
