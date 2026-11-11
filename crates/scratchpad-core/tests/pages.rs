//! Pages wikilinks resolve to: stems ignoring case as NTFS does, `.md` before `.txt`, and names that cannot leave
//! the folder.

use std::fs;
use std::path::{Path, PathBuf};

use scratchpad_core::{NoteStore, PageIndex, fold_case, is_page_path};

fn index(names: &[&str]) -> PageIndex {
    let dir = Path::new("C:/notes");
    PageIndex::from_paths(dir, names.iter().map(|name| dir.join(name)))
}

fn at(name: &str) -> Option<PathBuf> {
    Some(Path::new("C:/notes").join(name))
}

#[test]
fn pages_resolve_by_stem_ignoring_case_and_prefer_markdown() {
    let index = index(&[
        "Ideas.md",
        "ideas.txt",
        "Todo.TXT",
        "Log.txt",
        "Café.md",
        "image.png",
        ".hidden.md",
    ]);
    assert_eq!(index.resolve("ideas"), at("Ideas.md"));
    assert_eq!(index.resolve("IDEAS"), at("Ideas.md"));
    assert_eq!(index.resolve("todo"), at("Todo.TXT"));
    assert_eq!(index.resolve("log"), at("Log.txt"));
    assert_eq!(index.resolve("CAFÉ"), at("Café.md"));
    assert!(index.has_page("café") && !index.has_page("cafe"));
    assert_eq!(index.resolve("image"), None);
    assert_eq!(index.resolve(".hidden"), None);
    assert_eq!(index.resolve("Missing"), None);
    assert_eq!(index.len(), 4);
}

#[test]
fn case_is_folded_char_by_char_as_ntfs_does() {
    let index = index(&["ΟΔΟΣ.md", "İstanbul.md", "Straße.md", "ǅ.md"]);
    // `str::to_lowercase` turns a final capital sigma into `ς`, so it would match neither of these.
    assert_eq!(index.resolve("οδοσ"), at("ΟΔΟΣ.md"));
    assert_eq!(index.resolve("οδος"), at("ΟΔΟΣ.md"));
    // `İ` has no single-char lower case and upper-cases to itself, so `i` does not match it.
    assert_eq!(index.resolve("İSTANBUL"), at("İstanbul.md"));
    assert_eq!(index.resolve("istanbul"), None);
    // `ß` upper-cases to two chars, so it keeps its own value: `SS` is a different name.
    assert_eq!(index.resolve("STRAßE"), at("Straße.md"));
    assert_eq!(index.resolve("STRASSE"), None);
    assert_eq!(index.resolve("ǆ"), at("ǅ.md"));
    assert_eq!(fold_case("ΟΔΟΣ"), fold_case("οδος"));
    assert_ne!(fold_case("ß"), fold_case("SS"));
}

#[test]
fn insert_and_remove_keep_the_preferred_file() {
    let dir = Path::new("C:/notes");
    let mut index = PageIndex::new(dir);
    assert!(index.insert(&dir.join("Plan.txt")));
    assert!(!index.insert(&dir.join("Plan.txt")));
    assert!(
        !index.insert(&dir.join("PLAN.txt")),
        "the same file on Windows"
    );
    assert!(index.contains(&dir.join("Plan.txt")) && !index.contains(&dir.join("Plan.md")));
    assert!(!index.insert(&dir.join("photo.jpg")));
    assert_eq!(index.resolve("plan"), at("Plan.txt"));
    assert!(index.insert(&dir.join("plan.md")));
    assert_eq!(index.resolve("Plan"), at("plan.md"));
    assert!(index.remove(&dir.join("plan.md")));
    assert!(!index.remove(&dir.join("plan.md")));
    assert!(index.has_page("PLAN"));
    assert_eq!(
        index.resolve("Plan"),
        at("plan.txt"),
        "spelled as the preferred file was"
    );
    assert!(index.remove(&dir.join("Plan.txt")));
    assert!(index.is_empty());
}

#[test]
fn clones_share_the_listed_pages_and_keep_their_own_changes() {
    let dir = Path::new("C:/notes");
    let names: Vec<_> = (0..1000)
        .map(|n| dir.join(format!("Note {n}.md")))
        .collect();
    let listed = PageIndex::from_paths(dir, &names);
    let mut changed = listed.clone();
    // Enough changes to be merged into a copy of the shared map along the way.
    for n in 0..200 {
        assert!(changed.remove(&dir.join(format!("note {n}.md"))));
        assert!(changed.insert(&dir.join(format!("New {n}.txt"))));
    }
    assert_eq!(listed.len(), 1000);
    assert!(listed.has_page("Note 0") && !listed.has_page("New 0"));
    assert_eq!(changed.len(), 1000);
    assert!(!changed.has_page("Note 0") && !changed.has_page("note 199"));
    assert!(changed.has_page("Note 200") && changed.has_page("new 0"));
    assert_eq!(changed.resolve("NEW 199"), at("New 199.txt"));
    // Adding back what is listed leaves nothing pending.
    let mut back = listed.clone();
    assert!(back.remove(&dir.join("Note 5.md")) && back.insert(&dir.join("Note 5.md")));
    assert_eq!(back.len(), 1000);
    assert_eq!(back.resolve("note 5"), at("Note 5.md"));
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
    assert_eq!(index.resolve("ideas"), Some(dir.path().join("Ideas.md")));
    assert_eq!(index.resolve("TODO"), Some(dir.path().join("todo.txt")));
    assert_eq!(index.resolve("folder"), None);
    assert!(PageIndex::read(&dir.path().join("missing")).is_err());
    assert!(is_page_path(Path::new("a.MD")) && !is_page_path(Path::new(".a.md")));

    // The store lists the same pages along with its notes, in one pass.
    let store = NoteStore::open(dir.path()).unwrap();
    let (notes, pages) = store.list_with_pages().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages.resolve("todo"), index.resolve("todo"));
    assert_eq!(pages.dir(), dir.path());
}
