# Performance

Performance is a product feature (PLAN §37-38). Numbers below are from a Windows 11 laptop
(Core i7-12700H, 32 GB, RTX 3050 Laptop GPU driving the only display, Iris Xe idle, 125 % scaling),
release builds, usually while other builds ran: they are medians with ranges, not lab figures.

## Budgets and where we are

| Metric | Budget (PLAN §37) | Now (2026-10-09) |
| --- | --- | --- |
| Warm start to first frame | < 100 ms (cold < 250 ms) | 438 ms median (432-481) |
| Input to render | < 16 ms | 4.5 ms p50, 12.6 ms max (10 MB note) |
| Idle memory | < 50 MB (stretch < 30 MB) | 52 MB working set, 77 MB private |
| 1 / 5 / 10 MB notes | usable | typing and scrolling as above |

Startup and private bytes are over budget, and nearly all of both is GPUI and the graphics driver,
not Scratchpad (ADR 0110, ADR 0113).

## How to measure

- **Startup and memory**: `powershell -File scripts/measure.ps1 [-Runs 7] [-Build] [-Note big.md]`.
  It runs the release build on scratch folders (never your notes or settings) and prints, from
  process creation: `main` reached, Direct3D device created, GPUI platform initialised, window
  created, first frame, first note shown, then working set and private bytes after 3 s.
  `-Exe other.exe` measures another build; alternate builds when comparing.
- **Timing guards**: `cargo test --release -p scratchpad-editor -p scratchpad-core --test perf --
  --nocapture` (or `measure.ps1 -Tests`). They fail only far above today's times (ADR 0114) and
  print every time.
- **Benches**: `cargo bench -p scratchpad-editor` and `cargo bench -p scratchpad-core` (add
  `-- --quick` for a fast pass, or a name filter such as `-- save_10MB`).
- **Typing and scrolling**: run with `SCRATCHPAD_LOG=info,scratchpad::editor_view=trace` and read
  the `input painted` lines: `latency` is from the input being applied to the end of the editor's
  paint, `frame` the editor's own layout and paint.

## Startup

Median of 7 runs each, alternating builds, machine ~16 % busy (ms from process creation):

| Build | `main` | Device created | Platform ready | Window | First frame | Note shown |
| --- | --- | --- | --- | --- | --- | --- |
| Before (11bb693) | 15 | 331 | - | 537 | 539-541 | 540-545 |
| Now | 16 | 334 | 391-396 | 437 | 438-439 | 442-443 |
| Now, 10 MB note | 16 | 343 | 402 | 457 | 460 | 499 (before: 613) |

GPUI's platform initialisation takes ~375 ms: a DXGI factory, two Direct3D 11 device creations
(~180 + ~115 ms on the NVIDIA GPU) and DirectWrite's font collection, which now loads on a helper
thread meanwhile (~160 -> ~55 ms on the main thread). GPUI then creates the window in ~45 ms. Our
code before the first frame takes under 10 ms; the note is read after the window exists. What was
tried and rejected (adapter choice, holding a device, DirectComposition) is in ADR 0110.

## Memory

Idle with one short note: 52 MB working set, 77 MB private bytes (unchanged by this pass). An empty
GPUI window costs 48-53 / 74-76 MB on its own, a Direct3D device alone ~30 / ~40 MB; Scratchpad's
own share is 2-4 MB (ADR 0113). With a 10 MB note open: 79 / 104 MB. What the app keeps after use is
bounded: layout cache (ADR 0030), note search cache freed when the search closes, undo history 32 MiB
of text (ADR 0112), saves and recovery snapshots share the rope (ADR 0111).

## Engine (criterion, release)

| Operation | Time |
| --- | --- |
| Open 1 MB / 10 MB | ~1.7 ms / ~21 ms (ADR 0007) |
| Keystroke in 10 MB, with undo | 3.5-5.5 µs |
| Keystroke + Markdown restyle, 10 MB (two keystrokes) | 20-95 µs (ADR 0041) |
| Style 50 visible lines | ~27-36 µs |
| Save or recovery snapshot of 10 MB, UI thread | 22 ns + ~3 µs on the next keystroke (was 17.4 ms) |
| The same, serialized by the writer (background) | 9.2 ms |
| Find bar: keep ~1M matches in step, edit at start / end | 0.68 ms / 0.7 µs (was 0.86 / 0.63 ms) |
| Note search, 1,000 notes: first / warm | ~85-134 ms / 1.5-5 ms |

## Typing and scrolling (Milestones 3-4)

Markdown-heavy notes repeated to 1 MB and 10 MB; an in-process driver typed a character every
33 ms at the end and the start, then scrolled 300 wheel steps of 120 px every 16 ms (ms):

| Document | Action | Latency p50 / p90 / max | Frame p50 / p99 / max |
| --- | --- | --- | --- |
| 1 MB | typing | 4.7 / 7.4 / 9.3 | 0.85 / 1.78 / 2.42 |
| 1 MB | scrolling | 4.6 / 7.3 / 10.7 | 0.82 / 1.79 / 2.07 |
| 10 MB | typing | 4.5 / 7.4 / 10.1 | 0.92 / 1.80 / 5.03 |
| 10 MB | scrolling | 4.5 / 7.3 / 12.6 | 0.53 / 1.19 / 1.41 |
| 10 MB | jump to end + first keystroke | 25.0 | 16.7 |

Most of the latency is waiting for the next frame; the editor's share stays under 2 ms because only
visible lines are shaped and an edit re-shapes only the lines whose style changed (ADR 0030, 0071).
These numbers predate this pass, which removed the 17 ms UI-thread copies that non-stop typing caused
every 500 ms (recovery snapshots) and every 2 s (saves); they were not re-measured with the driver.

## Binary

`target/release/scratchpad.exe` is 7.8 MB (icon and version resource ~12 KB); the MSI is ~2.5 MB.
