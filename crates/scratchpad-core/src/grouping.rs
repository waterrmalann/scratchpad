//! Sidebar sections by modification date (PLAN §8).

use std::time::{SystemTime, UNIX_EPOCH};

pub use chrono::NaiveDate;
use chrono::{DateTime, Local};

use crate::note::Note;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateGroup {
    Today,
    Yesterday,
    Previous7Days,
    Previous30Days,
    Older,
}

impl DateGroup {
    /// The section a note modified on `modified` belongs to when today is `today`. Both are
    /// local calendar dates, so a note edited at 23:59 moves to "Yesterday" a minute later.
    /// Dates in the future (clock changes, synced files) count as today.
    pub fn of(modified: NaiveDate, today: NaiveDate) -> Self {
        match today.signed_duration_since(modified).num_days() {
            ..=0 => DateGroup::Today,
            1 => DateGroup::Yesterday,
            2..=7 => DateGroup::Previous7Days,
            8..=30 => DateGroup::Previous30Days,
            _ => DateGroup::Older,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DateGroup::Today => "Today",
            DateGroup::Yesterday => "Yesterday",
            DateGroup::Previous7Days => "Previous 7 Days",
            DateGroup::Previous30Days => "Previous 30 Days",
            DateGroup::Older => "Older",
        }
    }
}

/// The local calendar date of `time`, using the system time zone.
pub fn local_date(time: SystemTime) -> NaiveDate {
    let (seconds, negate) = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => (after.as_secs(), false),
        Err(before) => (before.duration().as_secs(), true),
    };
    // Timestamps outside chrono's range (corrupt file metadata) fall back to the epoch.
    let utc = i64::try_from(seconds)
        .ok()
        .and_then(|s| DateTime::from_timestamp(if negate { -s } else { s }, 0))
        .unwrap_or(DateTime::UNIX_EPOCH);
    utc.with_timezone(&Local).date_naive()
}

/// Splits `notes` into consecutive runs that share a section, in the given order. Pass the
/// output of [`NoteStore::list`](crate::NoteStore::list), which is sorted by modification time
/// and therefore yields each section at most once.
pub fn group_notes(notes: &[Note], today: NaiveDate) -> Vec<(DateGroup, &[Note])> {
    let group_of = |note: &Note| DateGroup::of(local_date(note.modified_at), today);
    notes
        .chunk_by(|a, b| group_of(a) == group_of(b))
        .map(|run| (group_of(&run[0]), run))
        .collect()
}
