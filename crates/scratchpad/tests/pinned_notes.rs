//! Pinning notes to the top of the sidebar.

mod common;

use common::{click, days_ago, right_click, wait, write_note};
use gpui::{TestAppContext, VisualTestContext};
use scratchpad::notes::Selection;
use scratchpad::settings::SAVE_DELAY;
use scratchpad_core::Config;

/// Asserts the elements tagged with these selectors are rendered top to bottom in this order.
fn assert_rendered_in_order(selectors: &[&str], cx: &mut VisualTestContext) {
    let tops: Vec<_> = selectors
        .iter()
        .map(|selector| common::center_of(selector, cx).y)
        .collect();
    for (pair, top) in selectors.windows(2).zip(tops.windows(2)) {
        assert!(top[0] < top[1], "{} is not above {}", pair[0], pair[1]);
    }
}

fn write_three(dir: &std::path::Path) {
    write_note(dir, "Old", "", days_ago(100, 12));
    write_note(dir, "Todo", "", days_ago(0, 9));
    write_note(dir, "Ideas", "", days_ago(1, 12));
}

#[gpui::test]
fn a_pinned_note_moves_to_a_section_above_the_dates(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_three(dir.path());
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    right_click("note:Old", cx);
    click("menu:Pin", cx);

    assert_rendered_in_order(
        &[
            "group:Pinned",
            "note:Old",
            "group:Today",
            "note:Todo",
            "group:Yesterday",
            "note:Ideas",
        ],
        cx,
    );
}

#[gpui::test]
fn unpinning_returns_the_note_to_its_date(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_three(dir.path());
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    right_click("note:Old", cx);
    click("menu:Pin", cx);
    right_click("note:Old", cx);
    click("menu:Unpin", cx);

    assert_rendered_in_order(&["group:Today", "note:Todo", "group:Older", "note:Old"], cx);
}

#[gpui::test]
fn pins_are_remembered_across_runs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_three(dir.path());
    let data = tempfile::tempdir().unwrap();
    let storage = common::storage(dir.path(), data.path());
    let config_path = storage.config_path.clone().unwrap();
    let (_root, cx) = common::open_with(storage.clone(), cx);

    right_click("note:Old", cx);
    click("menu:Pin", cx);
    wait(SAVE_DELAY, cx);
    assert_eq!(
        Config::load(&config_path).pinned_notes,
        [dir.path().join("Old.md")]
    );
    common::close(cx);

    let (_root, cx) = common::open_with(storage, cx);
    assert_rendered_in_order(
        &["group:Pinned", "note:Old", "group:Today", "note:Todo"],
        cx,
    );
}

#[gpui::test]
fn renaming_a_pinned_note_keeps_it_pinned_and_deleting_it_unpins(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_three(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let old = dir.path().join("Old.md");

    right_click("note:Old", cx);
    click("menu:Pin", cx);
    notes
        .update(cx, |notes, cx| notes.rename(&old, "Ancient", cx))
        .unwrap();
    cx.run_until_parked();
    let renamed = dir.path().join("Ancient.md");
    assert_rendered_in_order(&["group:Pinned", "note:Ancient", "group:Today"], cx);
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.pinned().to_vec()),
        std::slice::from_ref(&renamed)
    );

    notes
        .update(cx, |notes, cx| notes.delete(&renamed, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(notes.read_with(cx, |notes, _| notes.pinned().is_empty()));
}

#[gpui::test]
fn the_arrow_keys_follow_the_pinned_section_first(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_three(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    right_click("note:Old", cx);
    click("menu:Pin", cx);
    click("note:Old", cx);
    cx.simulate_keystrokes("down");

    // The list shows Old, Todo, Ideas.
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(dir.path().join("Todo.md"))
    );
}
