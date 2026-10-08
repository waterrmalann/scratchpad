# 0052. A minimal single-line text input
Date: 2026-10-08
Status: Accepted

## Context
GPUI 0.2.2 has no text field. The note search and inline rename need one with IME support;
pulling in a component library would contradict PLAN §42 and §62. The editor view is a
separate, much richer component built on `scratchpad-editor`.

## Decision
- `text_input::TextInput` follows gpui's `examples/input.rs`: an entity implementing
  `EntityInputHandler` plus a custom element that shapes one line, paints selection and caret
  from theme tokens, and registers the input handler.
- Offsets are UTF-8 byte offsets on grapheme boundaries (`unicode-segmentation`); the IME
  interface converts to and from UTF-16 at the boundary.
- Supported: typing, Backspace/Delete by grapheme, Ctrl+Backspace by word, Left/Right/Home/
  End (Up/Down act as Home/End) with Shift selection, Ctrl+A, clipboard (newlines pasted as
  spaces), mouse placement and drag selection, horizontal scrolling to keep the caret visible.
- Enter and Escape are emitted as `TextInputEvent::{Confirmed, Cancelled}`; text changes as
  `Changed`. The owner decides what they mean. Actions and bindings live in the module and
  are scoped to the `TextInput` key context, so they win over list bindings around it.

## Consequences
- No undo, word-wise caret movement, double-click word selection or caret blinking.
- The editor integration should not reuse this for note text.
