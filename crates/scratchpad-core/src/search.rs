//! Note search over titles and contents (PLAN §28). See ADR 0012.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::note::Note;
use crate::store::NoteStore;

/// Characters of context kept before and after the first content match in a snippet.
const CONTEXT_BEFORE: usize = 30;
const CONTEXT_AFTER: usize = 70;
const ELLIPSIS: &str = "…";
/// Upper bound for the text (original plus lower-cased copy) kept in memory between searches.
/// Notes that do not fit are re-read on every search instead of being cached.
const CACHE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub title: String,
    /// Byte ranges of `title` that match the query, on char boundaries.
    pub title_ranges: Vec<Range<usize>>,
    /// A single-line excerpt of the note around the first match in its content, with `…` where
    /// it was cut. Empty when only the title matches.
    pub snippet: String,
    /// Byte ranges of `snippet` that match the query, on char boundaries.
    pub snippet_ranges: Vec<Range<usize>>,
}

/// Searches note titles and contents.
///
/// Keeps the text of notes it has read, keyed by path, modification time and size, so typing
/// another character does not re-read unchanged files. Reading happens on the first search, so
/// the app should call [`search`](Self::search) off the UI thread.
#[derive(Debug, Default)]
pub struct NoteSearch {
    cache: HashMap<PathBuf, Arc<CachedText>>,
    cached_bytes: usize,
}

#[derive(Debug)]
struct CachedText {
    modified_at: SystemTime,
    len: u64,
    text: String,
    /// `fold(text)`: same byte offsets as `text`, so matches map back without conversion.
    folded: String,
}

impl NoteSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Finds `query` (case-insensitive, whitespace-normalized, matched as one phrase) in the
    /// titles and contents of `notes`, reading contents through `store`.
    ///
    /// Notes whose title matches come first, then notes matching only in their content; each
    /// group is ordered by modification time, newest first. An empty query yields no hits, the
    /// app shows the normal note list instead. Unreadable notes are still searched by title.
    pub fn search(&mut self, store: &NoteStore, notes: &[Note], query: &str) -> Vec<SearchHit> {
        let query = fold(&query.split_whitespace().collect::<Vec<_>>().join(" "));
        if query.is_empty() {
            return Vec::new();
        }
        self.forget_notes_not_in(notes);

        let mut newest_first: Vec<&Note> = notes.iter().collect();
        newest_first.sort_by_key(|note| std::cmp::Reverse(note.modified_at));

        let mut title_hits = Vec::new();
        let mut content_hits = Vec::new();
        for note in newest_first {
            let title_ranges = find_all(&fold(&note.title), &query);
            let content = self.content_of(store, note);
            let first_match = content
                .as_ref()
                .and_then(|c| c.folded.find(&query).map(|start| (c, start)));
            let (snippet, snippet_ranges) = match first_match {
                Some((c, start)) => snippet_around(&c.text, start..start + query.len(), &query),
                None => (String::new(), Vec::new()),
            };
            if title_ranges.is_empty() && snippet.is_empty() {
                continue;
            }
            let hit = SearchHit {
                path: note.path.clone(),
                title: note.title.clone(),
                snippet,
                snippet_ranges,
                title_ranges,
            };
            if hit.title_ranges.is_empty() {
                content_hits.push(hit);
            } else {
                title_hits.push(hit);
            }
        }
        title_hits.extend(content_hits);
        title_hits
    }

    fn forget_notes_not_in(&mut self, notes: &[Note]) {
        let current: HashSet<&PathBuf> = notes.iter().map(|note| &note.path).collect();
        self.cache.retain(|path, _| current.contains(path));
        self.cached_bytes = self.cache.values().map(|c| c.footprint()).sum();
    }

    fn content_of(&mut self, store: &NoteStore, note: &Note) -> Option<Arc<CachedText>> {
        if let Some(cached) = self.cache.get(&note.path)
            && cached.modified_at == note.modified_at
            && cached.len == note.len
        {
            return Some(Arc::clone(cached));
        }
        let text = match store.read(&note.path) {
            Ok(read) => read.text,
            Err(error) => {
                tracing::debug!(%error, "searching title only");
                return None;
            }
        };
        let cached = Arc::new(CachedText {
            modified_at: note.modified_at,
            len: note.len,
            folded: fold(&text),
            text,
        });
        if let Some(stale) = self.cache.remove(&note.path) {
            self.cached_bytes -= stale.footprint();
        }
        if self.cached_bytes + cached.footprint() <= CACHE_BUDGET_BYTES {
            self.cached_bytes += cached.footprint();
            self.cache.insert(note.path.clone(), Arc::clone(&cached));
        }
        Some(cached)
    }
}

impl CachedText {
    fn footprint(&self) -> usize {
        self.text.len() + self.folded.len()
    }
}

/// Lower-cases `text` without ever changing a byte offset.
///
/// A character is replaced only by a single lower-case character of the same UTF-8 length. The
/// rare ones that lower-case differently (`İ` becomes two characters, `ẞ` becomes a shorter
/// `ß`) stay as they are, so they match only themselves. In exchange, a match position in the
/// folded text is the match position in the original, always on a char boundary.
fn fold(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    text.chars()
        .map(|c| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) if l.len_utf8() == c.len_utf8() => l,
                _ => c,
            }
        })
        .collect()
}

/// Byte ranges of all non-overlapping occurrences of `needle` in `haystack`.
fn find_all(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    haystack
        .match_indices(needle)
        .map(|(start, found)| start..start + found.len())
        .collect()
}

/// A one-line excerpt of `text` around `first_match` (a byte range of one line) and the ranges
/// of `query` inside it.
fn snippet_around(
    text: &str,
    first_match: Range<usize>,
    query: &str,
) -> (String, Vec<Range<usize>>) {
    let line_breaks = ['\n', '\r'];
    let line_start = text[..first_match.start]
        .rfind(line_breaks)
        .map_or(0, |i| i + 1);
    let line_end = text[first_match.end..]
        .find(line_breaks)
        .map_or(text.len(), |i| first_match.end + i);

    let start = text[line_start..first_match.start]
        .char_indices()
        .rev()
        .nth(CONTEXT_BEFORE - 1)
        .map_or(line_start, |(i, _)| line_start + i);
    let end = text[first_match.end..line_end]
        .char_indices()
        .nth(CONTEXT_AFTER)
        .map_or(line_end, |(i, _)| first_match.end + i);

    let excerpt = text[start..end].trim();
    let mut snippet = String::new();
    if start > line_start {
        snippet.push_str(ELLIPSIS);
    }
    let excerpt_offset = snippet.len();
    snippet.extend(excerpt.chars().map(|c| if c == '\t' { ' ' } else { c }));
    if end < line_end {
        snippet.push_str(ELLIPSIS);
    }

    let ranges = find_all(&fold(excerpt), query)
        .into_iter()
        .map(|r| r.start + excerpt_offset..r.end + excerpt_offset)
        .collect();
    (snippet, ranges)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// A line from an alphabet where case folding is awkward: `İ` and `ẞ` change length when
    /// lower-cased, the Kelvin sign lower-cases to ASCII `k`, `ß` is the lower-case of `ẞ`.
    fn awkward_line() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("İ"),
                Just("ẞ"),
                Just("ß"),
                Just("\u{212A}"),
                Just("é"),
                Just("É"),
                Just("i"),
                Just("I"),
                Just("a"),
                Just("B"),
                Just("日"),
                Just("👋"),
                Just(" "),
                Just("-"),
            ],
            1..200,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #[test]
        fn fold_keeps_every_byte_offset(text in any::<String>()) {
            let folded = fold(&text);
            prop_assert_eq!(folded.len(), text.len());
            for (offset, _) in text.char_indices() {
                prop_assert!(folded.is_char_boundary(offset));
            }
        }

        #[test]
        fn snippet_ranges_are_valid_slices_of_a_single_line(
            lines in proptest::collection::vec(awkward_line(), 1..5),
            pick in any::<proptest::sample::Index>(),
            from in any::<proptest::sample::Index>(),
            len in 1usize..40,
            crlf in any::<bool>(),
        ) {
            let text = lines.join(if crlf { "\r\n" } else { "\n" });
            let chars: Vec<char> = pick.get(&lines).chars().collect();
            let start = from.index(chars.len());
            let query: String = chars[start..].iter().take(len).collect();
            let query = fold(query.trim());
            prop_assume!(!query.is_empty());

            // The query is a piece of one line, so it has to be found.
            let at = fold(&text).find(&query);
            prop_assert!(at.is_some(), "query copied from the text was not found");
            let at = at.unwrap();
            let (snippet, ranges) = snippet_around(&text, at..at + query.len(), &query);

            prop_assert!(!snippet.contains(['\n', '\r']));
            prop_assert!(!ranges.is_empty());
            let mut previous_end = 0;
            for range in &ranges {
                prop_assert!(range.start >= previous_end && range.start < range.end);
                prop_assert!(range.end <= snippet.len());
                prop_assert!(snippet.is_char_boundary(range.start));
                prop_assert!(snippet.is_char_boundary(range.end));
                prop_assert_eq!(fold(&snippet[range.clone()]), query.clone());
                previous_end = range.end;
            }
            let max_chars = CONTEXT_BEFORE + CONTEXT_AFTER + query.chars().count() + 2;
            prop_assert!(snippet.chars().count() <= max_chars);
        }
    }
}
