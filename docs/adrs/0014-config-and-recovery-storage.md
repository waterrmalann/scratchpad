# 0014. Config and recovery data live outside the notes folder as small JSON files
Date: 2026-10-08
Status: Accepted

## Context
The app must remember window geometry, sidebar width, theme and the last note (PLAN §32), and
recover unsaved text after a crash (PLAN §40), without adding metadata files to the user's notes
(PLAN §31) and without ever failing startup.

## Decision
- Config is `<config dir>/Scratchpad/config.json` (`%APPDATA%` on Windows). Each field is
  deserialized independently: a missing field, a wrong type or an unknown theme resets only that
  field to its default (and logs a warning). Unreadable, empty or non-object files give the
  defaults. Unknown keys are ignored. Saving is atomic (ADR 0010).
- Missing values are `None` (e.g. no sidebar width) so the UI owns its defaults; core does not
  validate geometry.
- Recovery snapshots are one JSON file per note (`{note_path, text, saved_at}`) in
  `<local data dir>/Scratchpad/recovery`, named by an FNV-1a hash of the note path (stable across
  Rust versions, unlike `DefaultHasher`). The store takes its folder as an argument, so tests use
  temp dirs. Corrupt snapshots are skipped and left on disk.
- All locations are injectable; `default_config_path`, `default_notes_dir` and
  `RecoveryStore::default_dir` provide the platform defaults.

## Consequences
- A hand-edited config with one bad value still loads; a downgrade drops keys it does not know on
  its next save.
- Snapshots of renamed or deleted notes are orphaned unless the app removes them under the old
  path; at startup the app should ignore snapshots whose text equals the file on disk.
