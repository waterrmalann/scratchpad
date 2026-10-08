# 0113. Where idle memory goes, and what we keep after use
Date: 2026-10-09
Status: Accepted

## Context
PLAN §37 asks for < 50 MB idle (stretch < 30 MB) and to treat memory regressions seriously. Idle
Scratchpad (release, one short note) measures ~77 MB private bytes and ~52 MB working set. Before
cutting anything we needed to know whose memory that is.

## Decision
- Measured on the RTX 3050 / Iris Xe laptop, 125 % scaling, 4 s after start, separate processes:

  | Process | Private | Working set |
  | --- | --- | --- |
  | Rust program that only sleeps | 1.7 MB | 8 MB |
  | + DirectWrite system font collection | 3.0 MB | 11 MB |
  | A Direct3D 11 device alone (NVIDIA / Intel) | 40 / 38 MB | 28 / 32 MB |
  | Empty GPUI window, 1100x720, one line of text | 74-76 MB | 48-53 MB |
  | Scratchpad, empty note | 77 MB | 52 MB |

  So ~40 MB is the graphics driver's per-device state, ~30 MB GPUI's window (swap chain and
  DirectComposition surfaces at 125 %, glyph and path atlases, buffers) and only 2-4 MB ours. The
  < 50 MB private target is below what a GPUI window costs on this machine; the working set is near
  50 MB. Neither can move without changing GPUI's renderer, which we do not fork.
- What we keep after use is bounded, so idle memory does not grow with use:
  - the layout cache: 512 shaped lines, then only those near the viewport (ADR 0030);
  - the note search cache (twice the folder's text, at most 64 MiB) is dropped when the search is
    left; the next search reads the notes again in the background (~130 ms for 1,000 notes);
  - undo history: 32 MiB of edit text (ADR 0112); saves and recovery snapshots share the rope
    (ADR 0111).
- Rejected: keeping a second Direct3D device alive during startup made GPUI's second device creation
  ~85 ms faster, but left ~7.5 MB more private bytes for the rest of the run (ADR 0110).

## Consequences
- An open note costs about its size again on top (rope plus Markdown regions): ~63 / 89 MB working
  set / private with a 10 MB note.
- Memory figures in docs/performance.md are compared against the empty GPUI window, not only the
  budget, so regressions in our code stay visible.
