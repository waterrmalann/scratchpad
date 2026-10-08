# 0114. Measuring performance and guarding against regressions
Date: 2026-10-09
Status: Accepted

## Context
PLAN §38 lists what to measure (startup, opening 1 KB-10 MB, inserting, deleting, Markdown parsing,
layout, scrolling, search, memory) and §55 asks that a 5 ms to 25 ms regression fails or becomes
immediately visible. There is no CI yet, and the machine the numbers come from is often busy.

## Decision
- Criterion benches give the numbers: `cargo bench -p scratchpad-editor` (open, insert, delete,
  save and snapshot, motions, in-note search and find-match upkeep, Markdown scan/parse/keystroke/
  styling) and `cargo bench -p scratchpad-core` (note search). ADRs 0007 and 0041 hold the budgets.
- Layout and scrolling need GPUI's text system and a window, so they are not benchmarked headlessly;
  the editor view logs an `input painted` trace event per input frame instead (docs/performance.md).
- `tests/perf.rs` in the editor and core crates time the hot paths and fail at limits 10x or more
  above what they measure, so they do not flake on a loaded machine yet catch an order-of-magnitude
  regression (a copy of the document per keystroke, a reparse of everything). They run in release
  builds (`cargo test --release -p scratchpad-editor -p scratchpad-core --test perf`) and are
  ignored in debug builds; `--nocapture` prints every time, so smaller drifts are visible.
- `scripts/measure.ps1` measures startup and idle memory of the release binary: N runs on scratch
  notes and config folders, timings from the log measured from process creation, working set and
  private bytes after a pause, every run plus the median. `-Tests` runs the guards first.

## Consequences
- A 5x regression of a fast path (e.g. 7 µs to 35 µs per keystroke) is visible in the printed
  times and benches, not a failure; only regressions past the limits fail.
- Startup and memory have no automatic limit: they depend on the GPU driver and machine load, so
  docs/performance.md records medians and ranges with the machine's state instead.
