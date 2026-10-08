# 0091. Per-user MSI installer built with WiX
Date: 2026-10-08
Status: Accepted

## Context
Scratchpad should install like a normal Windows app without elevation, upgrade in place, and be
removable from Settings (PLAN §52 Milestone 6, §59). The build must stay reproducible from the
repo and must not require installing software system-wide.

## Decision
- An MSI built by WiX v5.0.2 (`packaging/scratchpad.wxs`). It is the last release published
  under the plain MS-RL licence; the NuGet packages for 6.0.0 and later carry the Open Source
  Maintenance Fee EULA, which is not worth taking on for a free MIT-licensed app.
- WiX is a repo-local dotnet tool pinned in `.config/dotnet-tools.json`; it is restored into the
  user's NuGet cache and nothing is installed globally. `packaging/build-installer.ps1`
  builds the release binary, reads the version from `cargo metadata` and writes
  `target/installer/Scratchpad-<version>-x64.msi`, then runs the ICE validation that `wix build`
  skips (ICE61 and ICE91 are suppressed: same-version upgrades and the fixed per-user folder are
  intended).
- `Scope="perUser"`: installs to `%LOCALAPPDATA%\Programs\Scratchpad`, adds a Start Menu
  shortcut and an Apps & features entry with the app icon. The fixed `UpgradeCode`
  plus `MajorUpgrade` replaces an older install in place (also at the same version, so a
  rebuilt MSI replaces an earlier build) and refuses downgrades. The one
  component is keyed on an `HKCU` registry value, as per-user components must be, and removes
  the (then empty) install folders on uninstall, which ICE64 requires for the user profile.
- The package has no UI sequence: a double-click shows only Windows Installer's progress
  dialog. A wizard needs the WiX UI extension, which is not worth another dependency for an
  app with no install options.
- Uninstalling removes only what the installer put down; notes, settings
  (`%APPDATA%\Scratchpad`) and logs (`%LOCALAPPDATA%\Scratchpad`) stay, since they are the
  user's data and live outside the install folder.

## Consequences
- The MSI is not code-signed, so SmartScreen may warn on first run. Signing is future work.
- No code or build step touches the registry beyond the installer's own keys.
- The build machine needs the .NET SDK (6.0 or newer) and network access the first time.
