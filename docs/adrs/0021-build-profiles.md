# 0021. Build profiles and panic strategy
Date: 2026-10-08
Status: Accepted

## Context
Scratchpad must ship a small, fast binary (PLAN §1, §37) while debug builds stay quick to
iterate on. GPUI pulls in a large dependency graph that is slow when unoptimised. Crash
recovery (PLAN §40) must not lose unsaved text when the app panics.

## Decision
- Release: `lto = "fat"`, `codegen-units = 1`, `strip = true`, default `opt-level = 3`
  (typing latency matters more than the last few hundred KB).
- Release uses `panic = "abort"`. Checked against gpui 0.2.2 on Windows: foreground tasks and
  all window/input/paint callbacks run inside `extern "system"` window procedures, and
  background tasks inside a WinRT thread-pool delegate (also `extern "system"`). Since Rust
  1.81 a panic unwinding out of such a function aborts, so for almost all app code unwinding
  could never be caught anyway. The exceptions are the `Application::run` launch closure and
  threads we spawn ourselves (e.g. the file watcher), where unwinding would kill only that
  thread and leave the app running in a broken state; aborting is preferable there too.
- Dev: dependencies (`[profile.dev.package."*"]`) build at `opt-level = 2`; workspace crates
  stay at 0. Measured debug time to first frame went from ~950 ms to ~810 ms; the larger
  win is per-frame layout and text shaping once the editor exists. The cost is a slower
  first build (~6 min cold); dependencies are cached afterwards and incremental builds of
  the app take ~5 s.
- `main.rs` sets `windows_subsystem = "windows"` only when `debug_assertions` are off, so
  release builds open no console window and debug builds keep stderr logs visible. Release
  builds log to `%LOCALAPPDATA%\Scratchpad\scratchpad.log` instead (previous run kept as
  `scratchpad.prev.log`).

## Consequences
- Crash recovery cannot rely on `catch_unwind` or on running code after a panic: the panic
  hook has no access to GPUI state (and the app state is mid-update when it fires). Recovery
  uses periodic snapshots (PLAN §40, `scratchpad_core::recovery`); the panic hook only logs.
- Release builds take minutes (fat LTO over GPUI); use dev builds for day-to-day work.
- Tests use the dev/test profile and keep unwinding, so `#[should_panic]` still works.
