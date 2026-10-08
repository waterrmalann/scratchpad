//! What the app remembers between runs: window bounds, sidebar width and the last open note
//! (PLAN §32).

mod common;

use std::fs;

use common::{click, days_ago, wait, write_note};
use gpui::{Bounds, Entity, TestAppContext, VisualTestContext, point, px, size};
use scratchpad::AppWindow;
use scratchpad::notes::Selection;
use scratchpad::settings::SAVE_DELAY;
use scratchpad_core::{Config, WindowBounds};

fn open_note(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Selection {
    let notes = common::notes(root, cx);
    notes.read_with(cx, |notes, _| notes.selection().clone())
}

#[gpui::test]
fn sidebar_width_and_the_last_note_are_restored(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    write_note(notes_dir.path(), "Newest", "Newest", days_ago(0, 10));
    let older = write_note(notes_dir.path(), "Older", "Older", days_ago(1, 10));
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();

    let (root, cx) = common::open_with(storage.clone(), cx);
    let sidebar = root.read_with(cx, |root, _| root.sidebar().clone());
    sidebar.update(cx, |sidebar, cx| sidebar.set_width(px(333.), cx));
    click("note:Older", cx);
    // Written a moment after the changes stop.
    wait(SAVE_DELAY, cx);
    let saved = Config::load(&config_path);
    assert_eq!(saved.sidebar_width, Some(333.));
    assert_eq!(saved.last_opened_note.as_ref(), Some(&older));
    common::close(cx);

    let (root, cx) = common::open_with(storage, cx);
    let sidebar = root.read_with(cx, |root, _| root.sidebar().clone());
    assert_eq!(
        sidebar.read_with(cx, |sidebar, _| sidebar.width()),
        px(333.)
    );
    assert_eq!(open_note(&root, cx), Selection::Note(older));
}

#[gpui::test]
fn closing_writes_the_settings_without_waiting(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let (root, cx) = common::open_with(storage, cx);
    let sidebar = root.read_with(cx, |root, _| root.sidebar().clone());

    sidebar.update(cx, |sidebar, cx| sidebar.set_width(px(222.), cx));
    cx.simulate_keystrokes("ctrl-w");

    assert_eq!(Config::load(&config_path).sidebar_width, Some(222.));
}

#[gpui::test]
fn window_bounds_are_restored_when_they_fit_a_display(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config_path = storage.config_path.clone().unwrap();
    let write_bounds = |x: f32| {
        let config = Config {
            window: Some(WindowBounds {
                x,
                y: 80.,
                width: 900.,
                height: 600.,
                maximized: false,
            }),
            ..Config::default()
        };
        config.save(&config_path).unwrap();
    };

    write_bounds(100.);
    let (_root, cx) = common::open_with(storage.clone(), cx);
    assert_eq!(
        cx.update(|window, _| window.bounds()),
        Bounds::new(point(px(100.), px(80.)), size(px(900.), px(600.)))
    );
    common::close(cx);

    // The display it was on is gone (the test display is 1920 x 1080).
    write_bounds(5000.);
    let (_root, cx) = common::open_with(storage, cx);
    let bounds = cx.update(|window, _| window.bounds());
    assert!(bounds.origin.x < px(1920.), "{bounds:?}");
    assert_eq!(bounds.size, size(px(1100.), px(720.)));
}

#[gpui::test]
fn a_missing_last_note_falls_back_to_the_newest(cx: &mut TestAppContext) {
    let notes_dir = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let newest = write_note(notes_dir.path(), "Newest", "Newest", days_ago(0, 10));
    let storage = common::storage(notes_dir.path(), data_dir.path());
    let config = Config {
        last_opened_note: Some(notes_dir.path().join("Deleted.md")),
        ..Config::default()
    };
    config.save(storage.config_path.as_ref().unwrap()).unwrap();

    let (root, cx) = common::open_with(storage, cx);

    assert_eq!(open_note(&root, cx), Selection::Note(newest));
    assert_eq!(fs::read_dir(notes_dir.path()).unwrap().count(), 1);
}
