# 0090. App icon and executable resources
Date: 2026-10-08
Status: Accepted

## Context
The Windows executable needs an icon (title bar, taskbar, Start Menu, installer) and version
information (PLAN §59). The icon has to be reproducible from the repo and stay legible at 16 px.

## Decision
- The design is an amber rounded tile (the theme's accent) holding a white note sheet with a
  folded corner and a heading plus text lines. `assets/icons/scratchpad.svg` is the master;
  `assets/icons/generate-icon.ps1` draws the same shapes with System.Drawing (available in
  Windows PowerShell, nothing to install) and writes `scratchpad.ico` with PNG-compressed
  entries at 16, 20, 24, 32, 40, 48, 64 and 256 px. The `.ico` is committed because it is a build
  input. Up to 32 px the script lays the text lines out on whole pixels; anti-aliased
  one-pixel lines turned to grey smears.
- `crates/scratchpad/build.rs` embeds the icon and version information using `winresource`
  (Windows-only build dependency; it embeds only when the *target* OS is Windows). It was chosen
  over `embed-resource` because it fills the version resource (file/product version) from
  Cargo's package metadata, so the version has one source of truth; the cost is a few small
  build-time crates and no runtime code.
- GPUI 0.2.2 registers its window class with the executable's icon resource ID 1
  (`LoadImageW(module, 1, ...)` in `platform/windows/platform.rs`), and `winresource` writes
  the icon as ID 1, so no application code is needed for the window and taskbar icon.

## Consequences
- Building needs the Windows SDK's `rc.exe`; `winresource` finds it through the SDK's registry
  entries, so it need not be on `PATH`. `build.rs` fails the build when embedding fails.
- Changing the design means editing the SVG and the script together and re-running the script.
