//! The settings panel (Ctrl+,) and the theme (PLAN §33, §35). See ADR 0080.

mod common;

use common::{click, days_ago, wait, write_note};
use gpui::{
    Entity, Focusable, KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext, point,
    px,
};
use scratchpad::AppWindow;
use scratchpad::settings::SAVE_DELAY;
use scratchpad::theme::{ActiveTheme, Appearance, ThemeMode};
use scratchpad_core::{Config, ThemePreference};

fn is_open(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    root.read_with(cx, |root, _| root.settings_panel().is_some())
}

fn editor_has_focus(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> bool {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn focus_editor(root: &Entity<AppWindow>, cx: &mut VisualTestContext) {
    let editor = common::editor(root, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
}

fn theme_mode(cx: &mut VisualTestContext) -> ThemeMode {
    cx.update(|_, cx| cx.theme_mode())
}

/// Presses Enter the way a real keyboard does: GPUI presses a focused button on key-up.
fn press_enter(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("enter");
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
}

#[gpui::test]
fn ctrl_comma_opens_the_settings_and_closing_them_returns_to_the_note(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "Ideas", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    focus_editor(&root, cx);

    cx.simulate_keystrokes("ctrl-,");
    assert!(is_open(&root, cx));
    assert!(!editor_has_focus(&root, cx), "the panel takes the focus");
    cx.simulate_keystrokes("escape");
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));

    // A click outside the panel closes it too.
    cx.simulate_keystrokes("ctrl-,");
    assert!(is_open(&root, cx));
    cx.simulate_click(point(px(4.), px(4.)), Modifiers::none());
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));

    // So does the close button, and a click inside the panel does not.
    cx.simulate_keystrokes("ctrl-,");
    click("settings", cx);
    assert!(is_open(&root, cx));
    click("button:close-settings", cx);
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));
}

#[gpui::test]
fn the_settings_close_when_another_command_takes_the_focus(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);

    cx.simulate_keystrokes("ctrl-, ctrl-p");

    assert!(!is_open(&root, cx));
    assert!(
        !editor_has_focus(&root, cx),
        "the search field keeps the focus"
    );
}

#[gpui::test]
fn choosing_a_theme_applies_it_and_remembers_it(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (_root, cx) = common::open_with(storage, cx);
    let appearance = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.theme().appearance);
    // The test platform's OS appearance is light.
    assert_eq!(appearance(cx), Appearance::Light);

    cx.simulate_keystrokes("ctrl-,");
    click("theme:Dark", cx);

    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    assert_eq!(appearance(cx), Appearance::Dark);
    wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).theme, ThemePreference::Dark);

    click("theme:System", cx);
    assert_eq!(appearance(cx), Appearance::Light);
    wait(SAVE_DELAY, cx);
    assert_eq!(Config::load(&config_path).theme, ThemePreference::System);
}

#[gpui::test]
fn the_settings_work_from_the_keyboard(cx: &mut TestAppContext) {
    let (root, cx) = common::open_main_window(cx);
    cx.simulate_keystrokes("ctrl-,");

    // The theme control has the focus first; the arrows pick a theme, wrapping around.
    cx.simulate_keystrokes("right");
    assert_eq!(theme_mode(cx), ThemeMode::Light);
    cx.simulate_keystrokes("down");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    cx.simulate_keystrokes("right");
    assert_eq!(theme_mode(cx), ThemeMode::System);
    cx.simulate_keystrokes("left");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);

    // Tab leaves the theme control (the arrows do nothing on a button)...
    cx.simulate_keystrokes("tab right");
    assert_eq!(theme_mode(cx), ThemeMode::Dark);
    // ...and comes back to it after Close.
    cx.simulate_keystrokes("tab right");
    assert_eq!(theme_mode(cx), ThemeMode::System);

    // Shift+Tab from the theme control wraps around to Close; Enter presses it.
    cx.simulate_keystrokes("shift-tab");
    assert!(is_open(&root, cx));
    press_enter(cx);
    assert!(!is_open(&root, cx));
    assert!(editor_has_focus(&root, cx));
}
