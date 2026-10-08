# 0022. Theme tokens and typography
Date: 2026-10-08
Status: Accepted

## Context
PLAN §27 and §33 ask for system/light/dark themes, a small set of colour tokens, no literal
colours in view code, and typography tuned for reading and writing rather than code.

## Decision
- `theme::Theme` is a flat struct of `Hsla` tokens: exactly the seven from the plan
  (`background`, `surface`, `foreground`, `muted`, `border`, `accent`, `selection`). Add a
  token (e.g. hover, code background) only when a view needs it.
- The active theme and the user's `ThemeMode` live in one GPUI global, read through the
  `ActiveTheme` extension trait (`cx.theme()`). `theme::set_mode` changes the preference;
  the root view observes window appearance and calls `theme::system_appearance_changed`, so
  `System` follows the OS live. Changes call `cx.refresh_windows()`.
- Palette: neutral greys with a warm amber accent (caret, selection) in the spirit of
  iCloud Notes. A unit test enforces WCAG contrast (body ≥ 7:1, muted ≥ 4.5:1) on every
  background token in both themes.
- Typography constants live in `theme::typography`: Segoe UI for chrome and body text
  (`.SystemUIFont` elsewhere), 13 px UI, 15 px body at 1.5 line height, heading scales
  1.8 / 1.5 / 1.25.
- Monospace: Cascadia Mono, else Consolas. GPUI's DirectWrite backend silently substitutes
  the UI font for a missing family instead of walking a fallback list, so
  `typography::mono_font_family(cx)` picks from the installed fonts. Enumeration measured
  ~0.6 ms; call it once and cache the result.

## Consequences
- The native title bar follows the OS appearance, not an explicit Light/Dark override
  (GPUI 0.2.2 sets DWM dark mode from the system setting only).
- Config persistence of `ThemeMode` is left to the config work in `scratchpad-core`; the app
  currently starts in `System`.
