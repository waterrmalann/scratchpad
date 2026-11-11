//! Which wikilink targets name a page (ADR 0188).
//!
//! `[[Page]]`, `[[Page|label]]` and `[[Page#Heading]]` link to the page `Page`: a file of that stem in the folder
//! of the note. A target is only a link if its page is a plain file stem that cannot leave that folder: no path
//! separators or other characters Windows forbids in names, no control characters, no `..`, no leading or trailing
//! dots and spaces, no runs of spaces or other whitespace, and not a reserved device name such as `CON` or `com1`.
//! These are the stems the notes store's sanitizing leaves unchanged, so a missing page can be created under its
//! exact name. This is the one check of page names: the app asks it before opening or creating a page, and its
//! agreement with `scratchpad_core::sanitize_file_stem` (whose reserved names it shares) is tested there.

/// The longest page name, in chars: the notes store caps file stems at this.
const MAX_PAGE_CHARS: usize = 100;

/// The page `target` (the text between `[[` and `|` or `]]`) links to, without any `#heading` and the spaces
/// around it, or `None` if that is not a plain file stem and so not a link.
pub fn wikilink_page(target: &str) -> Option<&str> {
    let page = target.split_once('#').map_or(target, |(page, _)| page);
    let page = page.trim_matches(' ');
    is_plain_file_stem(page).then_some(page)
}

fn is_plain_file_stem(name: &str) -> bool {
    let allowed = |c: char| {
        !c.is_control() && !r#"/\:*?"<>|"#.contains(c) && (c == ' ' || !c.is_whitespace())
    };
    !name.is_empty()
        && name.chars().count() <= MAX_PAGE_CHARS
        && name.chars().all(allowed)
        && !name.starts_with(['.', ' '])
        && !name.ends_with(['.', ' '])
        && !name.contains("..")
        && !name.contains("  ")
        && !is_reserved_device_name(name)
}

/// Windows treats a name whose part before the first dot (ignoring trailing spaces) is a device name as that
/// device, e.g. `CON`, `nul.txt` or `COM1`. `COM0` and `LPT0` are documented as reserved too.
fn is_reserved_device_name(name: &str) -> bool {
    const FIXED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    let head = name.split('.').next().unwrap_or(name).trim_end();
    if FIXED
        .iter()
        .any(|reserved| head.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    let upper = head.to_ascii_uppercase();
    let Some(suffix) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = suffix.chars();
    matches!(chars.next(), Some('0'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_the_target_without_its_heading() {
        assert_eq!(wikilink_page("Page"), Some("Page"));
        assert_eq!(wikilink_page("My page 2"), Some("My page 2"));
        assert_eq!(wikilink_page("Page#Heading"), Some("Page"));
        assert_eq!(wikilink_page("Page #Heading"), Some("Page"));
        assert_eq!(wikilink_page(" Page "), Some("Page"));
        assert_eq!(wikilink_page("Page#^block"), Some("Page"));
        assert_eq!(wikilink_page("v1.2 notes"), Some("v1.2 notes"));
        assert_eq!(wikilink_page("日本語"), Some("日本語"));
        assert_eq!(wikilink_page("COM10"), Some("COM10"));
        assert_eq!(wikilink_page("Console"), Some("Console"));
    }

    #[test]
    fn targets_that_could_leave_the_folder_or_are_not_file_names_are_rejected() {
        for target in [
            "",
            "#Heading",
            " ",
            "..",
            ".",
            "../Secret",
            "..\\Secret",
            "a..b",
            "sub/Page",
            "sub\\Page",
            "/etc/passwd",
            "\\\\server\\share",
            "C:Page",
            "C:\\Windows\\win.ini",
            "https://example.org/",
            ".hidden",
            "Page.",
            "two  spaces",
            "tab\there",
            "line\nbreak",
            "nul\u{0}byte",
            "bell\u{7}",
            "non\u{a0}breaking",
            "a*b",
            "a?b",
            "a\"b",
            "a<b>",
            "a|b",
            "CON",
            "con",
            "Nul.txt",
            "aux .md",
            "COM1",
            "lpt9",
            "COM0",
            "lpt0.md",
            "COM¹",
            "CONIN$",
        ] {
            assert_eq!(wikilink_page(target), None, "{target:?}");
        }
        assert_eq!(wikilink_page(&"x".repeat(100)).map(str::len), Some(100));
        assert_eq!(wikilink_page(&"x".repeat(101)), None);
    }
}
