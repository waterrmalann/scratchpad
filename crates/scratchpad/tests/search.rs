mod common;

use std::path::Path;

use common::{EventLog, days_ago, write_note};
use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use scratchpad::AppWindow;
use scratchpad::notes::{Notes, NotesEvent, Selection};

/// Title, highlighted title parts, snippet and highlighted snippet parts of each hit.
type HitView = (String, Vec<String>, String, Vec<String>);

fn hits(notes: &Entity<Notes>, cx: &mut VisualTestContext) -> Option<Vec<HitView>> {
    notes.read_with(cx, |notes, _| {
        notes.search_hits().map(|hits| {
            hits.iter()
                .map(|hit| {
                    let parts = |text: &str, ranges: &[std::ops::Range<usize>]| {
                        ranges.iter().map(|r| text[r.clone()].to_owned()).collect()
                    };
                    (
                        hit.title.clone(),
                        parts(&hit.title, &hit.title_ranges),
                        hit.snippet.clone(),
                        parts(&hit.snippet, &hit.snippet_ranges),
                    )
                })
                .collect()
        })
    })
}

fn hit_titles(notes: &Entity<Notes>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    hits(notes, cx).map(|hits| hits.into_iter().map(|hit| hit.0).collect())
}

fn search_text(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> String {
    root.read_with(cx, |root, cx| root.sidebar().read(cx).search_text(cx))
}

fn write_notes(dir: &Path) {
    write_note(
        dir,
        "Meeting Notes",
        "# Project\n\nWe need to finish the authentication work this week.",
        days_ago(0, 10),
    );
    write_note(dir, "Auth gateway", "Tokens rotate daily.", days_ago(1, 10));
    write_note(dir, "Groceries", "Milk, eggs, coffee", days_ago(2, 10));
}

#[gpui::test]
fn search_matches_titles_and_contents_and_escape_restores_the_list(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let editor_focus = root.read_with(cx, |root, cx| root.editor_pane().focus_handle(cx));
    assert!(cx.update(|window, _| editor_focus.is_focused(window)));

    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("AUTH");

    assert_eq!(
        hits(&notes, cx).unwrap(),
        [
            (
                "Auth gateway".into(),
                vec!["Auth".into()],
                String::new(),
                vec![]
            ),
            (
                "Meeting Notes".into(),
                vec![],
                "We need to finish the authentication work this week.".into(),
                vec!["auth".into()],
            ),
        ]
    );
    // Title matches first, each hit with its snippet line.
    let gateway = common::center_of("hit:Auth gateway", cx);
    let meeting = common::center_of("hit:Meeting Notes", cx);
    assert!(gateway.y < meeting.y);

    cx.simulate_keystrokes("escape");
    assert_eq!(search_text(&root, cx), "");
    assert_eq!(hit_titles(&notes, cx), None);
    assert!(!notes.read_with(cx, |notes, _| notes.is_searching()));
    // Focus goes back to where it was before Ctrl+P.
    assert!(cx.update(|window, _| editor_focus.is_focused(window)));
}

#[gpui::test]
fn search_without_matches_shows_no_hits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    cx.simulate_keystrokes("ctrl-shift-f");
    cx.simulate_input("zebra");

    assert_eq!(hit_titles(&notes, cx), Some(vec![]));
    common::center_of("no-results", cx);
}

// Each keystroke starts a new background search. The test executor runs the pending searches
// in a different random order for every seed, so stale results would win on some seeds.
#[gpui::test(iterations = 20)]
fn only_the_latest_query_shows_results(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    cx.simulate_keystrokes("ctrl-p");
    // "e" matches every note, "egg" only the content of one.
    cx.simulate_input("egg");

    assert_eq!(hit_titles(&notes, cx), Some(vec!["Groceries".into()]));
}

#[gpui::test]
fn enter_opens_the_first_hit_and_a_new_note_leaves_search(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);
    let events = EventLog::new(&notes, cx);

    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("coffee");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        events.take(),
        [NotesEvent::OpenNote(dir.path().join("Groceries.md"))]
    );

    cx.simulate_keystrokes("ctrl-n");
    let draft = events.take_draft();
    assert_eq!(search_text(&root, cx), "");
    assert_eq!(
        notes.read_with(cx, |notes, _| notes.selection().clone()),
        Selection::Draft(draft)
    );
    common::center_of("note:draft", cx);
}
