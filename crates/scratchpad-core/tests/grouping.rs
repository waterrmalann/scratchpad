use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{Duration, Local, NaiveDate, TimeZone};
use scratchpad_core::{DateGroup, Note, group_notes, local_date};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn groups_follow_calendar_day_distance() {
    let today = date(2026, 10, 8);
    let cases = [
        (date(2026, 10, 8), DateGroup::Today),
        (date(2026, 10, 9), DateGroup::Today),
        (date(2027, 1, 1), DateGroup::Today),
        (date(2026, 10, 7), DateGroup::Yesterday),
        (date(2026, 10, 6), DateGroup::Previous7Days),
        (date(2026, 10, 1), DateGroup::Previous7Days),
        (date(2026, 9, 30), DateGroup::Previous30Days),
        (date(2026, 9, 8), DateGroup::Previous30Days),
        (date(2026, 9, 7), DateGroup::Older),
        (date(2020, 1, 1), DateGroup::Older),
    ];
    for (modified, expected) in cases {
        assert_eq!(DateGroup::of(modified, today), expected, "{modified}");
    }
}

#[test]
fn groups_cross_month_and_year_boundaries() {
    assert_eq!(
        DateGroup::of(date(2025, 12, 31), date(2026, 1, 1)),
        DateGroup::Yesterday
    );
    assert_eq!(
        DateGroup::of(date(2025, 12, 25), date(2026, 1, 1)),
        DateGroup::Previous7Days
    );
}

/// A note modified at local noon, `days_ago` days before `today`.
fn note_modified(title: &str, today: NaiveDate, days_ago: i64) -> Note {
    let noon = (today - Duration::days(days_ago))
        .and_hms_opt(12, 0, 0)
        .unwrap();
    let modified_at: SystemTime = Local.from_local_datetime(&noon).unwrap().into();
    Note {
        path: PathBuf::from(format!("{title}.md")),
        title: title.to_owned(),
        modified_at,
        created_at: None,
        len: 0,
    }
}

#[test]
fn local_date_uses_the_local_calendar() {
    let today = date(2026, 10, 8);
    assert_eq!(local_date(note_modified("a", today, 0).modified_at), today);
    assert_eq!(
        local_date(note_modified("a", today, 40).modified_at),
        date(2026, 8, 29)
    );
}

#[test]
fn group_notes_yields_sections_in_order_with_their_notes() {
    let today = date(2026, 10, 8);
    let notes = vec![
        note_modified("a", today, 0),
        note_modified("b", today, 0),
        note_modified("c", today, 1),
        note_modified("d", today, 3),
        note_modified("e", today, 20),
        note_modified("f", today, 400),
    ];

    let grouped: Vec<(DateGroup, Vec<&str>)> = group_notes(&notes, today)
        .into_iter()
        .map(|(group, run)| (group, run.iter().map(|n| n.title.as_str()).collect()))
        .collect();

    assert_eq!(
        grouped,
        [
            (DateGroup::Today, vec!["a", "b"]),
            (DateGroup::Yesterday, vec!["c"]),
            (DateGroup::Previous7Days, vec!["d"]),
            (DateGroup::Previous30Days, vec!["e"]),
            (DateGroup::Older, vec!["f"]),
        ]
    );
    assert!(group_notes(&[], today).is_empty());
}
