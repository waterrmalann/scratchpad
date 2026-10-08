mod common;

use std::fs;

use common::{at, set_modified, temp_store};
use scratchpad_core::{Note, NoteSearch, NoteStore, SearchHit};

/// Creates a note titled `title` with `text`, last modified at `modified` seconds.
fn add(store: &NoteStore, title: &str, text: &str, modified: u64) {
    let note = store.create(Some(title)).unwrap();
    store.save(&note.path, text).unwrap();
    set_modified(&note.path, at(modified));
}

fn search(store: &NoteStore, query: &str) -> Vec<SearchHit> {
    NoteSearch::new().search(store, &store.list().unwrap(), query)
}

fn titles(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.title.as_str()).collect()
}

fn highlighted<'a>(text: &'a str, ranges: &[std::ops::Range<usize>]) -> Vec<&'a str> {
    ranges.iter().map(|r| &text[r.clone()]).collect()
}

#[test]
fn empty_queries_find_nothing() {
    let (_dir, store) = temp_store();
    add(&store, "Anything", "some text", 1);
    for query in ["", "   ", "\n\t"] {
        assert!(search(&store, query).is_empty(), "{query:?}");
    }
}

#[test]
fn title_matches_rank_before_newer_content_matches() {
    let (_dir, store) = temp_store();
    add(&store, "Auth design", "nothing relevant", 100);
    add(&store, "Groceries", "buy AUTH tokens?", 400);
    add(&store, "Old auth notes", "x", 50);
    add(&store, "Standup", "discussed authentication", 300);
    add(&store, "Unrelated", "nope", 500);

    let hits = search(&store, "auth");

    // Title matches by recency, then content matches by recency.
    assert_eq!(
        titles(&hits),
        ["Auth design", "Old auth notes", "Groceries", "Standup"]
    );
}

#[test]
fn matching_ignores_case_and_normalizes_whitespace() {
    let (_dir, store) = temp_store();
    add(&store, "Notes", "Über die HTTP Gateway\nsecond line", 1);

    assert_eq!(titles(&search(&store, "ÜBER")), ["Notes"]);
    assert_eq!(titles(&search(&store, "  http   gateway ")), ["Notes"]);
    assert!(search(&store, "gateway second").is_empty());
}

#[test]
fn content_hits_carry_a_highlighted_snippet() {
    let (_dir, store) = temp_store();
    add(
        &store,
        "Meeting Notes",
        "Agenda\nWe agreed on the Authentication changes for v2.\nAOB",
        1,
    );

    let hits = search(&store, "authentication");
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.title, "Meeting Notes");
    assert!(hit.title_ranges.is_empty());
    assert_eq!(
        hit.snippet,
        "We agreed on the Authentication changes for v2."
    );
    assert_eq!(
        highlighted(&hit.snippet, &hit.snippet_ranges),
        ["Authentication"]
    );
}

#[test]
fn snippets_highlight_every_match_in_the_excerpt() {
    let (_dir, store) = temp_store();
    add(&store, "Doc", "foo bar Foo baz FOO", 1);

    let hit = &search(&store, "foo")[0];
    assert_eq!(hit.snippet, "foo bar Foo baz FOO");
    assert_eq!(
        highlighted(&hit.snippet, &hit.snippet_ranges),
        ["foo", "Foo", "FOO"]
    );
}

#[test]
fn long_lines_are_cut_around_the_match_with_ellipses() {
    let (_dir, store) = temp_store();
    let line = format!("{}needle{}", "a".repeat(200), "b".repeat(200));
    add(&store, "Long", &format!("{line}\nsecond line"), 1);

    let hit = &search(&store, "needle")[0];

    assert_eq!(
        hit.snippet,
        format!("…{}needle{}…", "a".repeat(30), "b".repeat(70))
    );
    assert_eq!(highlighted(&hit.snippet, &hit.snippet_ranges), ["needle"]);
}

#[test]
fn snippets_stay_on_one_line_and_skip_indentation() {
    let (_dir, store) = temp_store();
    add(
        &store,
        "Crlf",
        "first\r\n    \tindented needle here\r\nlast",
        1,
    );

    let hit = &search(&store, "needle")[0];
    assert_eq!(hit.snippet, "indented needle here");
    assert_eq!(highlighted(&hit.snippet, &hit.snippet_ranges), ["needle"]);
}

#[test]
fn title_only_matches_have_title_ranges_and_no_snippet() {
    let (_dir, store) = temp_store();
    add(&store, "Roadmap 2026", "unrelated body", 1);

    let hit = &search(&store, "map")[0];
    assert_eq!(highlighted(&hit.title, &hit.title_ranges), ["map"]);
    assert_eq!(hit.snippet, "");
    assert!(hit.snippet_ranges.is_empty());
}

#[test]
fn ranges_stay_valid_where_lowercasing_changes_byte_lengths() {
    let (_dir, store) = temp_store();
    // `İ` lower-cases to two chars and `ẞ` to a shorter one, which would shift every offset
    // after them if matching used `str::to_lowercase`.
    add(&store, "Tricky", "İİİ ẞẞ Straße needle ÉCOLE", 1);

    let hit = &search(&store, "needle")[0];
    assert_eq!(highlighted(&hit.snippet, &hit.snippet_ranges), ["needle"]);
    let hit = &search(&store, "école")[0];
    assert_eq!(highlighted(&hit.snippet, &hit.snippet_ranges), ["ÉCOLE"]);
    let hit = &search(&store, "ẞẞ")[0];
    assert_eq!(highlighted(&hit.snippet, &hit.snippet_ranges), ["ẞẞ"]);
}

#[test]
fn an_unreadable_note_is_still_found_by_title() {
    let (_dir, store) = temp_store();
    add(&store, "Vanishing plan", "text", 1);
    let notes = store.list().unwrap();
    fs::remove_file(&notes[0].path).unwrap();

    let hits = NoteSearch::new().search(&store, &notes, "plan");
    assert_eq!(titles(&hits), ["Vanishing plan"]);
    assert_eq!(hits[0].snippet, "");
}

#[test]
fn notes_with_invalid_utf8_are_searchable() {
    let (dir, store) = temp_store();
    fs::write(
        dir.path().join("Binary.md"),
        b"before \xFF\xFE needle after",
    )
    .unwrap();

    let hit = &search(&store, "needle")[0];
    assert_eq!(hit.snippet, "before \u{FFFD}\u{FFFD} needle after");
}

fn note_titled(store: &NoteStore, title: &str) -> Note {
    store
        .list()
        .unwrap()
        .into_iter()
        .find(|n| n.title == title)
        .unwrap()
}

#[test]
fn edits_are_picked_up_when_size_or_modification_time_changes() {
    let (_dir, store) = temp_store();
    add(&store, "Live", "alpha", 100);
    let mut searcher = NoteSearch::new();
    let notes = store.list().unwrap();
    assert_eq!(searcher.search(&store, &notes, "alpha").len(), 1);

    store.save(&notes[0].path, "alpha and omega").unwrap();
    let notes = store.list().unwrap();
    assert_eq!(searcher.search(&store, &notes, "omega").len(), 1);

    // Same size as before, different modification time.
    store.save(&notes[0].path, "omega and alpha").unwrap();
    set_modified(&notes[0].path, at(200));
    let notes = store.list().unwrap();
    let hit = &searcher.search(&store, &notes, "omega")[0];
    assert_eq!(hit.snippet, "omega and alpha");
}

#[test]
fn unchanged_notes_are_not_read_again() {
    let (_dir, store) = temp_store();
    add(&store, "Cached", "alpha", 100);
    let mut searcher = NoteSearch::new();
    let notes = store.list().unwrap();
    assert_eq!(searcher.search(&store, &notes, "alpha").len(), 1);

    // Swap the content behind the searcher's back without changing size or modification time:
    // the cache key says "unchanged", so the old text is still served.
    fs::write(&notes[0].path, "omega").unwrap();
    set_modified(&notes[0].path, at(100));
    let notes = vec![note_titled(&store, "Cached")];
    assert_eq!(searcher.search(&store, &notes, "alpha").len(), 1);
    assert!(searcher.search(&store, &notes, "omega").is_empty());
}

#[test]
fn deleted_notes_stop_matching() {
    let (_dir, store) = temp_store();
    add(&store, "Keep", "needle", 1);
    add(&store, "Drop", "needle", 2);
    let mut searcher = NoteSearch::new();
    assert_eq!(
        searcher
            .search(&store, &store.list().unwrap(), "needle")
            .len(),
        2
    );

    store.delete(&note_titled(&store, "Drop").path).unwrap();

    let hits = searcher.search(&store, &store.list().unwrap(), "needle");
    assert_eq!(titles(&hits), ["Keep"]);
}
