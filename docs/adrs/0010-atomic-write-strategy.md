# 0010. Atomic writes: temp file, sync, rename
Date: 2026-10-08
Status: Accepted

## Context
Notes autosave every few hundred milliseconds (PLAN §7), so a crash, power loss or full disk can
happen mid-write. Writing in place would leave a truncated note. The same primitive is needed for
the config file and recovery snapshots. Windows adds two wrinkles: a file cannot be renamed while
we hold it open, and antivirus scanners, indexers and cloud-sync clients briefly open files without
`FILE_SHARE_DELETE`, which makes a rename over the target fail with access denied or a sharing
violation.

## Decision
`write_atomic(path, bytes)` in `scratchpad-core`:
1. Create `.scratchpad-<pid>-<n>.tmp` next to the target with `create_new` (same volume, so the
   rename is a metadata operation; hidden and not `.md`, so listing and watching ignore it; short,
   so targets with names near the 255-character limit can still be saved).
2. Write, copy the target's creation time (Windows), `sync_all`, drop the handle.
3. `fs::rename` over the target. Rust's Windows implementation uses `MoveFileEx` /
   POSIX-semantics rename, which replaces an existing file atomically.
4. On Windows, retry the rename up to four times (10-80 ms) for error codes 5, 32 and 33. Other
   errors fail immediately.
5. On any failure remove the temp file; the original is never touched. A target that is a
   directory is rejected up front.

## Consequences
- A save leaves either the old or the new content, never a mix.
- Replacing by rename drops file attributes and ACLs of the original (the creation time is
  copied) and breaks hard links and symlinks. A read-only note is not overwritten on Windows (the rename is denied), which is the
  desired behaviour; on Unix it would be replaced.
- The parent directory is not fsynced (not possible on Windows); on power loss the rename may be
  lost but the old file stays intact.
- A crash between steps 1 and 3 leaves a `.tmp` file, which Explorer shows. `NoteStore::open`
  deletes ones older than a minute (younger ones may belong to another instance mid-save).
