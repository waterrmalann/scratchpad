mod common;

use gpui::{TestAppContext, VisualTestContext, WindowAppearance, px, size};
use scratchpad::sidebar::DEFAULT_SIDEBAR_WIDTH;
use scratchpad::theme::{self, ActiveTheme, Appearance, ThemeMode};

#[gpui::test]
fn close_window_shortcut_closes_the_main_window(cx: &mut TestAppContext) {
    let (_root, cx) = common::open_main_window(cx);
    assert_eq!(cx.windows().len(), 1);

    cx.simulate_keystrokes("ctrl-w");

    assert!(cx.windows().is_empty());
}

#[gpui::test]
fn sidebar_keeps_its_width_and_editor_pane_fills_the_rest(cx: &mut TestAppContext) {
    let (_root, cx) = common::open_main_window(cx);

    for window_size in [size(px(1100.), px(720.)), size(px(600.), px(400.))] {
        cx.simulate_resize(window_size);
        cx.run_until_parked();

        let sidebar = cx.debug_bounds("sidebar").expect("sidebar rendered");
        let editor = cx
            .debug_bounds("editor-pane")
            .expect("editor pane rendered");
        assert_eq!(sidebar.origin.x, px(0.));
        assert_eq!(sidebar.size.width, DEFAULT_SIDEBAR_WIDTH);
        assert_eq!(sidebar.size.height, window_size.height);
        assert_eq!(editor.left(), sidebar.right());
        assert_eq!(editor.right(), window_size.width);
        assert_eq!(editor.size.height, window_size.height);
    }
}

#[gpui::test]
fn system_theme_follows_os_appearance_until_a_mode_is_chosen(cx: &mut TestAppContext) {
    // The test platform reports a light OS appearance and cannot simulate a change, so the
    // OS switching to dark is driven through the same call the window's observer makes.
    let (_root, cx) = common::open_main_window(cx);
    let appearance = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.theme().appearance);
    let os_switches_to = |system, cx: &mut VisualTestContext| {
        cx.update(|_, cx| theme::system_appearance_changed(system, cx))
    };
    assert_eq!(appearance(cx), Appearance::Light);

    os_switches_to(WindowAppearance::Dark, cx);
    assert_eq!(appearance(cx), Appearance::Dark);

    cx.update(|_, cx| theme::set_mode(ThemeMode::Light, cx));
    assert_eq!(appearance(cx), Appearance::Light);
    os_switches_to(WindowAppearance::Dark, cx);
    assert_eq!(appearance(cx), Appearance::Light);

    cx.update(|_, cx| theme::set_mode(ThemeMode::Dark, cx));
    assert_eq!(appearance(cx), Appearance::Dark);
    os_switches_to(WindowAppearance::Light, cx);
    assert_eq!(appearance(cx), Appearance::Dark);

    // Back to System: re-reads the OS appearance (light on the test platform).
    cx.update(|_, cx| theme::set_mode(ThemeMode::System, cx));
    assert_eq!(appearance(cx), Appearance::Light);
}
