use std::time::SystemTime;

use chrono::{Local, NaiveDate, TimeZone};
use scratchpad_core::{DateGroup, local_date};

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

#[test]
fn local_date_uses_the_local_calendar() {
    // Noon is far enough from midnight that no time zone puts it on another date.
    let noon = date(2026, 8, 29).and_hms_opt(12, 0, 0).unwrap();
    let modified: SystemTime = Local.from_local_datetime(&noon).unwrap().into();
    assert_eq!(local_date(modified), date(2026, 8, 29));
}
