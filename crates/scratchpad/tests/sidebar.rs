mod common;

use common::{EventLog, click, days_ago, write_note};
use gpui::{TestAppContext, VisualTestContext};
use scratchpad::notes::{NotesEvent, Selection};

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

#[gpui::test]
fn window_renders_before_the_notes_are_listed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Ideas", "", days_ago(0, 9));
    cx.update(scratchpad::init);
    let location = common::location(dir.path());
    let window = cx.update(|cx| scratchpad::open_main_window(location, cx).unwrap());
    let root = window.root(cx).unwrap();
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    let notes = common::notes(&root, cx);

    assert!(cx.debug_bounds("sidebar").is_some(), "first frame drawn");
    assert!(!notes.read_with(cx, |notes, _| notes.is_loaded()));

    cx.run_until_parked();
    let titles = notes.read_with(cx, |notes, _| {
        notes
            .notes()
            .iter()
            .map(|n| n.title.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(titles, ["Ideas"]);
    common::center_of("note:Ideas", cx);
}

#[gpui::test]
fn notes_are_grouped_by_date_and_newest_first(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let notes = [
        ("Old", days_ago(100, 12)),
        ("Todo", days_ago(0, 9)),
        ("Plan", days_ago(20, 12)),
        ("Random", days_ago(3, 12)),
        ("Meeting", days_ago(0, 10)),
        ("Ideas", days_ago(1, 12)),
    ];
    for (title, modified) in notes {
        write_note(dir.path(), title, "", modified);
    }
    let (_root, cx) = common::open_main_window_in(dir.path(), cx);

    assert_rendered_in_order(
        &[
            "group:Today",
            "note:Meeting",
            "note:Todo",
            "group:Yesterday",
            "note:Ideas",
            "group:Previous 7 Days",
            "note:Random",
            "group:Previous 30 Days",
            "note:Plan",
            "group:Older",
            "note:Old",
        ],
        cx,
    );
}

#[gpui::test]
fn clicking_a_note_opens_it_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Meeting", "", days_ago(0, 10));
    let ideas = write_note(dir.path(), "Ideas", "", days_ago(1, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    click("note:Ideas", cx);
    assert_eq!(events.take(), [NotesEvent::OpenNote(ideas.clone())]);
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(ideas)
    );

    // Clicking the open note again must not reload it (and lose the editor's state).
    click("note:Ideas", cx);
    assert_eq!(events.take(), []);
}

#[gpui::test]
fn new_note_is_kept_in_memory_until_it_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_note(dir.path(), "Meeting", "", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    cx.simulate_keystrokes("ctrl-n");
    let draft = events.take_draft();
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Draft(draft)
    );
    assert_rendered_in_order(&["group:Today", "note:draft", "note:Meeting"], cx);
    assert_eq!(common::titles_on_disk(dir.path()), ["Meeting"]);

    // The editor integration saves the draft once it has content.
    let saved = notes
        .update(cx, |notes, cx| {
            notes.save_draft(draft, Some("Webhook ideas"), cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(saved.path, dir.path().join("Webhook ideas.md"));
    assert_eq!(
        common::titles_on_disk(dir.path()),
        ["Meeting", "Webhook ideas"]
    );
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Note(saved.path)
    );
    assert_eq!(events.take(), [], "the saved draft is already open");
    assert_rendered_in_order(&["note:Webhook ideas", "note:Meeting"], cx);
}

#[gpui::test]
fn leaving_an_empty_new_note_discards_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let meeting = write_note(dir.path(), "Meeting", "", days_ago(0, 10));
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    click("new-note-button", cx);
    events.take_draft();
    click("note:Meeting", cx);

    assert_eq!(events.take(), [NotesEvent::OpenNote(meeting)]);
    assert!(!notes.read_with(cx, |notes, _| notes.has_draft()));
    assert_eq!(common::titles_on_disk(dir.path()), ["Meeting"]);
}

#[gpui::test]
fn saving_a_draft_the_user_has_left_keeps_the_newer_draft_open(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    cx.simulate_keystrokes("ctrl-n");
    let first = events.take_draft();
    // Ctrl+N again before the first draft's text was saved...
    cx.simulate_keystrokes("ctrl-n");
    let second = events.take_draft();
    // ...so the editor saves the first draft while switching to the second.
    let saved = notes
        .update(cx, |notes, cx| notes.save_draft(first, Some("First"), cx))
        .unwrap();
    cx.run_until_parked();

    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Draft(second)
    );
    assert_eq!(common::titles_on_disk(dir.path()), ["First"]);
    assert_rendered_in_order(&["note:draft", "note:First"], cx);
    assert_eq!(saved.path, dir.path().join("First.md"));
}
