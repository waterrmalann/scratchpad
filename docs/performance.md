# Performance

## Budgets (PLAN §37)

| Metric | Target |
| --- | --- |
| Cold startup to first frame | < 250 ms where realistically achievable |
| Warm startup to first frame | < 100 ms |
| Input-to-render | < 16 ms |
| Idle memory | < 50 MB (stretch < 30 MB) |
| Large documents | 1 / 5 / 10 MB Markdown stay usable |

## Measurements

Release build, Windows 11, RTX 3050 Laptop GPU, 125 % scaling. Startup is from the log
(`first frame shown`, measured from the start of `main`); memory from PowerShell
`Get-Process` 3 s after launch.

| Date | Commit | Build | First frame (warm) | Working set | Private bytes | Binary |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-10-08 | Milestone 0 shell | release | ~550 ms | ~50 MB | ~76 MB | 5.5 MB |
| 2026-10-08 | Milestone 0 shell | dev | ~810 ms | ~54 MB | ~76 MB | 14.7 MB |
| 2026-10-08 | Packaging (notes, editor, icon) | release | not measured | not measured | not measured | 7.1 MB |

The packaging row is the size only: `target/release/scratchpad.exe` is 7,075,840 bytes, of which the
embedded icon and version resource are about 12 KB. The per-user MSI built from it
(`packaging/build-installer.ps1`, ADR 0091) is 2.45 MB.

Where the startup time goes (release log): ~20 ms to GPUI platform init, ~315 ms creating the
Direct3D 11 device, ~160 ms DirectWrite setup, ~60 ms creating and drawing the window. Almost
all of it is inside GPUI/driver initialisation before any Scratchpad code runs, so the
< 250 ms target needs work at that level (GPUI picks the first DXGI adapter, here the
discrete GPU).

To reproduce: run `target/release/scratchpad.exe` and read `%LOCALAPPDATA%\Scratchpad\scratchpad.log`
(`SCRATCHPAD_LOG=debug` for more detail). Dev builds log to stderr.

## Editor: typing and scrolling

Same machine, release build with the editor view (Milestone 1). Every frame caused by input logs
an `input painted` trace event with two durations:

- `latency`: from the moment the editor has applied the key, IME text or wheel event (engine edits
  take microseconds, ADR 0007) to the end of the editor's paint, including the wait for GPUI to
  start the frame;
- `frame`: the editor's own layout (scrolling, shaping, hit geometry) and paint time.

Documents are generated Markdown notes (headings, lists, wrapped paragraphs, CJK), typed into with
`SendKeys` at about 30 characters per second, half at the end and half at the very start of the
document, and scrolled with 120-unit wheel steps every 16 ms. Times in milliseconds.

| Document | Action | Events | Latency p50 / p90 / max | Frame p50 / p99 / max |
| --- | --- | --- | --- | --- |
| 1 MB, 29k lines | typing | 224 | 4.0 / 6.0 / 11.2 | 0.59 / 1.25 / 1.55 |
| 10 MB, 289k lines | typing | 156 | 3.2 / 5.8 / 8.8 | 0.74 / 1.18 / 1.57 |
| 10 MB, 289k lines | wheel scrolling | 351 | 3.8 / 6.5 / 7.5 | 0.62 / 1.13 / 1.67 |

Input-to-render stays well under the 16 ms budget, and the editor's share of a frame is under
2 ms regardless of document size: only the visible lines (about 30 here) are painted, and an edit
re-shapes just the line it touched. Most of the latency is waiting for the next frame. Memory 4 s
after opening: empty note 48 MB working set / 75 MB private, 1 MB note 52 / 78 MB, 10 MB note
63 / 89 MB. The release binary is 6.1 MB.

To reproduce: run with `SCRATCHPAD_LOG=info,scratchpad::editor_view=trace` and read the
`input painted` lines from the log. To open a large document, put it in a folder and start the
app with `SCRATCHPAD_NOTES_DIR` pointing there.

### With Markdown rendering and live preview (Milestones 3–4)

Same machine and build, after Markdown styling, hidden markers and per-line layout keys (ADR
0070, 0071). The documents repeat a Markdown-heavy sample note (headings, emphasis, inline code,
links, tasks, nested lists, a quote, a rule, a fenced code block) to 1 MB (41k lines) and 10 MB
(407k lines). Input no longer comes from `SendKeys`: a temporary in-process driver types a
character every 33 ms through the same edit path as a keystroke (`insert_typed`), 150 at the end
of the document and 150 at its start, then scrolls 300 wheel steps of 120 px every 16 ms. Times in
milliseconds; the first event after jumping to the end is listed apart.

| Document | Action | Events | Latency p50 / p90 / max | Frame p50 / p99 / max |
| --- | --- | --- | --- | --- |
| 1 MB | typing | 298 | 4.7 / 7.4 / 9.3 | 0.85 / 1.78 / 2.42 |
| 1 MB | scrolling | 300 | 4.6 / 7.3 / 10.7 | 0.82 / 1.79 / 2.07 |
| 10 MB | typing | 298 | 4.5 / 7.4 / 10.1 | 0.92 / 1.80 / 5.03 |
| 10 MB | scrolling | 300 | 4.5 / 7.3 / 12.6 | 0.53 / 1.19 / 1.41 |
| 1 MB | jump to end + first keystroke | 1 | 10.6 | 6.1 |
| 10 MB | jump to end + first keystroke | 1 | 25.0 | 16.7 |

Typing stays far inside the 16 ms budget; the editor's frame grew from ~0.6–0.7 ms to ~0.9 ms
(styled runs, several shaping segments per line, shapes). A keystroke re-styles the visible lines
(one `styled_lines` query each, microseconds) and re-shapes only lines whose style key changed,
normally just the edited one; caret blinks re-shape nothing (tested in
`tests/live_preview.rs`). The one slow frame is the jump into a never-viewed part of the 10 MB
note, which parses the regions there and shapes a new screen of lines.

## Startup with notes

Release build with the note session wired in (open the last note, autosave, watcher, recovery
scan), on a folder with two small notes; warm starts while other builds were running on the
machine, so absolute numbers are noisy. From the log (`main window created`, `first frame shown`,
`first note shown`, all measured from the start of `main`):

| Run | Window created | First frame | First note in editor |
| --- | --- | --- | --- |
| 3 | 759 ms | 766 ms | 764 ms |
| 4 | 630 ms | 635 ms | 635 ms |
| 5 | 542 ms | 546 ms | 544 ms |
| 6 | 586 ms | 590 ms | 589 ms |

(The first two runs, 1.15-1.36 s, were cold after the binary was rebuilt.) Nothing note-related
runs before the window exists: only the config file is read. Once it does, the folder is opened
and the last note read on the background executor; the note is in the editor 2-10 ms after the
window, so it is painted in the first or second frame. Listing the folder and the recovery scan
follow in the background. Startup time is still dominated by GPUI/Direct3D
initialisation (see above).
