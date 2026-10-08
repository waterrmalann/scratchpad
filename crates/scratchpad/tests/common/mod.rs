//! Shared setup for end-to-end tests. See docs/adrs/0020-e2e-testing-strategy.md.

use gpui::{Entity, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;

/// Initialises the app exactly as `main` does and opens the real main window in GPUI's
/// headless test platform.
///
/// Returns the root view and a window-bound context for simulating input
/// (`simulate_keystrokes`, `simulate_input`, `dispatch_action`, ...). The context lives until
/// the end of the test.
pub fn open_main_window(cx: &mut TestAppContext) -> (Entity<AppWindow>, &mut VisualTestContext) {
    cx.update(scratchpad::init);
    let window = cx.update(|cx| scratchpad::open_main_window(cx).expect("open main window"));
    let root = window.root(cx).expect("main window root view");
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.run_until_parked();
    (root, cx)
}
