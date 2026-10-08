# 0082. SCRATCHPAD_CONFIG_DIR keeps trial runs away from the user's settings
Date: 2026-10-08
Status: Accepted

## Context
`SCRATCHPAD_NOTES_DIR` points the app at a scratch notes folder, but the config file
(`%APPDATA%\Scratchpad\config.json`) and the recovery snapshots (`%LOCALAPPDATA%\Scratchpad\
recovery`) still came from the known folders. `dirs` asks Windows for them directly, so changing
`APPDATA` does not redirect them. Trying the app by hand (e.g. checking the settings panel) wrote
window bounds, the theme and the notes folder into the user's real settings.

## Decision
- If `SCRATCHPAD_CONFIG_DIR` is set and not empty, the config file is `<dir>\config.json`,
  recovery snapshots go to `<dir>\recovery` and release builds log to `<dir>\scratchpad.log`.
  Otherwise the platform folders are used as before.
- It sits next to `SCRATCHPAD_NOTES_DIR` in `run`; tests keep passing explicit paths in
  `Storage`.

## Consequences
- `SCRATCHPAD_NOTES_DIR` plus `SCRATCHPAD_CONFIG_DIR` run the app without touching any of the
  user's data, logs included (debug builds log to the console).
