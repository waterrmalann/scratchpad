//! In-document search (PLAN §29).

use std::ops::Range;

use crate::coords::ByteOffset;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaseSensitivity {
    #[default]
    Insensitive,
    Sensitive,
}

/// All non-overlapping matches of `query` in `text`, left to right. An empty query matches nothing.
///
/// Case-insensitive matching compares the per-char lowercase mappings of both strings. Lowercasing can change
/// lengths (`İ` becomes `i̇`, `K` (Kelvin) becomes `k`), so matching runs on the original text: a match must
/// start and end on char boundaries of `text`, and a query ending halfway through a char's lowercase
/// expansion does not match. Returned ranges are therefore always valid ranges of `text`.
pub fn find_all(text: &str, query: &str, case: CaseSensitivity) -> Vec<Range<ByteOffset>> {
    if query.is_empty() {
        return Vec::new();
    }
    match case {
        CaseSensitivity::Sensitive => text
            .match_indices(query)
            .map(|(start, m)| ByteOffset(start)..ByteOffset(start + m.len()))
            .collect(),
        CaseSensitivity::Insensitive => find_all_insensitive(text, query),
    }
}

fn find_all_insensitive(text: &str, query: &str) -> Vec<Range<ByteOffset>> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let first = needle[0];
    let mut matches = Vec::new();
    let mut resume_at = 0;
    for (start, c) in text.char_indices() {
        // Cheap first-char filter; most positions fail here.
        let could_start = if c.is_ascii() {
            c.to_ascii_lowercase() == first
        } else {
            c.to_lowercase().next() == Some(first)
        };
        if !could_start || start < resume_at {
            continue;
        }
        if let Some(len) = match_len_at(&text[start..], &needle) {
            matches.push(ByteOffset(start)..ByteOffset(start + len));
            resume_at = start + len;
        }
    }
    matches
}

/// Byte length of the prefix of `haystack` whose lowercase mapping equals `needle`, if any.
fn match_len_at(haystack: &str, needle: &[char]) -> Option<usize> {
    let mut matched = 0;
    for (offset, c) in haystack.char_indices() {
        for folded in c.to_lowercase() {
            if needle.get(matched) != Some(&folded) {
                return None;
            }
            matched += 1;
        }
        if matched == needle.len() {
            return Some(offset + c.len_utf8());
        }
    }
    None
}

/// Index of the first match starting at or after `from`, wrapping around to the first match. Pass the end of
/// the current match (or the cursor) to step forward.
pub fn next_match(matches: &[Range<ByteOffset>], from: ByteOffset) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|m| m.start < from);
    Some(if index == matches.len() { 0 } else { index })
}

/// Index of the last match starting before `before`, wrapping around to the last match. Pass the start of
/// the current match (or the cursor) to step backward.
pub fn prev_match(matches: &[Range<ByteOffset>], before: ByteOffset) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|m| m.start < before);
    Some(index.checked_sub(1).unwrap_or(matches.len() - 1))
}
