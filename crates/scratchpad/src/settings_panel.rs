//! The settings (PLAN §33, §35): a small panel over the window with the only two things worth
//! choosing, the theme and the notes folder. See ADR 0080.
//!
//! Ctrl+, opens it; Escape, the close button or a click outside closes it, as does focus
//! leaving it. Tab and Shift+Tab move between its controls, Enter and Space press the focused
//! button, and the arrow keys switch the theme while the theme control has focus.

use std::path::PathBuf;

use gpui::{
    App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    PathPromptOptions, SharedString, Stateful, Subscription, Task, Window, actions, div,
    prelude::*, px,
};

use crate::notes::Notes;
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

/// Stands in for the native folder dialog, which GPUI's test platform does not implement:
/// while set, "Change…" picks this folder (`None`: the dialog is cancelled).
#[cfg(feature = "test-support")]
pub struct PickFolderForTests(pub Option<PathBuf>);

#[cfg(feature = "test-support")]
impl gpui::Global for PickFolderForTests {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsPanelEvent {
    /// The user dismissed the settings.
    Close,
    /// The user picked this folder for their notes.
    NotesFolderPicked(PathBuf),
}

pub struct SettingsPanel {
    notes: Entity<Notes>,
    /// `SCRATCHPAD_NOTES_DIR` decides the folder, so it cannot be changed here.
    notes_dir_overridden: bool,
    focus_handle: FocusHandle,
    theme_focus: FocusHandle,
    change_folder_focus: FocusHandle,
    open_folder_focus: FocusHandle,
    close_focus: FocusHandle,
    /// The folder dialog while it is open.
    _picking: Option<Task<()>>,
    _notes_changed: Subscription,
}

impl EventEmitter<SettingsPanelEvent> for SettingsPanel {}

impl SettingsPanel {
    pub fn new(
        notes: Entity<Notes>,
        notes_dir_overridden: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panel = Self {
            _notes_changed: cx.observe(&notes, |_, _, cx| cx.notify()),
            notes,
            notes_dir_overridden,
            focus_handle: cx.focus_handle(),
            theme_focus: cx.focus_handle(),
            change_folder_focus: cx.focus_handle(),
            open_folder_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            _picking: None,
        };
        window.focus(&panel.theme_focus);
        panel
    }

    /// The controls in Tab order. "Change…" is left out while it is disabled.
    fn controls(&self) -> Vec<&FocusHandle> {
        let mut controls = vec![&self.theme_focus];
        if !self.notes_dir_overridden {
            controls.push(&self.change_folder_focus);
        }
        controls.extend([&self.open_folder_focus, &self.close_focus]);
        controls
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

    fn change_folder(&mut self, cx: &mut Context<Self>) {
        if self.notes_dir_overridden {
            return;
        }
        let picked = pick_folder(cx);
        self._picking = Some(cx.spawn(async move |this, cx| {
            if let Some(dir) = picked.await {
                this.update(cx, |_, cx| {
                    cx.emit(SettingsPanelEvent::NotesFolderPicked(dir))
                })
                .ok();
            }
        }));
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

    fn render_notes_folder(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let dir = self.notes.read(cx).location().dir.clone();
        let change = if self.notes_dir_overridden {
            disabled_button("Change\u{2026}", theme)
        } else {
            button(
                "change-folder",
                "Change\u{2026}",
                &self.change_folder_focus,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| this.change_folder(cx)))
        };
        let open = button("open-folder", "Open Folder", &self.open_folder_focus, theme).on_click({
            let dir = dir.clone();
            move |_, _, cx| cx.open_with_system(&dir)
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .debug_selector(|| "notes-folder-path".into())
                    .text_color(theme.foreground)
                    .child(SharedString::from(dir.display().to_string())),
            )
            .when(self.notes_dir_overridden, |section| {
                section.child(
                    div()
                        .debug_selector(|| "notes-folder-overridden".into())
                        .text_color(theme.muted)
                        .child("Set by the SCRATCHPAD_NOTES_DIR environment variable."),
                )
            })
            .child(div().flex().flex_row().gap_2().child(change).child(open))
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

/// The folder the user picks in the native dialog, `None` if they cancel.
fn pick_folder(cx: &mut App) -> Task<Option<PathBuf>> {
    #[cfg(feature = "test-support")]
    if let Some(PickFolderForTests(dir)) = cx.try_global::<PickFolderForTests>() {
        return Task::ready(dir.clone());
    }
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |_| match picked.await {
        Ok(Ok(paths)) => paths.and_then(|mut paths| paths.pop()),
        Ok(Err(error)) => {
            tracing::warn!("could not show the folder dialog: {error:#}");
            None
        }
        Err(_) => None,
    })
}

/// A push button in the style of the notice bar's (ADR 0063).
fn button(
    id: &'static str,
    label: &'static str,
    focus: &FocusHandle,
    theme: &Theme,
) -> Stateful<Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .debug_selector(move || format!("button:{id}"))
        .track_focus(focus)
        .px_3()
        .py(px(3.))
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .text_color(theme.foreground)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.border_color(accent))
        .focus(move |style| style.border_color(accent))
        .child(label)
}

fn disabled_button(label: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id("change-folder-disabled")
        .debug_selector(|| "button:change-folder-disabled".into())
        .px_3()
        .py(px(3.))
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .text_color(theme.muted)
        .font_weight(FontWeight::MEDIUM)
        .child(label)
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
            ))
            .child(section(
                "Notes folder",
                &theme,
                self.render_notes_folder(&theme, cx),
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
