# 0020. End-to-end testing strategy
Date: 2026-10-08
Status: Accepted

## Context
App tests should drive the real views as a user would (CONVENTIONS), headlessly and
deterministically, on Windows with crates.io `gpui = 0.2.2`.

## Decision
App e2e tests live in `crates/scratchpad/tests/`, use `gpui` with `test-support` (dev-dep)
and `#[gpui::test]` (sync or `async fn`, taking `&mut TestAppContext`). Start every test with
`common::open_main_window(cx)`, which runs `scratchpad::init` and the real
`open_main_window`, returning the root `Entity<AppWindow>` and a `&mut VisualTestContext`.

Verified working on Windows:
- `cx.simulate_keystrokes("ctrl-w")` (also `secondary-…`): keymap, key contexts, actions.
- `cx.simulate_input("héllo")`: unbound keys reach `EntityInputHandler::replace_text_in_range`
  via the element's `window.handle_input`, so text input tests go through the real path.
  IME composition is tested by calling `replace_and_mark_text_in_range` /
  `replace_text_in_range` inside `entity.update_in(cx, …)`.
- `cx.dispatch_action(A)` to the focused element; `cx.update(|window, cx| …)`,
  `entity.read_with(cx, …)` / `update_in` to read and drive state.
- Layout: taffy runs and custom elements paint. `.debug_selector(|| "name".into())` on a div
  (no-op outside tests) + `cx.debug_bounds("name")` gives its bounds; `simulate_click`,
  `simulate_mouse_*` at those points hit-test correctly; `simulate_resize` works.
- Time: timers only fire on `cx.executor().advance_clock(d)`; `simulate_*` and
  `dispatch_action` already `run_until_parked`. Never sleep.
- Clipboard: in-memory; `cx.write_to_clipboard` / `cx.read_from_clipboard` see app writes.
- Tests run in parallel; the whole suite runs in milliseconds.

## Consequences / limitations
- Text uses `NoopTextSystem`: every family "exists", each BMP char advances 0.6 × font size,
  non-BMP chars (emoji) 1.2 ×, wrapping works. Pixel assertions are deterministic but not
  real-font accurate; real DirectWrite metrics need manual/visual checks.
- No pixels: the scene is never rasterised, so no screenshot tests.
- Test windows report scale factor 2.0 and always Light appearance (to test an OS change, call
  `theme::system_appearance_changed`); `window_title()` only sees `set_window_title`.
- `test-support` pulls extra crates (e.g. `git2`, a C build) into test builds only; the
  shipping binary is unaffected. A cold test build takes ~4 min.
- Do not test GPUI itself; assert on our view state and, once notes exist, files on disk.
