# 0007. Editor engine benchmarks and budgets
Date: 2026-10-08
Status: Accepted

## Context
PLAN §37/§38/§55 ask for benchmarks from the start and documented budgets, so that a regression such as
typing going from 5 ms to 25 ms is visible. The engine is the part of the typing path we control fully.

## Decision
- Criterion benches live in `crates/scratchpad-editor/benches/editor.rs` and generate Markdown-like input in
  code: open 1 KB / 100 KB / 1 MB / 10 MB (plus 1 MB CRLF), one keystroke at the beginning / middle / end of
  a 10 MB document (including undo recording), deleting 1 KB and 1 MB from a 10 MB document, Down and
  Ctrl+Left across a 1 MB line (ASCII and flags), and case-(in)sensitive search in 1 MB.
- Run with `cargo bench -p scratchpad-editor` (add `-- --quick` for a fast check). Criterion's default
  features (plotting, rayon) are off to keep the dependency tree small.
- Engine budgets (release build, desktop CPU), well under the 16 ms input-to-render target except on huge lines:
  - keystroke in a 10 MB document: < 100 µs (measured 3–5 µs)
  - open 10 MB: < 100 ms (measured ~21 ms; 1 MB ~1.7 ms)
  - delete 1 MB from 10 MB: < 5 ms (measured ~0.5 ms)
  - case-insensitive search in 1 MB: < 5 ms (measured 1.5–2.5 ms, plus ~0.6 ms to copy the buffer)
  - Down / Ctrl+Left across a 1 MB line: < 50 ms (measured 12–42 ms / 6–16 ms); vertical movement is linear
    in the column, which only matters for huge single-line documents

## Consequences
- Budgets are checked by eye for now; CI threshold enforcement can be added once CI exists.
- Markdown parsing, incremental update and line styling benches and their budgets are in ADR 0041; layout
  benches will join this suite when that layer exists.
