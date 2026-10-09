# 0155. Recovery snapshots belong to the instance that wrote them
Date: 2026-10-09
Status: Accepted

## Context
A second instance is allowed (ADR 0120), and `scratchpad.exe <file>` (ADR 0145) makes one
likely: opening a file from Explorer while the app runs. Both used one recovery folder, so the
second offered the first one's live snapshots as crash leftovers. Discard deleted text that then
lived only in the first one's memory; Restore copied it into a second editor. Release builds
abort on panic (ADR 0021), so telling a crashed instance from a running one cannot rely on code
running at the end.

## Decision
- Each instance keeps its snapshots in a folder of its own, `recovery/<pid>-<time>-<n>/`, made
  when it first needs one, next to a lock file `<same name>.lock` that it holds open while it runs.
  On Windows the lock file is created with no sharing (`share_mode(0)`): it is locked from the
  moment it exists, nobody else can open or delete it, and the OS closes it however the process
  ends. Elsewhere `File::try_lock` is a best-effort stand-in. No new dependency, no unsafe code.
- At startup `RecoveryStore::leftovers` (in the background) tries each lock. One it can open
  belongs to an instance that is gone: its snapshots move into this instance's folder unless
  they match their note, then its emptied folder and lock file go. Snapshots in the recovery
  folder itself, from earlier versions, are taken over the same way. Running instances' are
  left alone.
- Taken over, they are this instance's own: Restore, Discard and the protection of offered text
  (ADR 0120) work as before, a third instance does not offer them as well, and text neither
  restored nor discarded is offered again after this instance ends.
- A snapshot of a note this instance already holds one of (two instances crashed with text in
  the same note) stays where it is and is offered on the next start, never overwritten.
- `leftovers` no longer takes the start time: an instance's own snapshots are never leftovers.
  `list` still shows the snapshots of every instance.

## Consequences
- Each run leaves its lock file and folder behind until the next start removes them.
- A folder holding a corrupt snapshot is never removed, as corrupt snapshots were kept before.
- Snapshots in the old place have no lock: an instance of an earlier version running alongside
  has its live ones taken over, and two instances starting at the same moment may both offer one.
- A program holding a dead instance's lock file open (a virus scan) postpones its leftovers to
  a later start.
- Tests simulate a crash by dropping the instance's store: the app's views must be released.
