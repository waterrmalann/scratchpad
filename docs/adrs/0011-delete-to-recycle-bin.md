# 0011. Delete moves notes to the OS recycle bin
Date: 2026-10-08
Status: Accepted

## Context
PLAN §41 wants deletes to be recoverable and prefers the platform's trash over a custom
`.trash` folder. Tests, including the app's headless GPUI tests, must never put files into the
developer's real recycle bin.

## Decision
`NoteStore::delete` calls a `fn(&Path) -> io::Result<()>` stored in the store. `NoteStore::open`
installs the `trash` crate (`trash::delete`, errors converted to `io::Error`); tests build the
store with `NoteStore::with_deleter` and pass a function that moves the file into a temp folder.
It is a plain function pointer, not a trait: there is exactly one production implementation and
the seam only exists for tests.

## Consequences
- Users can restore deleted notes from the Recycle Bin / Trash with their original name.
- No custom permanent-deletion workflow and no `.trash` folder in the notes directory.
- `trash` may fail (network drives, volumes without a bin). The error is returned and the app
  should say the note could not be deleted rather than falling back to permanent deletion.
- The real `trash::delete` path is not exercised by automated tests.
