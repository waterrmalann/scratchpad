//! Note titles and file names.
//!
//! In V1 a note's title is its file stem, so a typed title has to survive the trip through
//! [`sanitize_file_stem`] and a collision check before it can become a file name.

/// Title (and file stem) used when nothing meaningful is available.
pub const UNTITLED: &str = "Untitled";

const MAX_TITLE_CHARS: usize = 80;
/// NTFS allows 255 UTF-16 units per name; staying well below leaves room for " 12" and ".md" and
/// keeps full paths clear of the legacy 260 character limit.
const MAX_STEM_CHARS: usize = 100;

/// The first meaningful line of `text`, usable as a note title.
///
/// Blank lines, thematic breaks (`---`) and code fences are skipped. Leading Markdown block
/// markers (`#`, `>`, `-`, `1.`, `[ ]`) are stripped. The result is capped at 80 characters.
pub fn title_from_content(text: &str) -> Option<String> {
    // Not `str::lines`: it keeps a lone `\r` (classic Mac line ending) inside the line.
    text.split(['\n', '\r']).find_map(title_from_line)
}

fn title_from_line(line: &str) -> Option<String> {
    let mut rest = line.trim();
    if is_rule_or_fence(rest) {
        return None;
    }
    loop {
        let stripped = strip_block_marker(rest).trim_start();
        if stripped.len() == rest.len() {
            break;
        }
        rest = stripped;
    }
    let title: String = rest.chars().take(MAX_TITLE_CHARS).collect();
    let title = title.trim_end();
    (!title.is_empty()).then(|| title.to_owned())
}

fn is_rule_or_fence(line: &str) -> bool {
    if line.starts_with("```") || line.starts_with("~~~") {
        return true;
    }
    let mut marks = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = marks.next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_' | '=')
        && marks.clone().all(|c| c == first)
        && marks.count() >= 2
}

/// Removes one leading block marker, or returns `s` unchanged.
fn strip_block_marker(s: &str) -> &str {
    // A marker only counts when followed by whitespace, so `#tag` and `-5` stay as they are.
    let marker_end = |rest: &str| rest.is_empty() || rest.starts_with(char::is_whitespace);

    if let Some(rest) = s.strip_prefix('>') {
        return rest;
    }
    let hashes = s.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) && marker_end(&s[hashes..]) {
        return &s[hashes..];
    }
    if s.starts_with(['-', '*', '+']) && marker_end(&s[1..]) {
        return &s[1..];
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits)
        && s[digits..].starts_with(['.', ')'])
        && marker_end(&s[digits + 1..])
    {
        return &s[digits + 1..];
    }
    for checkbox in ["[ ]", "[x]", "[X]"] {
        if let Some(rest) = s.strip_prefix(checkbox)
            && marker_end(rest)
        {
            return rest;
        }
    }
    s
}

/// Turns arbitrary text into a string that is safe as a file stem on Windows, macOS and Linux.
///
/// Characters Windows forbids (`/ \ : * ? " < > |`) and control characters become spaces, runs of
/// whitespace collapse, and leading or trailing dots and spaces are removed (a leading dot would
/// hide the note from the sidebar). Reserved device names such as `CON` or `com1.txt` get an
/// underscore appended to their first component. The result is capped at 100 characters.
///
/// Returns `None` when nothing usable is left. The function is idempotent.
pub fn sanitize_file_stem(title: &str) -> Option<String> {
    let spaced: String = title
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(['.', ' ']);
    if trimmed.is_empty() {
        return None;
    }

    // Windows treats everything before the first dot as the device name, ignoring trailing spaces.
    let (head, tail) = trimmed.split_at(trimmed.find('.').unwrap_or(trimmed.len()));
    let head = head.trim_end();
    let renamed = if is_reserved_device_name(head) {
        format!("{head}_{tail}")
    } else {
        trimmed.to_owned()
    };

    let capped: String = renamed.chars().take(MAX_STEM_CHARS).collect();
    let capped = capped.trim_end_matches(['.', ' ']);
    (!capped.is_empty()).then(|| capped.to_owned())
}

fn is_reserved_device_name(name: &str) -> bool {
    const FIXED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if FIXED
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    let upper = name.to_ascii_uppercase();
    let Some(suffix) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    // Windows 10+ also reserves the superscript digits.
    let mut chars = suffix.chars();
    matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
}
