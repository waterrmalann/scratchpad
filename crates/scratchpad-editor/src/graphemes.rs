//! Grapheme cluster boundaries over a rope.
//!
//! A rope stores text in chunks, and a grapheme cluster may straddle a chunk boundary, so [`Graphemes`] feeds
//! `GraphemeCursor` chunk by chunk instead of materialising whole lines. That keeps cursor movement cheap even on a
//! single 10 MB line.

use std::borrow::Cow;

use ropey::Rope;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

/// A position in a rope that steps from one grapheme cluster boundary to the next.
///
/// Walks of many steps (word motions, grapheme columns) must reuse one `Graphemes`: its cursor remembers how many
/// regional indicators precede it, which a fresh cursor can only learn by scanning back through the whole run of
/// flags. A fresh cursor per step makes walking a long line of flags quadratic.
pub(crate) struct Graphemes<'a> {
    rope: &'a Rope,
    cursor: GraphemeCursor,
    /// The text fed to the cursor, which contains the cursor's offset or ends or starts at it: a rope chunk, or a
    /// copy spanning two chunks (see [`Graphemes::cross_into_next_chunk`]).
    chunk: Cow<'a, str>,
    chunk_start: usize,
}

impl<'a> Graphemes<'a> {
    /// `offset` must be a char boundary within the rope.
    pub fn at(rope: &'a Rope, offset: usize) -> Self {
        let mut graphemes = Self {
            rope,
            cursor: GraphemeCursor::new(offset, rope.len_bytes(), true),
            chunk: Cow::Borrowed(""),
            chunk_start: 0,
        };
        graphemes.load_chunk_containing(offset);
        if offset > 0 && offset == graphemes.chunk_start {
            graphemes.load_chunk_containing(offset - 1);
            graphemes.cross_into_next_chunk();
        }
        graphemes
    }

    pub fn offset(&self) -> usize {
        self.cursor.cur_cursor()
    }

    pub fn is_boundary(&mut self) -> bool {
        loop {
            match self.cursor.is_boundary(&self.chunk, self.chunk_start) {
                Ok(is_boundary) => return is_boundary,
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(end),
                // Only pre-context is ever requested at a char boundary.
                Err(_) => return false,
            }
        }
    }

    /// Moves to the next boundary and returns it; `None` at the end of the rope.
    pub fn next(&mut self) -> Option<usize> {
        loop {
            match self.cursor.next_boundary(&self.chunk, self.chunk_start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(end),
                Err(GraphemeIncomplete::NextChunk) => self.cross_into_next_chunk(),
                // The chunk always contains the cursor, so the offset cannot be invalid.
                Err(_) => return None,
            }
        }
    }

    /// Moves to the previous boundary and returns it; `None` at the start of the rope.
    pub fn prev(&mut self) -> Option<usize> {
        loop {
            match self.cursor.prev_boundary(&self.chunk, self.chunk_start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(end),
                Err(GraphemeIncomplete::PrevChunk) => {
                    self.load_chunk_containing(self.chunk_start - 1)
                }
                Err(_) => return None,
            }
        }
    }

    fn load_chunk_containing(&mut self, byte: usize) {
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(byte);
        self.chunk = Cow::Borrowed(chunk);
        self.chunk_start = chunk_start;
    }

    /// Feeds the cursor the next rope chunk, prefixed with the last char of the current one, so that the cursor
    /// never sits exactly at the start of the text it is given.
    ///
    /// There, `GraphemeCursor` (unicode-segmentation 1.13) asks for pre-context and then mishandles it: it counts
    /// a regional indicator it had already counted, splitting flags, and it joins a prepended concatenation mark
    /// (e.g. U+0600) to a following line break. The copy costs one chunk (about 1 KB) per chunk crossed.
    fn cross_into_next_chunk(&mut self) {
        let end = self.chunk_start + self.chunk.len();
        let (next, _, _, _) = self.rope.chunk_at_byte(end);
        let last_char = self.chunk.chars().next_back().map_or(0, char::len_utf8);
        let mut text = String::with_capacity(last_char + next.len());
        text.push_str(&self.chunk[self.chunk.len() - last_char..]);
        text.push_str(next);
        self.chunk = Cow::Owned(text);
        self.chunk_start = end - last_char;
    }

    /// Provides the text ending at `context_end`, which need not be a chunk boundary.
    fn provide_context(&mut self, context_end: usize) {
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(context_end - 1);
        self.cursor
            .provide_context(&chunk[..context_end - chunk_start], chunk_start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn str_boundaries(text: &str) -> Vec<usize> {
        text.grapheme_indices(true)
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect()
    }

    /// Walking with one cursor, stepping with a fresh cursor each time, and testing every char boundary must all
    /// agree with `str` segmentation.
    fn assert_boundaries_match_str(text: &str) {
        let rope = Rope::from_str(text);
        let expected = str_boundaries(text);

        let mut walk = Graphemes::at(&rope, 0);
        let forward: Vec<usize> = [0]
            .into_iter()
            .chain(std::iter::from_fn(|| walk.next()))
            .collect();
        assert_eq!(forward, expected);
        let mut steps = vec![0];
        while let Some(next) = Graphemes::at(&rope, *steps.last().unwrap()).next() {
            steps.push(next);
        }
        assert_eq!(steps, expected);

        let mut walk = Graphemes::at(&rope, text.len());
        let mut backward: Vec<usize> = [text.len()]
            .into_iter()
            .chain(std::iter::from_fn(|| walk.prev()))
            .collect();
        backward.reverse();
        assert_eq!(backward, expected);
        let mut steps = vec![text.len()];
        while let Some(prev) = Graphemes::at(&rope, *steps.last().unwrap()).prev() {
            steps.push(prev);
        }
        steps.reverse();
        assert_eq!(steps, expected);

        let detected: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .filter(|&i| Graphemes::at(&rope, i).is_boundary())
            .collect();
        assert_eq!(detected, expected);
    }

    /// Ropey chunks are around 1 KB, so a long run of multi-code-point clusters is guaranteed to put some of them
    /// across chunk boundaries — the case the chunk-feeding loops exist for.
    #[test]
    fn boundaries_match_str_segmentation_across_chunks() {
        let text = "e\u{301}👩‍👩‍👧‍👦🇩🇪a\u{20dd}\u{20dd}한\r\n🇫🇷🇩🇪🇫".repeat(400);
        assert!(
            Rope::from_str(&text).chunks().count() > 4,
            "test text must span several chunks"
        );
        assert_boundaries_match_str(&text);
        // An Arabic number sign (a prepended concatenation mark) before a line break at a chunk boundary.
        assert_boundaries_match_str(&"a\u{600}\n".repeat(751));
    }

    /// Code points whose segmentation depends on context: combining marks, joiners, emoji with modifiers,
    /// regional indicators, Hangul jamo, a prepended concatenation mark, a Devanagari conjunct and CR/LF.
    const ATOMS: &[&str] = &[
        "a",
        "\u{301}",
        "\u{200d}",
        "👩",
        "🏽",
        "\u{1f1e9}",
        "\u{1f1ea}",
        "\u{1100}",
        "\u{1161}",
        "\u{11a8}",
        "\u{600}",
        "\u{915}\u{94d}",
        "\r",
        "\n",
        "日",
    ];

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn walks_match_str_segmentation_for_hostile_text_spanning_chunks(
            unit in prop::collection::vec(prop::sample::select(ATOMS), 1..40),
            repeats in 50..200usize,
            moves in prop::collection::vec(any::<bool>(), 0..200),
            start in any::<prop::sample::Index>(),
        ) {
            let text = unit.concat().repeat(repeats);
            assert_boundaries_match_str(&text);

            // Changing direction mid-walk, including right after crossing a chunk.
            let rope = Rope::from_str(&text);
            let expected = str_boundaries(&text);
            let mut index = start.index(expected.len());
            let mut walk = Graphemes::at(&rope, expected[index]);
            for forward in moves {
                if forward {
                    prop_assert_eq!(walk.next(), expected.get(index + 1).copied());
                    index = (index + 1).min(expected.len() - 1);
                } else {
                    prop_assert_eq!(walk.prev(), index.checked_sub(1).map(|i| expected[i]));
                    index = index.saturating_sub(1);
                }
            }
        }
    }

    #[test]
    fn empty_rope_has_a_single_boundary() {
        let rope = Rope::new();
        let mut graphemes = Graphemes::at(&rope, 0);
        assert!(graphemes.is_boundary());
        assert_eq!(graphemes.prev(), None);
        assert_eq!(graphemes.next(), None);
    }
}
