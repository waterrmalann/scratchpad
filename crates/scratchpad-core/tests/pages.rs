//! Pages wikilinks resolve to: stems ignoring case, `.md` before `.txt`, and names that cannot leave the folder.

use std::fs;
use std::path::{Path, PathBuf};

use scratchpad_core::{PageIndex, is_page_path, is_plain_stem};

fn paths(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|name| dir.join(name)).collect()
}

#[test]
fn pages_resolve_by_stem_ignoring_case_and_prefer_markdown() {
    let dir = Path::new("C:/notes");
    let index = PageIndex::from_paths(paths(
        dir,
        &[
            "Ideas.md",
            "ideas.txt",
            "Todo.TXT",
            "Log.txt",
            "Café.md",
            "image.png",
            ".hidden.md",
        ],
    ));
    assert_eq!(index.resolve("ideas"), Some(dir.join("Ideas.md").as_path()));
    assert_eq!(index.resolve("IDEAS"), Some(dir.join("Ideas.md").as_path()));
    assert_eq!(index.resolve("todo"), Some(dir.join("Todo.TXT").as_path()));
    assert_eq!(index.resolve("log"), Some(dir.join("Log.txt").as_path()));
    assert_eq!(index.resolve("CAFÉ"), Some(dir.join("Café.md").as_path()));
    assert_eq!(index.resolve("image"), None);
    assert_eq!(index.resolve(".hidden"), None);
    assert_eq!(index.resolve("Missing"), None);
    assert_eq!(index.len(), 4);
}

#[test]
fn insert_and_remove_keep_the_preferred_file() {
    let dir = Path::new("C:/notes");
    let mut index = PageIndex::default();
    assert!(index.insert(dir.join("Plan.txt")));
    assert!(!index.insert(dir.join("Plan.txt")));
    assert!(index.contains(&dir.join("Plan.txt")) && !index.contains(&dir.join("Plan.md")));
    assert!(!index.insert(dir.join("photo.jpg")));
    assert_eq!(index.resolve("plan"), Some(dir.join("Plan.txt").as_path()));
    assert!(index.insert(dir.join("plan.md")));
    assert_eq!(index.resolve("Plan"), Some(dir.join("plan.md").as_path()));
    assert!(index.remove(&dir.join("plan.md")));
    assert!(!index.remove(&dir.join("plan.md")));
    assert_eq!(index.resolve("Plan"), Some(dir.join("Plan.txt").as_path()));
    assert!(index.remove(&dir.join("Plan.txt")));
    assert!(index.is_empty());
}

#[test]
fn names_that_could_leave_the_folder_resolve_to_nothing() {
    let dir = Path::new("C:/notes");
    // Even a file actually named like the escaped target is not reached through it.
    let index = PageIndex::from_paths(paths(dir, &["CON.md", "a..b.md", "x.md"]));
    for page in [
        "",
        "..",
        "../x",
        "..\\x",
        "sub/x",
        "sub\\x",
        "C:x",
        "C:\\x",
        "/x",
        "\\\\host\\x",
        "CON",
        "con",
        "nul.txt",
        "COM1",
        "lpt9",
        "a..b",
        " x",
        "x ",
        ".x",
        "x.",
        "x\ty",
        "x\u{0}",
        "two  spaces",
    ] {
        assert_eq!(index.resolve(page), None, "{page:?}");
        assert!(!is_plain_stem(page), "{page:?}");
    }
    assert!(is_plain_stem("x"));
    assert!(is_plain_stem("Meeting notes 2026.11"));
}

#[test]
fn reading_a_folder_lists_its_markdown_and_text_files_only() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "Ideas.md",
        "todo.txt",
        "photo.png",
        ".draft.md",
        "notes.md.bak",
    ] {
        fs::write(dir.path().join(name), "x").unwrap();
    }
    fs::create_dir(dir.path().join("Folder.md")).unwrap();
    let index = PageIndex::read(dir.path()).unwrap();
    assert_eq!(index.len(), 2);
    assert_eq!(
        index.resolve("ideas"),
        Some(dir.path().join("Ideas.md").as_path())
    );
    assert_eq!(
        index.resolve("TODO"),
        Some(dir.path().join("todo.txt").as_path())
    );
    assert_eq!(index.resolve("folder"), None);
    assert!(PageIndex::read(&dir.path().join("missing")).is_err());
    assert!(is_page_path(Path::new("a.MD")) && !is_page_path(Path::new(".a.md")));
}
