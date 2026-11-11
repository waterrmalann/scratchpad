//! What the in-window menus share: the menu bar's menus (ADR 0150) and the note's context menu
//! (ADR 0175) look and move alike. The box, its rows, separators and the keys shown beside
//! commands are drawn here, and the arrow keys, Enter and Escape are the same actions in both,
//! bound in each menu's own key context.

use gpui::{
    Action, Div, ElementId, FocusHandle, KeyBinding, SharedString, Stateful, Window, actions, div,
    prelude::*, px,
};

use crate::theme::{Theme, typography};

actions!(
    menu,
    [
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

/// The key contexts of the menu bar's open menu and of the note's context menu.
pub const MENU_BAR_CONTEXT: &str = "MenuBar";
pub const CONTEXT_MENU_CONTEXT: &str = "ContextMenu";

pub fn key_bindings() -> Vec<KeyBinding> {
    [MENU_BAR_CONTEXT, CONTEXT_MENU_CONTEXT]
        .into_iter()
        .flat_map(|context| {
            let context = Some(context);
            [
                KeyBinding::new("up", SelectPreviousItem, context),
                KeyBinding::new("down", SelectNextItem, context),
                KeyBinding::new("left", PreviousMenu, context),
                KeyBinding::new("right", NextMenu, context),
                KeyBinding::new("enter", ActivateItem, context),
                KeyBinding::new("space", ActivateItem, context),
                KeyBinding::new("escape", CloseMenu, context),
            ]
        })
        .collect()
}

const ITEM_HEIGHT: gpui::Pixels = px(28.);
const MENU_MIN_WIDTH: gpui::Pixels = px(220.);
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

/// A menu item as shown, for tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownItem {
    pub label: String,
    /// The key that runs the command, e.g. `Ctrl+Shift+S`.
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
}

/// One row of a menu, as drawn by [`row`].
pub struct Row {
    pub id: ElementId,
    /// Tests find the row by it (`.debug_selector`).
    pub selector: String,
    pub label: SharedString,
    pub shortcut: Option<String>,
    pub enabled: bool,
    /// `Some` for an item that is ticked while its setting is on.
    pub checked: Option<bool>,
    pub submenu: bool,
    /// Drawn in semibold, as spelling suggestions are.
    pub strong: bool,
}

/// A menu item: its label, the key beside it, a tick or a submenu arrow, greyed out when
/// disabled and shaded while `highlighted`. Menus with ticks keep a `check_column` so that the
/// labels line up.
pub fn row(item: Row, highlighted: bool, check_column: bool, theme: &Theme) -> Stateful<Div> {
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
    let selector = item.selector;
    div()
        .id(item.id)
        .debug_selector(move || selector.clone())
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
        .child(
            div()
                .flex_1()
                .pr_2()
                .when(item.strong, |label| {
                    label.font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .child(item.label),
        )
        .children(
            item.shortcut
                .map(|keys| div().pl_6().pr_1().text_color(secondary).child(keys)),
        )
        .when(item.submenu, |row| {
            row.child(icon(SUBMENU_ICON).pr_1().text_color(secondary))
        })
}

/// A line of muted text in a menu that cannot be chosen, such as "No suggestions".
pub fn note(text: &'static str, theme: &Theme) -> Div {
    div()
        .flex_none()
        .h(ITEM_HEIGHT)
        .pl_3()
        .pr_2()
        .flex()
        .items_center()
        .text_color(theme.muted)
        .child(text)
}

/// The box a menu or submenu is drawn in.
pub fn panel(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id.into())
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

pub fn separator(theme: &Theme) -> Div {
    div().flex_none().mx_1().my_1().h(px(1.)).bg(theme.border)
}

/// The item after (or before) `from` that `enabled` accepts, among `count`, wrapping around;
/// the first (or last) without one.
pub fn next_enabled(
    count: usize,
    from: Option<usize>,
    forward: bool,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    (1..=count)
        .map(|step| match (from, forward) {
            (Some(from), true) => (from + step) % count,
            (Some(from), false) => (from + count - step) % count,
            (None, true) => step - 1,
            (None, false) => count - step,
        })
        .find(|&ix| enabled(ix))
}

/// The key bound to `action` where `target` has focus (window-wide keys work there too), as
/// Windows writes it. Of several keys, the one with the fewest modifiers, then a character
/// rather than a named key: Ctrl+Y over Ctrl+Shift+Z, Ctrl+C over Ctrl+Insert.
pub fn shortcut(action: &dyn Action, target: &FocusHandle, window: &Window) -> Option<String> {
    window
        .bindings_for_action_in(action, target)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn text(keys: &str) -> String {
        shortcut_text(&KeyBinding::new(keys, SelectNextItem, None))
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

    #[test]
    fn the_next_enabled_item_wraps_around() {
        let enabled = |ix: usize| ix != 1 && ix != 3;
        assert_eq!(next_enabled(4, None, true, enabled), Some(0));
        assert_eq!(next_enabled(4, None, false, enabled), Some(2));
        assert_eq!(next_enabled(4, Some(0), true, enabled), Some(2));
        assert_eq!(next_enabled(4, Some(2), true, enabled), Some(0));
        assert_eq!(next_enabled(4, Some(0), false, enabled), Some(2));
        assert_eq!(next_enabled(4, None, true, |_| false), None);
    }
}
