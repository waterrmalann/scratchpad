use proptest::prelude::*;
use scratchpad_core::{sanitize_file_stem, title_from_content};

#[test]
fn title_is_first_meaningful_line() {
    let text = "\n   \n# HTTP Gateway Ideas\n\nNeed to benchmark...";
    assert_eq!(
        title_from_content(text).as_deref(),
        Some("HTTP Gateway Ideas")
    );
}

#[test]
fn title_strips_nested_block_markers() {
    assert_eq!(
        title_from_content("> - [ ] Buy milk").as_deref(),
        Some("Buy milk")
    );
    assert_eq!(
        title_from_content("12. Second step").as_deref(),
        Some("Second step")
    );
    assert_eq!(title_from_content("### Deep   ").as_deref(), Some("Deep"));
}

#[test]
fn title_keeps_text_that_only_looks_like_a_marker() {
    assert_eq!(title_from_content("#hashtag").as_deref(), Some("#hashtag"));
    assert_eq!(
        title_from_content("-5 degrees").as_deref(),
        Some("-5 degrees")
    );
    assert_eq!(
        title_from_content("2024 plans").as_deref(),
        Some("2024 plans")
    );
    assert_eq!(
        title_from_content("####### seven").as_deref(),
        Some("####### seven")
    );
}

#[test]
fn title_skips_lines_without_text() {
    assert_eq!(
        title_from_content("#\n---\n```rust\n* * *\n- \nReal title").as_deref(),
        Some("Real title")
    );
    assert_eq!(title_from_content("  \n\n#\n"), None);
    assert_eq!(title_from_content(""), None);
}

#[test]
fn title_ends_at_any_line_break() {
    // Classic Mac OS line endings: a lone CR must not glue the body onto the title.
    assert_eq!(
        title_from_content("\r\rTitle\rBody").as_deref(),
        Some("Title")
    );
    assert_eq!(
        title_from_content("Title\r\nBody").as_deref(),
        Some("Title")
    );
}

#[test]
fn title_is_capped_on_a_char_boundary() {
    let title = title_from_content(&"é".repeat(200)).unwrap();
    assert_eq!(title.chars().count(), 80);
}

#[test]
fn sanitize_replaces_forbidden_characters_with_spaces() {
    assert_eq!(
        sanitize_file_stem("A/B: what?").as_deref(),
        Some("A B what")
    );
    assert_eq!(
        sanitize_file_stem(r#"a\b*c"d<e>f|g"#).as_deref(),
        Some("a b c d e f g")
    );
    assert_eq!(
        sanitize_file_stem("tab\tand\nnewline\u{7}").as_deref(),
        Some("tab and newline")
    );
}

#[test]
fn sanitize_trims_dots_and_spaces() {
    assert_eq!(
        sanitize_file_stem("  ..hidden. . ").as_deref(),
        Some("hidden")
    );
    assert_eq!(sanitize_file_stem("v1.2.3").as_deref(), Some("v1.2.3"));
}

#[test]
fn sanitize_rejects_empty_results() {
    for title in ["", "   ", "...", "///", "?*:", "\u{0}"] {
        assert_eq!(sanitize_file_stem(title), None, "{title:?}");
    }
}

#[test]
fn sanitize_defuses_reserved_device_names() {
    assert_eq!(sanitize_file_stem("CON").as_deref(), Some("CON_"));
    assert_eq!(sanitize_file_stem("nul").as_deref(), Some("nul_"));
    assert_eq!(sanitize_file_stem("Com1").as_deref(), Some("Com1_"));
    assert_eq!(sanitize_file_stem("LPT9.txt").as_deref(), Some("LPT9_.txt"));
    assert_eq!(sanitize_file_stem("aux .md").as_deref(), Some("aux_.md"));
    assert_eq!(sanitize_file_stem("COM¹").as_deref(), Some("COM¹_"));
    // Names that merely start with a reserved word are fine.
    assert_eq!(sanitize_file_stem("Console").as_deref(), Some("Console"));
    assert_eq!(sanitize_file_stem("COM10").as_deref(), Some("COM10"));
}

#[test]
fn sanitize_caps_length_without_splitting_chars() {
    let stem = sanitize_file_stem(&"日本語 ".repeat(100)).unwrap();
    assert!(stem.chars().count() <= 100);
    assert!(!stem.ends_with(' '));
}

fn is_valid_windows_stem(stem: &str) -> bool {
    let head = stem
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&head.as_str())
        || ((head.starts_with("COM") || head.starts_with("LPT"))
            && head.len() == 4
            && head.ends_with(|c: char| ('1'..='9').contains(&c)));
    !stem.is_empty()
        && !stem.starts_with('.')
        && !stem.ends_with(['.', ' '])
        && !stem
            .chars()
            .any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c))
        && !reserved
}

proptest! {
    #[test]
    fn sanitized_stems_are_valid_and_stable(title in any::<String>()) {
        if let Some(stem) = sanitize_file_stem(&title) {
            prop_assert!(is_valid_windows_stem(&stem), "{stem:?}");
            prop_assert_eq!(sanitize_file_stem(&stem), Some(stem));
        }
    }

    #[test]
    fn sanitized_stems_survive_hostile_titles(
        title in r"[ .a-zA-Z0-9/\\:*?<>|\u{0}-\u{1f}]{0,40}(con|nul|com1|lpt3)?[ .]{0,3}[a-z]{0,3}"
    ) {
        if let Some(stem) = sanitize_file_stem(&title) {
            prop_assert!(is_valid_windows_stem(&stem), "{stem:?}");
            prop_assert_eq!(sanitize_file_stem(&stem), Some(stem));
        }
    }

    #[test]
    fn titles_never_exceed_the_cap_or_panic(text in any::<String>()) {
        if let Some(title) = title_from_content(&text) {
            prop_assert!(!title.is_empty());
            prop_assert!(title.chars().count() <= 80);
            prop_assert_eq!(title.trim(), title.as_str());
        }
    }
}
