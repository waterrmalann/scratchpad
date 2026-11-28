# 0095. The installer offers Scratchpad in "Open with" for .md, .markdown and .txt
Date: 2026-11-28
Status: Accepted

## Context
ADR 0092 put file associations off until the app could open a file passed on the command line.
ADR 0145 added that: `scratchpad.exe <file>` opens a note of the notes folder as that note and
edits any other file in place. Opening a `.md` from Explorer still meant Open with > Choose
another app and browsing to the exe.

## Decision
- The MSI's `OpenWith` component writes per-user entries under `HKCU\Software\Classes`:
  - `Applications\scratchpad.exe`: `FriendlyAppName` "Scratchpad", `DefaultIcon` (the exe's icon),
    `shell\open\command` = `"<install folder>\scratchpad.exe" "%1"`, and `SupportedTypes` listing
    `.md`, `.markdown` and `.txt`.
  - Two ProgIDs, `Scratchpad.Markdown` ("Markdown Document") and `Scratchpad.Text` ("Text
    Document"), each with the same icon and open verb. Two so a user who makes Scratchpad the
    default for `.txt` does not see text files described as Markdown.
  - A named value under each extension's `OpenWithProgids`: `.md` and `.markdown` name
    `Scratchpad.Markdown`, `.txt` names `Scratchpad.Text`.
- No extension's default value is written: Scratchpad never takes over the default handler. The
  user makes it the default from Open with ("Always") or Settings > Default apps, and Windows
  protects that choice for them.
- Uninstall removes the app's own keys (`ForceDeleteOnUninstall`) and only its named values under
  the shared extension keys. The component is keyed on `HKCU\Software\Scratchpad\OpenWith`, like
  `Application` (ADR 0091), and its GUID must never change.
- A second launch with a file keeps separate windows: every launch is its own instance, as with
  Notepad (ADRs 0120 and 0155). Opening a file from Explorer while the app runs opens a second
  window showing that file. A single instance with a hand-off would need a named pipe or window
  message and a way to bring the running window forward, which GPUI 0.2.2 does not offer, and
  ADR 0155 already makes two instances safe for recovery snapshots.

## Consequences
- Explorer offers Scratchpad, with its name and icon, in Open with for the three extensions, as
  a recommended app, beside whatever other editors registered.
- Two windows editing the same note see each other's saves as changes by another program
  (ADR 0063): nothing is overwritten unasked, but the user may be asked which text to keep.
- The portable zip writes nothing to the registry; its users still pick `scratchpad.exe` with
  Choose another app. A Settings button to register the portable exe is not planned yet.
- A Microsoft Store (MSIX) package, when there is one, must declare the same three extensions
  as a file type association in its manifest instead: packaged apps do not write these keys.
- Amends ADR 0092.
