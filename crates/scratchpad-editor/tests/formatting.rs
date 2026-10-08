//! Markdown formatting shortcuts: Ctrl+B / Ctrl+I and friends edit delimiters in the text.

mod common;

use common::{editor, state};
use proptest::prelude::*;
use scratchpad_editor::markdown::{DecorationKind, MarkdownState};
use scratchpad_editor::{ByteOffset, Editor, Selection};

#[test]
fn select_a_word_and_press_ctrl_b_twice() {
    let mut ed = editor("Say ^hello| now");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "Say **^hello|** now");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "Say ^hello| now");

    assert!(ed.undo());
    assert_eq!(state(&ed), "Say **^hello|** now");
    assert!(ed.undo());
    assert_eq!(state(&ed), "Say ^hello| now");
    assert!(!ed.undo());
}

#[test]
fn a_backward_selection_keeps_its_direction() {
    let mut ed = editor("|hello^");
    toggle(&mut ed, Editor::toggle_italic);
    assert_eq!(state(&ed), "*|hello^*");
}

#[test]
fn the_cursor_inside_a_word_formats_the_word() {
    let mut ed = editor("Say hel|lo now");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "Say **hel|lo** now");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "Say hel|lo now");
}

#[test]
fn the_cursor_elsewhere_gets_an_empty_pair_to_type_into() {
    let mut ed = editor("Say |");
    toggle(&mut ed, Editor::toggle_italic);
    assert_eq!(state(&ed), "Say *|*");
    ed.insert_text("x");
    assert_eq!(state(&ed), "Say *x|*");

    let mut ed = editor("end of word|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "end of word**|**");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "end of word|");
}

#[test]
fn markers_selected_along_with_the_text_are_removed() {
    let mut ed = editor("a ^**b**| c");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "a ^b| c");

    let mut ed = editor("^`code`|");
    toggle(&mut ed, Editor::toggle_inline_code);
    assert_eq!(state(&ed), "^code|");
}

#[test]
fn italic_and_bold_combine_on_shared_runs() {
    let mut ed = editor("**^b|**");
    toggle(&mut ed, Editor::toggle_italic);
    assert_eq!(state(&ed), "***^b|***");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "*^b|*");
    toggle(&mut ed, Editor::toggle_italic);
    assert_eq!(state(&ed), "^b|");
}

#[test]
fn underscore_emphasis_is_recognised_when_toggling_off() {
    let mut ed = editor("__^b|__");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "^b|");

    let mut ed = editor("_wo|rd_ x");
    toggle(&mut ed, Editor::toggle_italic);
    assert_eq!(state(&ed), "wo|rd x");
}

#[test]
fn strikethrough_and_inline_code_toggle_like_bold() {
    let mut ed = editor("^gone|");
    toggle(&mut ed, Editor::toggle_strikethrough);
    assert_eq!(state(&ed), "~~^gone|~~");
    toggle(&mut ed, Editor::toggle_strikethrough);
    assert_eq!(state(&ed), "^gone|");

    let mut ed = editor("call fo|o()");
    toggle(&mut ed, Editor::toggle_inline_code);
    assert_eq!(state(&ed), "call `fo|o`()");
    toggle(&mut ed, Editor::toggle_inline_code);
    assert_eq!(state(&ed), "call fo|o()");
}

#[test]
fn words_with_combining_marks_are_formatted_whole() {
    let mut ed = editor("nai\u{308}|ve");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**nai\u{308}|ve**");
}

#[test]
fn a_multi_line_selection_is_wrapped_at_its_ends() {
    let mut ed = editor("^one\ntwo|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**^one\ntwo|**");
}

#[test]
fn insert_link_wraps_the_selection_and_waits_for_the_destination() {
    let mut ed = editor("see ^docs| here");
    ed.insert_link();
    assert_eq!(state(&ed), "see [docs](|) here");
    ed.insert_text("https://x.y");
    assert_eq!(state(&ed), "see [docs](https://x.y|) here");

    let mut ed = editor("a |");
    ed.insert_link();
    assert_eq!(state(&ed), "a [|]()");
    assert!(ed.undo());
    assert_eq!(state(&ed), "a |");
}

#[test]
fn a_selection_partly_formatted_becomes_formatted_as_a_whole() {
    let mut ed = editor("^a **bo|ld** c");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**^a bo|ld** c");

    let mut ed = editor("a **bo^ld** c|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "a **bo^ld c|**");

    let mut ed = editor("^one **two** three|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**^one two three|**");

    let mut ed = editor("^**a** b **c**|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**^a b c|**");
    assert!(ed.undo());
    assert_eq!(state(&ed), "^**a** b **c**|");

    // A span right next to the selection joins it.
    let mut ed = editor("**a**^b| c");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**a^b|** c");

    let mut ed = editor("^`a` b|");
    toggle(&mut ed, Editor::toggle_inline_code);
    assert_eq!(state(&ed), "`^a b|`");
}

#[test]
fn a_selection_inside_formatted_text_removes_the_whole_span() {
    let mut ed = editor("a **b^ol|d** c");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "a b^ol|d c");

    let mut ed = editor("^**a** **b**|");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "^a b|");
}

#[test]
fn the_cursor_right_after_a_span_removes_it_rather_than_nesting_markers() {
    let mut ed = editor("a **bold**| c");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "a bold| c");

    let mut ed = editor("**bold|**");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "bold|");
}

#[test]
fn whitespace_at_the_selection_ends_stays_outside_the_markers() {
    let mut ed = editor("^word |next");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "**^word** |next");
    toggle(&mut ed, Editor::toggle_bold);
    assert_eq!(state(&ed), "^word |next");
}

fn toggle(ed: &mut Editor, toggle: fn(&mut Editor, &mut MarkdownState)) {
    let mut markdown = MarkdownState::new(ed.buffer());
    toggle(ed, &mut markdown);
}

const TOGGLES: [fn(&mut Editor, &mut MarkdownState); 4] = [
    Editor::toggle_bold,
    Editor::toggle_italic,
    Editor::toggle_strikethrough,
    Editor::toggle_inline_code,
];

/// Whether the decoration kind is the one `TOGGLES[toggle]` formats.
fn formats(toggle: usize, kind: &DecorationKind) -> bool {
    matches!(
        (toggle, kind),
        (0, DecorationKind::Strong)
            | (1, DecorationKind::Emphasis)
            | (2, DecorationKind::Strikethrough)
            | (3, DecorationKind::InlineCode)
    )
}

/// The visible (not whitespace, not a marker) chars of the selection, and how many of them have the
/// formatting of `TOGGLES[toggle]`.
fn formatted_chars(ed: &Editor, toggle: usize) -> (usize, usize) {
    let buffer = ed.buffer();
    let range = ed.selection().range();
    let decorations = MarkdownState::new(buffer).decorations(buffer, range.clone());
    let inside =
        |offset: ByteOffset, r: &std::ops::Range<ByteOffset>| r.start <= offset && offset < r.end;
    let mut visible = 0;
    let mut formatted = 0;
    for (i, c) in buffer.text_for_range(range.clone()).char_indices() {
        let offset = ByteOffset(range.start.0 + i);
        if c.is_whitespace()
            || decorations
                .iter()
                .flat_map(|d| &d.markers)
                .any(|m| inside(offset, m))
        {
            continue;
        }
        visible += 1;
        if decorations
            .iter()
            .any(|d| formats(toggle, &d.kind) && inside(offset, &d.content))
        {
            formatted += 1;
        }
    }
    (visible, formatted)
}

/// Words and well-formed spans of one formatting kind.
fn formatted_text(toggle: usize) -> impl Strategy<Value = String> {
    let (open, close) = [("**", "**"), ("*", "*"), ("~~", "~~"), ("`", "`")][toggle];
    let word = prop::sample::select(&["a", "bc", "日本", "e\u{301}"][..]);
    let token = prop_oneof![
        word.clone().prop_map(str::to_string),
        Just(" ".to_string()),
        word.prop_map(move |w| format!("{open}{w}{close}")),
    ];
    // Starting with a word keeps the text from being indented code.
    prop::collection::vec(token, 0..10).prop_map(|tokens| format!("x {}", tokens.join(" ")))
}

fn text_and_toggle() -> impl Strategy<Value = (String, usize)> {
    (0..4usize).prop_flat_map(|toggle| (formatted_text(toggle), Just(toggle)))
}

proptest! {
    /// Toggling twice restores the text and the selected text, as long as no span of that formatting touches
    /// the selection (otherwise the first toggle merges them) and it has no whitespace at its ends (which stays
    /// outside the inserted markers).
    #[test]
    fn toggling_twice_restores_the_text(
        (text, toggle) in text_and_toggle(),
        anchor in 0..40usize,
        head in 0..40usize,
    ) {
        let mut ed = Editor::from_text(&text);
        ed.set_selection(Selection::new(ByteOffset(anchor), ByteOffset(head)));
        let selected = ed.copy().unwrap_or_default();
        let buffer = ed.buffer();
        let touched = MarkdownState::new(buffer).decorations(buffer, ed.selection().range());
        prop_assume!(selected.trim() == selected && !touched.iter().any(|d| formats(toggle, &d.kind)));
        let mut markdown = MarkdownState::new(ed.buffer());
        TOGGLES[toggle](&mut ed, &mut markdown);
        TOGGLES[toggle](&mut ed, &mut markdown);
        prop_assert_eq!(ed.buffer().normalized_text(), text);
        prop_assert_eq!(ed.copy().unwrap_or_default(), selected);
    }

    /// Like in a word processor, a toggle leaves all visible text of the selection formatted, unless all of it
    /// already was: then none of it is.
    #[test]
    fn toggling_formats_all_or_nothing(
        (text, toggle) in text_and_toggle(),
        anchor in 0..40usize,
        head in 0..40usize,
    ) {
        let mut ed = Editor::from_text(&text);
        ed.set_selection(Selection::new(ByteOffset(anchor), ByteOffset(head)));
        let (visible, formatted) = formatted_chars(&ed, toggle);
        prop_assume!(visible > 0);
        crate::toggle(&mut ed, TOGGLES[toggle]);
        let after = formatted_chars(&ed, toggle);
        let expected = if formatted == visible { (visible, 0) } else { (visible, visible) };
        prop_assert_eq!(after, expected, "{:?}", ed.buffer().normalized_text());
    }
}
