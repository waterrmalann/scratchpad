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

// Leaving search frees the note texts it cached, so the next search reads the files again. A
// change that keeps a file's size and time can only be seen that way.
#[gpui::test]
fn a_new_search_reads_the_notes_again(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("coffee");
    assert_eq!(hit_titles(&notes, cx), Some(vec!["Groceries".into()]));
    cx.simulate_keystrokes("escape");

    write_note(
        dir.path(),
        "Groceries",
        "Milk, eggs, cocoa!",
        days_ago(2, 10),
    );
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("cocoa");
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

/// The selected byte range in the editor.
fn editor_selection(
    root: &Entity<AppWindow>,
    cx: &mut VisualTestContext,
) -> std::ops::Range<usize> {
    common::editor(root, cx).read_with(cx, |editor, _| {
        let range = editor.editor().selection().range();
        range.start.0..range.end.0
    })
}

// A hit leads to its first match, as in VS Code's search, not just to the top of the note.
#[gpui::test]
fn choosing_a_hit_selects_its_first_match(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);

    cx.simulate_keystrokes("ctrl-shift-f");
    cx.simulate_input("AUTHENTICATION");
    common::click("hit:Meeting Notes", cx);
    cx.run_until_parked();
    let text = common::editor_text(&root, cx);
    let start = text.find("authentication").unwrap();
    assert_eq!(
        editor_selection(&root, cx),
        start..start + "authentication".len()
    );

    // Enter in the search field does the same, also for a note opened before.
    cx.simulate_keystrokes("ctrl-shift-f");
    cx.simulate_input("eggs");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let text = common::editor_text(&root, cx);
    assert!(text.contains("Milk, eggs"));
    let start = text.find("eggs").unwrap();
    assert_eq!(editor_selection(&root, cx), start..start + 4);
    common::click("hit:Groceries", cx);
    cx.run_until_parked();
    assert_eq!(editor_selection(&root, cx), start..start + 4);
}

// Browsing the hits with the arrow keys shows each one's match too.
#[gpui::test]
fn arrow_keys_through_hits_select_their_matches(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write_notes(dir.path());
    let (root, cx) = common::open_main_window_in(dir.path(), cx);
    let notes = common::notes(&root, cx);

    cx.simulate_keystrokes("ctrl-shift-f");
    cx.simulate_input("at");
    assert_eq!(
        hit_titles(&notes, cx),
        Some(vec!["Auth gateway".into(), "Meeting Notes".into()])
    );
    let first_match = |root: &Entity<AppWindow>, cx: &mut VisualTestContext| {
        let start = common::editor_text(root, cx).find("at").unwrap();
        start..start + 2
    };

    common::click("hit:Auth gateway", cx);
    cx.run_until_parked();
    assert_eq!(editor_selection(&root, cx), first_match(&root, cx));

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert!(common::editor_text(&root, cx).contains("authentication"));
    assert_eq!(editor_selection(&root, cx), first_match(&root, cx));

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert!(common::editor_text(&root, cx).contains("Tokens"));
    assert_eq!(editor_selection(&root, cx), first_match(&root, cx));
}
