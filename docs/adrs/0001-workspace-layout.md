# 0001. Three-crate workspace layout
Date: 2026-10-08
Status: Accepted

## Context
The product plan sketches five crates (`scratchpad`, `-ui`, `-editor`, `-core`, `-fs`) but also warns against
creating crates prematurely. The two things that matter most are keeping the editor engine testable without a UI
framework, and keeping filesystem/notes logic independent of both the editor and GPUI.

## Decision
Use three crates:
- `scratchpad-editor`: the editing engine (buffer, coordinates, cursor, selection, history, Markdown
  decorations). No GPUI dependency.
- `scratchpad-core`: notes, storage, search, config, recovery and file watching. No GPUI dependency and no
  dependency on the editor crate. This merges the planned `-fs` crate into core.
- `scratchpad`: the GPUI binary, including all views. This merges the planned `-ui` crate into the app.

## Consequences
- The engine and storage logic compile and test in seconds without building GPUI.
- UI code has a single home. If compile times of the binary become a problem, views can be split into a
  `scratchpad-ui` crate later without touching the engine crates.
- Dependency direction is strictly `scratchpad -> {scratchpad-editor, scratchpad-core}`.
