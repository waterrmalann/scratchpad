# 0111. Saves and recovery snapshots share the rope instead of copying the text
Date: 2026-10-09
Status: Accepted

## Context
While the user types without pausing, the session writes a crash recovery snapshot every 500 ms
(ADR 0064) and saves every 2 s (PLAN §7). Each copied the whole note on the UI thread (rope to
`String` through `Display`, then into an `Arc<str>`): 17.4 ms for a 10 MB note, a dropped frame
several times a second in the middle of typing (PLAN rule 8).

## Decision
- `Buffer::snapshot` returns a `TextSnapshot`: a clone of the rope (O(1), 22 ns) and the line
  ending; `to_text` serializes it exactly as `Buffer::to_text` would have at that moment.
- `Job::Snapshot` of the session's writer carries one; the writer serializes it on the background
  executor. Every snapshot (while typing, while a notice is open, for new notes) is taken this way.
- `Job::Save` carries a `SaveText`: the snapshot plus the string the writer makes of it, once, on
  its thread. The session reads that string after the save as the text now on disk, and a save
  queued behind it expects the file to hold the same string, so the check for changes by other
  programs (ADR 0060) compares exactly what it compared before. A new note's first save still
  copies on the UI thread (it checks the text is not blank); it happens once per note.
- The text and the moment it is taken are unchanged, so ordering and recovery behave as before.
- After a snapshot the editor's next edit copies the few rope nodes it touches: snapshot plus
  keystroke 6.7 µs against 3-5 µs for a keystroke alone (`cargo bench -- save_10MB`).
- Serializing builds the string with `String::from(&Rope)`, which reserves its length; `Display`
  grew it chunk by chunk (17.4 ms -> 9.2 ms for 10 MB with the `Arc<str>`, now off the UI thread).
  `RecoveryStore::write` serializes borrowed fields instead of copying the text again.

## Consequences
- A queued save or snapshot keeps the old rope nodes alive until it is written, typically
  milliseconds; a written save also holds its string until the session has read it.
