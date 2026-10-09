# 0131. Zooming the note
Date: 2026-10-09
Status: Accepted

## Context
Notepad zooms its text with Ctrl+= / Ctrl+- / Ctrl+0 and Ctrl+wheel in 10% steps and shows the
level in the status bar. Scratchpad's layout is derived from one body size (ADR 0022, 0070), and
the layout cache must not slow typing (PLAN rule 8).

## Decision
- `EditorView` owns the zoom as an integer percent, 50%-400% in steps of 10% (a level that is
  not a multiple of 10, e.g. edited by hand, steps to the next multiple). 10% text is unreadable
  and 500% leaves a few words per row, so the range is narrower than Notepad's 10%-500%.
- The zoom scales the body size and so everything derived from it: heading sizes and spacing,
  line heights, bullets and task boxes, code and quote indents, the hang room of heading markers
  (ADR 0130) and the caret's height. The top padding, hairlines, radii and the caret's 2 px width
  stay as they are. The chrome (sidebar, find bar, dialogs) does not zoom.
- The base style includes the font size, so a zoom drops the layout cache once; typing is
  unaffected.
- The cursor's row stays where it is on screen (then scrolled only if it no longer fits); with the
  cursor off screen the top line stays put. The view remembers where the cursor's row was in the
  last frame.
- Keys: Ctrl+=, Ctrl++ (Windows' name for Ctrl+Shift+= and Ctrl+numpad-plus), Ctrl+- (also
  numpad) and Ctrl+0, as `view::ZoomIn`, `ZoomOut` and `ResetZoom`, handled by the main window so
  they work wherever focus is (and from a menu). Ctrl+wheel over the note zooms a step per
  notch: Windows reports a notch as three lines by default, and a touchpad's fractions of a notch
  add up; at most one step per event, so a wheel set to scroll more lines still steps once.
- `Config::zoom_percent` remembers it; the main window writes it when the editor's zoom differs.

## Consequences
- A wheel set to scroll one line per notch needs three notches per step.
- Code blocks and inline code keep their proportions; very large zooms show few words per row.
