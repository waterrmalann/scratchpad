---
name: run-scratchpad
description: Launch a debug build of Scratchpad on scratch folders and drive it with computer use, to see a change working in the real app.
---

# Run Scratchpad

## Launch

Use scratch folders so the user's notes and settings are never touched. Seed any notes the check needs.

```bash
S="<scratchpad dir>/try"; mkdir -p "$S/notes" "$S/config"
printf '# Demo\n\n- [ ] task\n' > "$S/notes/Demo.md"
cargo build -p scratchpad
SCRATCHPAD_NOTES_DIR="$S/notes" SCRATCHPAD_CONFIG_DIR="$S/config" \
  ./target/debug/scratchpad.exe "$S/notes/Demo.md"   # run_in_background
```

## Drive

- **Access:** call `request_access` with the full path `<repo>\target\debug\scratchpad.exe`. The name "Scratchpad" grants the *installed* copy, and the debug window stays masked.
- **Focus:** `open_application` won't take the debug path, and launching "Scratchpad" opens the installed app with the user's real notes. Bring the window forward from PowerShell instead:
  ```powershell
  $p = Get-Process scratchpad | ? { $_.Path -like '*target\debug*' } | select -First 1
  (New-Object -ComObject WScript.Shell).AppActivate($p.Id)
  ```
- **Typing:** the `type` action pastes through the clipboard, and pasted text skips typed replacements, list continuation and similar. To test keystroke behaviour, send `key` actions one character at a time (`shift+comma` for `<`, `slash`, `shift+9`).
- **Keys:** zoom in is `ctrl+=` (not `ctrl+equal`).
- **Note list:** clicking a note leaves focus in the list. Press `Return` to move into the note before typing.
- Use `zoom` on regions to inspect small details: the status bar, inline icons, task boxes.

## Stop

```powershell
Get-Process scratchpad | ? { $_.Path -like '*target\debug*' } | Stop-Process
```

The background launch then reports exit 127. That's expected.
