//! Colour tokens and typography.
//!
//! Views never use literal colours: they read the active [`Theme`] through
//! [`ActiveTheme::theme`]. The active theme is a GPUI global derived from the user's
//! [`ThemeMode`] preference and, for [`ThemeMode::System`], the OS appearance.

use gpui::{App, Global, Hsla, Pixels, Rgba, WindowAppearance, px, rgb, rgba};

/// The user's theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Follow the OS light/dark setting, including changes while running.
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

impl ThemeMode {
    fn resolve(self, system: WindowAppearance) -> Appearance {
        match self {
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
            ThemeMode::System => match system {
                WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            },
        }
    }
}

/// Colour tokens shared by every view.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    /// Editor page background.
    pub background: Hsla,
    /// Sidebar and other chrome around the page.
    pub surface: Hsla,
    pub foreground: Hsla,
    /// Secondary text: placeholders, timestamps, de-emphasised Markdown markers.
    pub muted: Hsla,
    pub border: Hsla,
    /// Caret, focused controls and the selected note.
    pub accent: Hsla,
    /// Text selection; translucent so the text stays readable on top of it.
    pub selection: Hsla,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            background: color(rgb(0xffffff)),
            surface: color(rgb(0xf5f5f4)),
            foreground: color(rgb(0x1d1d1f)),
            muted: color(rgb(0x6e6e73)),
            border: color(rgb(0xe2e2e0)),
            accent: color(rgb(0xd99a00)),
            selection: color(rgba(0xf2c14e66)),
        }
    }

    pub fn dark() -> Self {
        Self {
            appearance: Appearance::Dark,
            background: color(rgb(0x1c1c1e)),
            surface: color(rgb(0x252527)),
            foreground: color(rgb(0xe8e8ea)),
            muted: color(rgb(0x98989f)),
            border: color(rgb(0x3a3a3c)),
            accent: color(rgb(0xf2b32a)),
            selection: color(rgba(0xf2b32a4d)),
        }
    }

    pub fn for_appearance(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::light(),
            Appearance::Dark => Self::dark(),
        }
    }
}

fn color(rgba: Rgba) -> Hsla {
    rgba.into()
}

/// Font families and sizes. Heading sizes are multiples of the body size (PLAN §27).
pub mod typography {
    use super::*;

    /// Chrome (sidebar, buttons, dialogs).
    pub const UI_FONT_FAMILY: &str = if cfg!(windows) {
        "Segoe UI"
    } else {
        ".SystemUIFont"
    };
    pub const UI_FONT_SIZE: Pixels = px(13.);
    /// Small chrome icons. Windows' own icon font, so they match the system; other platforms
    /// get plain-text fallbacks.
    pub const ICON_FONT_FAMILY: &str = if cfg!(windows) {
        "Segoe MDL2 Assets"
    } else {
        UI_FONT_FAMILY
    };

    /// Note text. Notes are for reading and writing, so the body uses the UI sans-serif
    /// rather than a code font.
    pub const BODY_FONT_FAMILY: &str = UI_FONT_FAMILY;
    pub const BODY_FONT_SIZE: Pixels = px(15.);
    /// Line height as a multiple of the font size.
    pub const BODY_LINE_HEIGHT: f32 = 1.5;

    pub const H1_SCALE: f32 = 1.8;
    pub const H2_SCALE: f32 = 1.5;
    pub const H3_SCALE: f32 = 1.25;

    /// Preferred monospace families, best first. Cascadia Mono ships with Windows 11;
    /// Consolas with every Windows since Vista.
    pub const MONO_FONT_FAMILIES: [&str; 2] = ["Cascadia Mono", "Consolas"];

    /// Picks the monospace family to use from the installed font names.
    ///
    /// GPUI silently substitutes the UI font for a missing family (it does not walk a
    /// fallback list), so the choice has to be made up front.
    pub fn pick_mono_font_family<S: AsRef<str>>(installed: &[S]) -> &'static str {
        MONO_FONT_FAMILIES
            .into_iter()
            .find(|family| installed.iter().any(|name| name.as_ref() == *family))
            .unwrap_or(MONO_FONT_FAMILIES[MONO_FONT_FAMILIES.len() - 1])
    }

    /// The monospace family for this machine. Enumerates installed fonts (about 0.6 ms on a
    /// typical Windows 11 machine), so call it once and keep the result.
    pub fn mono_font_family(cx: &App) -> &'static str {
        pick_mono_font_family(&cx.text_system().all_font_names())
    }
}

struct ActiveThemeState {
    mode: ThemeMode,
    theme: Theme,
}

impl Global for ActiveThemeState {}

/// Read access to the active theme from any GPUI context.
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
    fn theme_mode(&self) -> ThemeMode;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        &self.global::<ActiveThemeState>().theme
    }

    fn theme_mode(&self) -> ThemeMode {
        self.global::<ActiveThemeState>().mode
    }
}

/// Installs the theme global. Must run before any window is opened.
pub fn init(mode: ThemeMode, cx: &mut App) {
    let theme = Theme::for_appearance(mode.resolve(cx.window_appearance()));
    cx.set_global(ActiveThemeState { mode, theme });
}

/// Changes the user's preference and redraws every window.
pub fn set_mode(mode: ThemeMode, cx: &mut App) {
    let appearance = mode.resolve(cx.window_appearance());
    apply(mode, appearance, cx);
}

/// Called when the OS switches between light and dark; only matters for
/// [`ThemeMode::System`].
pub fn system_appearance_changed(system: WindowAppearance, cx: &mut App) {
    let mode = cx.theme_mode();
    apply(mode, mode.resolve(system), cx);
}

fn apply(mode: ThemeMode, appearance: Appearance, cx: &mut App) {
    let state = cx.global::<ActiveThemeState>();
    if state.mode == mode && state.theme.appearance == appearance {
        return;
    }
    tracing::info!(?mode, ?appearance, "theme changed");
    cx.set_global(ActiveThemeState {
        mode,
        theme: Theme::for_appearance(appearance),
    });
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2 contrast ratio between two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        fn luminance(c: Hsla) -> f32 {
            let c = c.to_rgb();
            let channel = |v: f32| {
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
        }
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn text_tokens_are_readable_in_both_themes() {
        for theme in [Theme::light(), Theme::dark()] {
            for bg in [theme.background, theme.surface] {
                let body = contrast(theme.foreground, bg);
                assert!(
                    body >= 7.0,
                    "{:?} body text contrast {body}",
                    theme.appearance
                );
                let muted = contrast(theme.muted, bg);
                assert!(
                    muted >= 4.5,
                    "{:?} muted contrast {muted}",
                    theme.appearance
                );
            }
        }
    }

    #[test]
    fn mono_font_prefers_cascadia_and_falls_back_to_consolas() {
        use typography::pick_mono_font_family;
        assert_eq!(
            pick_mono_font_family(&["Arial", "Consolas", "Cascadia Mono"]),
            "Cascadia Mono"
        );
        assert_eq!(pick_mono_font_family(&["Arial", "Consolas"]), "Consolas");
        assert_eq!(pick_mono_font_family::<&str>(&[]), "Consolas");
    }
}
