//! Loading, saving and coordinate mapping of documents.

use proptest::prelude::*;
use scratchpad_editor::{Bias, Buffer, ByteOffset, Editor, LineEnding, Point, Utf16Offset};

#[test]
fn windows_file_round_trips_and_edits_keep_crlf() {
    let file = "# Groceries\r\n- milk\r\n- eggs\r\n";
    let mut editor = Editor::from_text(file);
    assert_eq!(editor.buffer().line_ending(), LineEnding::Crlf);
    assert_eq!(
        editor.buffer().normalized_text(),
        "# Groceries\n- milk\n- eggs\n"
    );
    assert_eq!(editor.to_text(), file);

    // Text pasted from another Windows app arrives with CRLF; it must not double up line breaks.
    let end = editor.buffer().end();
    editor.replace_range(end..end, "- bread\r\n- jam\r\n");
    assert_eq!(editor.buffer().line_count(), 6);
    assert_eq!(
        editor.to_text(),
        "# Groceries\r\n- milk\r\n- eggs\r\n- bread\r\n- jam\r\n"
    );
}

#[test]
fn a_snapshot_keeps_the_text_from_when_it_was_taken() {
    let file = "# Plan\r\n- draft\r\n";
    let mut editor = Editor::from_text(file);
    let snapshot = editor.buffer().snapshot();

    let end = editor.buffer().end();
    editor.replace_range(end..end, "- review\r\n");
    editor.replace_range(ByteOffset(2)..ByteOffset(6), "Goals");
    let later = editor.buffer().snapshot();

    assert_eq!(snapshot.to_text(), file);
    assert_eq!(later.to_text(), "# Goals\r\n- draft\r\n- review\r\n");
    // Sent to another thread, as the app does to write it out.
    let sent = std::thread::spawn(move || snapshot.to_text())
        .join()
        .unwrap();
    assert_eq!(sent, file);
}

#[test]
fn unix_file_round_trips_unchanged() {
    let file = "line one\nline two\n\nline four";
    assert_eq!(Editor::from_text(file).to_text(), file);
}

#[test]
fn lone_carriage_returns_become_line_breaks() {
    let buffer = Buffer::from_text("old mac\rstyle\r\r");
    assert_eq!(buffer.normalized_text(), "old mac\nstyle\n\n");
    assert_eq!(buffer.line_count(), 4);
    assert_eq!(buffer.line_text(1), "style");
    assert_eq!(buffer.to_text(), "old mac\nstyle\n\n");
}

#[test]
fn mixed_line_endings_are_unified_to_the_dominant_one() {
    let buffer = Buffer::from_text("a\r\nb\r\nc\r\nd\ne\rf");
    assert_eq!(buffer.line_ending(), LineEnding::Crlf);
    assert_eq!(buffer.to_text(), "a\r\nb\r\nc\r\nd\r\ne\r\nf");

    let buffer = Buffer::from_text("a\nb\nc\r\nd");
    assert_eq!(buffer.line_ending(), LineEnding::Lf);
    assert_eq!(buffer.to_text(), "a\nb\nc\nd");
}

#[test]
fn empty_document_has_one_empty_line() {
    let buffer = Buffer::from_text("");
    assert!(buffer.is_empty());
    assert_eq!(buffer.line_count(), 1);
    assert_eq!(buffer.line_text(0), "");
    assert_eq!(buffer.line_start(0), ByteOffset(0));
    assert_eq!(buffer.line_end(0), ByteOffset(0));
    assert_eq!(buffer.offset_to_point(ByteOffset(0)), Point::new(0, 0));
    assert_eq!(buffer.to_text(), "");
}

#[test]
fn lines_map_to_offsets_in_multibyte_text() {
    // "é" is 2 bytes, "日本" 6 bytes, "👍" 4 bytes.
    let buffer = Buffer::from_text("café\n日本\n\n👍 ok\n");
    assert_eq!(buffer.line_count(), 5);
    assert_eq!(buffer.line_text(1), "日本");
    assert_eq!(buffer.line_start(1), ByteOffset(6));
    assert_eq!(buffer.line_end(1), ByteOffset(12));
    assert_eq!(buffer.line_text(2), "");
    assert_eq!(buffer.line_start(3), ByteOffset(14));
    assert_eq!(buffer.line_text(4), "");
    assert_eq!(
        buffer.line_of(ByteOffset(5)),
        0,
        "the line break belongs to the line it ends"
    );
    assert_eq!(buffer.line_of(ByteOffset(6)), 1);

    assert_eq!(buffer.offset_to_point(ByteOffset(9)), Point::new(1, 3));
    assert_eq!(buffer.point_to_offset(Point::new(3, 4)), ByteOffset(18));
    // Out-of-range points clamp to the line / document instead of panicking.
    assert_eq!(buffer.point_to_offset(Point::new(1, 100)), ByteOffset(12));
    assert_eq!(buffer.point_to_offset(Point::new(100, 0)), buffer.end());
    // A column inside a multi-byte char rounds down to its start.
    assert_eq!(buffer.point_to_offset(Point::new(1, 4)), ByteOffset(9));
    assert_eq!(buffer.line_start(100), ByteOffset(buffer.len()));
}

/// Guards against ropey's `unicode_lines`/`cr_lines` features being switched on by cargo feature unification: the
/// engine's notion of a line must stay "terminated by `\n`".
#[test]
fn only_line_feed_separates_lines() {
    let buffer = Buffer::from_text("a\u{2028}b\u{2029}c\u{85}d\u{b}e\u{c}f");
    assert_eq!(buffer.line_count(), 1);
}

#[test]
fn utf16_offsets_count_surrogate_pairs() {
    let buffer = Buffer::from_text("a😀b");
    assert_eq!(buffer.offset_to_utf16(ByteOffset(5)), Utf16Offset(3));
    assert_eq!(buffer.utf16_to_offset(Utf16Offset(3)), ByteOffset(5));
    assert_eq!(
        buffer.utf16_to_offset(Utf16Offset(2)),
        ByteOffset(1),
        "inside a surrogate pair rounds down"
    );
    assert_eq!(buffer.utf16_to_offset(Utf16Offset(99)), buffer.end());
}

#[test]
fn offsets_snap_to_grapheme_clusters() {
    // "e" + combining acute, then a family emoji made of 7 code points.
    let buffer = Buffer::from_text("e\u{301}👨‍👩‍👧x");
    let emoji_start = ByteOffset(3);
    let x = ByteOffset(buffer.len() - 1);

    assert_eq!(buffer.clip_offset(ByteOffset(1), Bias::Left), ByteOffset(0));
    assert_eq!(buffer.clip_offset(ByteOffset(1), Bias::Right), emoji_start);
    assert_eq!(buffer.clip_offset(ByteOffset(8), Bias::Left), emoji_start);
    assert_eq!(buffer.clip_offset(ByteOffset(8), Bias::Right), x);
    assert_eq!(buffer.next_grapheme_boundary(emoji_start), x);
    assert_eq!(buffer.prev_grapheme_boundary(x), emoji_start);
    assert_eq!(
        buffer.clip_offset(ByteOffset(999), Bias::Left),
        buffer.end()
    );
}

fn line_break() -> impl Strategy<Value = &'static str> {
    prop_oneof![Just("\n"), Just("\r\n"), Just("\r")]
}

/// Text without any line break characters.
fn line() -> impl Strategy<Value = String> {
    any::<String>().prop_map(|s| s.replace(['\r', '\n'], ""))
}

proptest! {
    /// Files with consistent LF or CRLF line endings are saved byte-for-byte as they were loaded.
    #[test]
    fn consistent_line_endings_round_trip_exactly(
        lines in prop::collection::vec(line(), 0..8),
        crlf in any::<bool>(),
    ) {
        let text = lines.join(if crlf { "\r\n" } else { "\n" });
        prop_assert_eq!(Buffer::from_text(&text).to_text(), text);
    }

    /// Any text, including mixed and lone-CR line breaks, is saved with every line break written as the detected
    /// line ending and nothing else changed; loading the saved text again is stable.
    #[test]
    fn any_text_saves_with_unified_line_endings(
        pieces in prop::collection::vec((any::<String>(), line_break()), 0..8),
        tail in any::<String>(),
    ) {
        let text: String = pieces.iter().flat_map(|(s, br)| [s.as_str(), br]).chain([tail.as_str()]).collect();
        let buffer = Buffer::from_text(&text);
        let saved = buffer.to_text();

        let expected = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', match buffer.line_ending() {
                LineEnding::Lf => "\n",
                LineEnding::Crlf => "\r\n",
            });
        prop_assert_eq!(&saved, &expected);

        let reloaded = Buffer::from_text(&saved);
        prop_assert_eq!(reloaded.normalized_text(), buffer.normalized_text());
        prop_assert_eq!(reloaded.to_text(), saved);
    }
}
