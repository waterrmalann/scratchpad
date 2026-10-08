//! The settings (PLAN §33, §35): a small panel over the window. See ADR 0080.
//!
//! Ctrl+, opens it; Escape, the close button or a click outside closes it, as does focus
//! leaving it. Tab and Shift+Tab move between its controls, Enter and Space press the focused
//! button, and the arrow keys switch the theme while the theme control has focus.

use gpui::{
    App, Context, Div, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding, Window,
    actions, div, prelude::*, px,
};

use crate::settings;
use crate::theme::{ActiveTheme, Theme, ThemeMode, typography};

actions!(
    settings_panel,
    [
        /// Close the settings.
        CloseSettings,
        /// Move focus to the next control of the settings, wrapping around.
        FocusNextControl,
        /// Move focus to the previous control of the settings, wrapping around.
        FocusPreviousControl,
        /// Choose the theme left of the current one.
        PreviousTheme,
        /// Choose the theme right of the current one.
        NextTheme,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let panel = Some("SettingsPanel");
    let theme_picker = Some("ThemePicker");
    vec![
        KeyBinding::new("escape", CloseSettings, panel),
        KeyBinding::new("tab", FocusNextControl, panel),
        KeyBinding::new("shift-tab", FocusPreviousControl, panel),
        KeyBinding::new("left", PreviousTheme, theme_picker),
        KeyBinding::new("up", PreviousTheme, theme_picker),
        KeyBinding::new("right", NextTheme, theme_picker),
        KeyBinding::new("down", NextTheme, theme_picker),
    ]
}

const THEMES: [(ThemeMode, &str); 3] = [
    (ThemeMode::System, "System"),
    (ThemeMode::Light, "Light"),
    (ThemeMode::Dark, "Dark"),
];

/// Segoe MDL2 Assets "ChromeClose"; a plain cross elsewhere.
const CLOSE_ICON: &str = if cfg!(windows) {
    "\u{E8BB}"
} else {
    "\u{00D7}"
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsPanelEvent {
    /// The user dismissed the settings.
    Close,
}

pub struct SettingsPanel {
    focus_handle: FocusHandle,
    theme_focus: FocusHandle,
    close_focus: FocusHandle,
}

impl EventEmitter<SettingsPanelEvent> for SettingsPanel {}

impl SettingsPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panel = Self {
            focus_handle: cx.focus_handle(),
            theme_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
        };
        window.focus(&panel.theme_focus);
        panel
    }

    /// The controls in Tab order.
    fn controls(&self) -> Vec<&FocusHandle> {
        vec![&self.theme_focus, &self.close_focus]
    }

    fn move_focus(&self, forward: bool, window: &mut Window) {
        let controls = self.controls();
        let count = controls.len();
        let next = match controls.iter().position(|handle| handle.is_focused(window)) {
            Some(ix) if forward => (ix + 1) % count,
            Some(ix) => (ix + count - 1) % count,
            None if forward => 0,
            None => count - 1,
        };
        window.focus(controls[next]);
    }

    fn focus_next(&mut self, _: &FocusNextControl, window: &mut Window, _: &mut Context<Self>) {
        self.move_focus(true, window);
    }

    fn focus_previous(
        &mut self,
        _: &FocusPreviousControl,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.move_focus(false, window);
    }

    fn close(&mut self, _: &CloseSettings, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(SettingsPanelEvent::Close);
    }

    fn previous_theme(&mut self, _: &PreviousTheme, _: &mut Window, cx: &mut Context<Self>) {
        step_theme(false, cx);
    }

    fn next_theme(&mut self, _: &NextTheme, _: &mut Window, cx: &mut Context<Self>) {
        step_theme(true, cx);
    }

    fn render_theme_picker(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let current = cx.theme_mode();
        let segments = THEMES.map(|(mode, label)| {
            let selected = mode == current;
            div()
                .id(label)
                .debug_selector(move || format!("theme:{label}"))
                .px_3()
                .py(px(3.))
                .rounded(px(4.))
                .cursor_pointer()
                .map(|segment| {
                    if selected {
                        segment.bg(theme.selection).text_color(theme.foreground)
                    } else {
                        let hover = theme.foreground;
                        segment
                            .text_color(theme.muted)
                            .hover(move |style| style.text_color(hover))
                    }
                })
                .on_click(move |_, _, cx| settings::set_theme_mode(mode, cx))
                .child(label)
        });
        // A row, so the control is as wide as its segments rather than the panel.
        div().flex().child(
            div()
                .id("theme-picker")
                .key_context("ThemePicker")
                .track_focus(&self.theme_focus)
                .on_action(cx.listener(Self::previous_theme))
                .on_action(cx.listener(Self::next_theme))
                .flex()
                .flex_row()
                .gap(px(2.))
                .p(px(2.))
                .rounded_md()
                .border_1()
                .border_color(if self.theme_focus.is_focused(window) {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(theme.surface)
                .children(segments),
        )
    }

    fn render_footer(&self, theme: &Theme, cx: &App) -> Div {
        let version = concat!("Scratchpad ", env!("CARGO_PKG_VERSION"));
        let config = settings::path(cx).map(|path| format!("Settings file: {}", path.display()));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted)
            .child(version)
            .children(config)
    }
}

/// Picks the theme next to the current one, wrapping around like a radio group.
fn step_theme(forward: bool, cx: &mut App) {
    let current = THEMES
        .iter()
        .position(|(mode, _)| *mode == cx.theme_mode())
        .unwrap_or(0);
    let count = THEMES.len();
    let next = if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    };
    settings::set_theme_mode(THEMES[next].0, cx);
}

fn section(label: &'static str, theme: &Theme, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_color(theme.foreground)
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(content)
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let close_accent = theme.accent;
        let close = div()
            .id("close-settings")
            .debug_selector(|| "button:close-settings".into())
            .track_focus(&self.close_focus)
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(theme.background)
            .font_family(typography::ICON_FONT_FAMILY)
            .text_size(px(10.))
            .text_color(theme.muted)
            .cursor_pointer()
            .hover({
                let (surface, foreground) = (theme.surface, theme.foreground);
                move |style| style.bg(surface).text_color(foreground)
            })
            .focus(move |style| style.border_color(close_accent))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsPanelEvent::Close)))
            .child(CLOSE_ICON);
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pl_5()
            .pr_3()
            .pt_3()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            )
            .child(close);
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .px_5()
            .pt_2()
            .pb_5()
            .child(section(
                "Theme",
                &theme,
                self.render_theme_picker(&theme, window, cx),
            ));
        let panel = div()
            .id("settings")
            .debug_selector(|| "settings".into())
            .key_context("SettingsPanel")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::close))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(SettingsPanelEvent::Close)))
            .w(px(440.))
            .mx_4()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .text_color(theme.foreground)
            .shadow_lg()
            .child(header)
            .child(body)
            .child(self.render_footer(&theme, cx));
        // Covers the window so the notes behind cannot be clicked, and fades them a little so
        // the panel stands out in both themes.
        div()
            .id("settings-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background.opacity(0.6))
            .child(panel)
    }
}
