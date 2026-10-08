//! Marker notation for readable editor tests: `|` is the head (cursor) and `^` the anchor of a non-empty
//! selection. Test texts must not otherwise contain these characters.

#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use scratchpad_editor::{ByteOffset, Editor, Selection};

/// Builds an editor from marked text such as `"hel|lo"` or `"^hel|lo"`.
pub fn editor(marked: &str) -> Editor {
    let (text, selection) = parse(marked);
    let mut editor = Editor::from_text(&text);
    editor.set_selection(selection);
    editor
}

/// Types `text` one character at a time, as the keyboard delivers it.
pub fn type_chars(editor: &mut Editor, text: &str) {
    for c in text.chars() {
        editor.insert_text(c.encode_utf8(&mut [0; 4]));
    }
}

/// Renders the editor's (normalized) text with selection markers.
pub fn state(editor: &Editor) -> String {
    let mut text = editor.buffer().normalized_text();
    let selection = editor.selection();
    let mut markers = vec![(selection.head.0, '|')];
    if !selection.is_empty() {
        markers.push((selection.anchor.0, '^'));
    }
    // Insert from the back so earlier offsets stay valid.
    markers.sort_by_key(|&(offset, _)| std::cmp::Reverse(offset));
    for (offset, marker) in markers {
        text.insert(offset, marker);
    }
    text
}

fn parse(marked: &str) -> (String, Selection) {
    let mut text = String::new();
    let mut head = None;
    let mut anchor = None;
    for c in marked.chars() {
        match c {
            '|' => head = Some(ByteOffset(text.len())),
            '^' => anchor = Some(ByteOffset(text.len())),
            c => text.push(c),
        }
    }
    let head = head.expect("marked text needs a `|` cursor");
    (text, Selection::new(anchor.unwrap_or(head), head))
}
