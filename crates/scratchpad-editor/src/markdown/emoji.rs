//! `:shortcode:` emoji (ADR 0190): a known GitHub shortcode such as `:smile:` or `:+1:` in prose is drawn as its
//! emoji while the selection is off it. The buffer keeps the shortcode.
//!
//! The shortcodes are gemoji's, in a table generated from the `emojis` crate (`emoji_table.rs`; a test checks
//! that it is up to date and rewrites it with `SCRATCHPAD_WRITE_EMOJI_TABLE=1`). The crate itself is only a
//! dev-dependency: its metadata of every emoji would add about half a megabyte to the app (ADR 0190).

use std::ops::Range;

use super::{Decoration, DecorationKind};
use crate::coords::ByteOffset;

#[path = "emoji_table.rs"]
#[rustfmt::skip]
mod table;

/// The longest name worth looking up, in bytes. gemoji's longest is shorter.
const MAX_NAME: usize = 64;

/// The emoji GitHub shows for `:name:`, e.g. `"😄"` for `"smile"`; `None` for a name it does not know.
pub fn emoji_for_shortcode(name: &str) -> Option<&'static str> {
    let table = &table::SHORTCODES;
    table
        .binary_search_by(|(shortcode, _)| (*shortcode).cmp(name))
        .ok()
        .map(|i| table[i].1)
}

/// Whether `b` may be part of a shortcode's name: gemoji's are lower-case letters, digits, `_`, `+` and `-`.
fn name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'+' | b'-')
}

/// Whether the char of `text` ending at `end` (or starting at `start`) is a letter or digit, which a shortcode
/// may not touch: `10:30:45` is a time, `a:b:c` no emoji.
fn alphanumeric_before(text: &str, end: usize) -> bool {
    text[..end]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
}

fn alphanumeric_after(text: &str, start: usize) -> bool {
    text[start..]
        .chars()
        .next()
        .is_some_and(char::is_alphanumeric)
}

/// The shortcode starting with the `:` at `colon` of `text` and closed by `end`, if it names a known emoji and
/// neither colon touches a letter or digit outside it, nor is the first one escaped. Returns its range and emoji.
pub(super) fn shortcode_at(
    text: &str,
    colon: usize,
    end: usize,
) -> Option<(Range<usize>, &'static str)> {
    let bytes = text.as_bytes();
    if alphanumeric_before(text, colon) || colon.checked_sub(1).is_some_and(|i| bytes[i] == b'\\') {
        return None;
    }
    let name_start = colon + 1;
    let name_len = bytes[name_start..end.min(name_start + MAX_NAME + 1)]
        .iter()
        .take_while(|&&b| name_byte(b))
        .count();
    let close = name_start + name_len;
    if name_len == 0 || close >= end || bytes[close] != b':' || alphanumeric_after(text, close + 1)
    {
        return None;
    }
    let emoji = emoji_for_shortcode(&text[name_start..close])?;
    Some((colon..close + 1, emoji))
}

/// The decoration of the shortcode at `range` showing `emoji`.
pub(super) fn decoration(range: Range<usize>, emoji: &'static str) -> Decoration {
    let end = ByteOffset(range.end);
    let range = ByteOffset(range.start)..end;
    Decoration {
        kind: DecorationKind::Emoji { emoji },
        content: end..end,
        markers: vec![range.clone()],
        range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_shortcodes_have_their_emoji() {
        assert_eq!(emoji_for_shortcode("smile"), Some("😄"));
        assert_eq!(emoji_for_shortcode("+1"), Some("👍"));
        assert_eq!(emoji_for_shortcode("-1"), Some("👎"));
        assert_eq!(emoji_for_shortcode("100"), Some("💯"));
        assert_eq!(emoji_for_shortcode("heart"), Some("❤️"));
        assert_eq!(emoji_for_shortcode("not_an_emoji"), None);
        assert_eq!(emoji_for_shortcode("Smile"), None);
        assert_eq!(emoji_for_shortcode(""), None);
    }

    #[test]
    fn shortcodes_do_not_touch_letters_or_digits() {
        let at = |text: &str| {
            text.find(':')
                .and_then(|colon| shortcode_at(text, colon, text.len()))
        };
        assert_eq!(at(":smile:"), Some((0..7, "😄")));
        assert_eq!(at("(:+1:)"), Some((1..5, "👍")));
        assert_eq!(at("é :tada:!"), Some((3..9, "🎉")));
        for text in [
            "a:smile:",
            ":smile:s",
            "10:30:45",
            ":smile",
            ":unknown:",
            r"\:smile:",
            ": smile:",
            ":Smile:",
            "::",
        ] {
            assert_eq!(at(text), None, "{text}");
        }
    }

    /// The table as generated from the `emojis` crate: every shortcode with its emoji, sorted by name.
    fn generated_table() -> String {
        let mut entries: Vec<(&str, &str)> = emojis::iter()
            .flat_map(|emoji| emoji.shortcodes().map(move |name| (name, emoji.as_str())))
            .collect();
        entries.sort_unstable();
        entries.dedup_by_key(|(name, _)| *name);
        let mut out = String::from(
            "// Generated from the `emojis` crate (gemoji's shortcodes; MIT OR Apache-2.0, and Unicode-3.0 for\n\
             // the emoji data) by `markdown::emoji::tests::the_table_is_generated_from_gemoji`. Do not edit.\n\n",
        );
        out.push_str(&format!(
            "pub(super) static SHORTCODES: [(&str, &str); {}] = [\n",
            entries.len()
        ));
        for (name, emoji) in entries {
            let escaped: String = emoji
                .chars()
                .map(|c| format!("\\u{{{:x}}}", c as u32))
                .collect();
            out.push_str(&format!("    (\"{name}\", \"{escaped}\"),\n"));
        }
        out.push_str("];\n");
        out
    }

    #[test]
    fn the_table_is_generated_from_gemoji() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/markdown/emoji_table.rs");
        let generated = generated_table();
        if std::env::var_os("SCRATCHPAD_WRITE_EMOJI_TABLE").is_some() {
            std::fs::write(path, &generated).unwrap();
        }
        let committed = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == generated,
            "emoji_table.rs is out of date: run the tests with SCRATCHPAD_WRITE_EMOJI_TABLE=1"
        );
        for emoji in emojis::iter() {
            for name in emoji.shortcodes() {
                assert_eq!(emoji_for_shortcode(name), Some(emoji.as_str()), "{name}");
            }
        }
    }

    #[test]
    fn no_gemoji_name_is_too_long_to_be_looked_up() {
        assert!(
            emojis::iter()
                .flat_map(emojis::Emoji::shortcodes)
                .all(|name| name.len() <= MAX_NAME && name.bytes().all(name_byte))
        );
    }
}
