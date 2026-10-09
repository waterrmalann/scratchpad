# 0070. Markdown typography in the editor view
Date: 2026-10-08
Status: Accepted; the margin headings hang in amended by ADR 0130

## Context
"Markdown looks excellent" is a V1 criterion, with restraint: one coherent document, not a code editor
(PLAN §18, §27, §48). The engine gives each line a `StyledLine` (ADR 0042); the view must turn it into
fonts, colours and shapes using the seven theme tokens (ADR 0022).

## Decision
- Per line: H1–H3 at 1.8 / 1.5 / 1.25 × body, H4–H6 at 1.1 / 1 / 1 (H6 muted), all semibold, line height
  1.3 × size, and 15 / 12 / 9 / 6 px of space above. Code blocks use the monospace family at 0.88 ×
  body, keep the body line height and get one rounded `surface` background per run of code lines,
  with the text indented 14 px; hidden fence lines become its top and bottom padding.
- Per span: bold 700, italic, inline code in monospace at 0.88 × the line size on a rounded `surface`
  background, strikethrough and quotes in `muted`, revealed markers and list numbers in `muted`, the
  text of checked tasks in `muted`. Links keep the text colour with an `accent` underline: the amber
  accent is too light for text on white (2.4:1). No new theme tokens were needed.
- Shapes instead of characters: bullets as a dot, task boxes as a rounded box (checked: `accent`
  fill and a check mark), a hidden `---` as a 1 px `border` rule, quotes as a 3 px bar with the text
  18 px in. Lines through and under text stop before the space a row wraps at.
- Heading `#`s hang in the margin, so revealing them does not move the heading's text, as far as
  the margin allows: where it is narrower (32 px in narrow windows), the text moves by the rest
  rather than the markers being cut off. Wrapped rows
  of a list item line up with its text, and a nested item's leading spaces are widened so two spaces
  line up with the parent's text: spaces are narrow in a proportional font.
- A line is shaped in segments of one font. GPUI 0.2.2's `layout_line` gives a run the previous
  run's font when only the font changes (not the colour), which loses bold and italic.

## Consequences
- Lines mixing styles cost a few shaping calls instead of one; only changed lines are shaped.
- In narrow windows, revealing `###`+ markers moves the heading's text a little.
- No syntax highlighting in code blocks yet (PLAN §48 allows postponing it).
