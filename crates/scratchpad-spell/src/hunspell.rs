//! Hunspell dictionaries, as Linux distributions install them for LibreOffice, Firefox and
//! GTK's spell checkers (`hunspell-en-us` and the like), read by `spellbook`.
//!
//! There is no autocorrect list here, only suggestions. Words the user adds go to a plain word
//! list of the app's own, one word per line, as Hunspell's personal dictionaries do.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use spellbook::Dictionary;

use crate::{Action, Backend, Misspelling, repeats, words};

/// Used when the user's own language has no dictionary installed.
const FALLBACK_LANGUAGE: &str = "en_US";
const MAX_SUGGESTIONS: usize = 6;

pub struct HunspellChecker {
    dictionary: Dictionary,
    ignored: HashSet<String>,
    /// Where added words are kept; `None` keeps them for this run only.
    personal: Option<PathBuf>,
}

impl HunspellChecker {
    /// The dictionary for the user's language (or US English) from the usual folders.
    pub fn from_system(personal: Option<PathBuf>) -> Option<Self> {
        let languages = [user_language(), Some(FALLBACK_LANGUAGE.to_owned())];
        let (aff, dic) = languages.iter().flatten().find_map(|language| {
            dictionary_dirs()
                .into_iter()
                .map(|dir| {
                    (
                        dir.join(format!("{language}.aff")),
                        dir.join(format!("{language}.dic")),
                    )
                })
                .find(|(aff, dic)| aff.is_file() && dic.is_file())
        })?;
        let checker = Self::open(&aff, &dic, personal)?;
        tracing::info!(dictionary = %dic.display(), "spell checking with Hunspell");
        Some(checker)
    }

    /// Opens a dictionary from its `.aff` and `.dic` files and adds the personal words.
    pub fn open(aff: &Path, dic: &Path, personal: Option<PathBuf>) -> Option<Self> {
        let (aff_text, dic_text) = (read_text(aff)?, read_text(dic)?);
        let mut dictionary = Dictionary::new(&aff_text, &dic_text)
            .inspect_err(
                |error| tracing::warn!(%error, path = %aff.display(), "invalid dictionary"),
            )
            .ok()?;
        if let Some(words) = personal
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok())
        {
            for word in words.lines().map(str::trim).filter(|word| !word.is_empty()) {
                dictionary.add(word).ok();
            }
        }
        Some(Self {
            dictionary,
            ignored: HashSet::new(),
            personal,
        })
    }

    fn knows(&self, word: &str) -> bool {
        // Dictionaries spell "it's" with a straight apostrophe; typography often curls it.
        let word = word.replace('\u{2019}', "'");
        self.ignored.contains(&word) || self.dictionary.check(&word)
    }
}

impl Backend for HunspellChecker {
    fn check(&mut self, text: &str) -> Vec<Misspelling> {
        let mut found = Vec::new();
        let mut previous: Option<(usize, &str)> = None;
        for (start, word) in words(text) {
            let range = start..start + word.len();
            if !self.knows(word) {
                found.push(Misspelling {
                    range,
                    action: Action::Suggest,
                });
            } else if previous
                .is_some_and(|(end, previous)| repeats(text, previous, end, word, start))
            {
                found.push(Misspelling {
                    range,
                    action: Action::Delete,
                });
            }
            previous = Some((start + word.len(), word));
        }
        found
    }

    fn suggest(&mut self, word: &str) -> Vec<String> {
        let mut suggestions = Vec::new();
        self.dictionary
            .suggest(&word.replace('\u{2019}', "'"), &mut suggestions);
        suggestions.truncate(MAX_SUGGESTIONS);
        suggestions
    }

    fn add(&mut self, word: &str) {
        self.dictionary.add(word).ok();
        let Some(path) = &self.personal else {
            return;
        };
        let written = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::File::options().create(true).append(true).open(path))
            .and_then(|mut file| writeln!(file, "{word}"));
        if let Err(error) = written {
            tracing::warn!(%error, path = %path.display(), "could not save an added word");
        }
    }

    fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.replace('\u{2019}', "'"));
    }
}

/// The user's language from the locale variables, e.g. `en_GB` from `LANG=en_GB.UTF-8`.
fn user_language() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .map(|value| {
            value
                .split(['.', '@'])
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .filter(|language| language != "C" && language != "POSIX")
}

/// Where dictionaries are installed: `$DICPATH` first, as Hunspell itself looks, then the
/// user's and the system's folders.
fn dictionary_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("DICPATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    {
        dirs.push(data.join("hunspell"));
    }
    dirs.extend(
        [
            "/usr/share/hunspell",
            "/usr/share/myspell",
            "/usr/share/myspell/dicts",
            "/usr/local/share/hunspell",
            // Flatpak runtimes
            "/app/share/hunspell",
            "/usr/share/hunspell-dictionaries",
        ]
        .map(PathBuf::from),
    );
    dirs
}

/// A dictionary file's text. Older dictionaries are Latin-1 rather than UTF-8; each of its
/// bytes is the character with that number.
fn read_text(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => error.into_bytes().into_iter().map(char::from).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFF: &str = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwz'\n\nSFX S Y 1\nSFX S 0 s .\n";
    const DIC: &str = "6\ncat/S\nthe\nsat\non\nmat/S\nit's\n";

    fn checker(dir: &Path) -> HunspellChecker {
        let (aff, dic) = (dir.join("xx.aff"), dir.join("xx.dic"));
        fs::write(&aff, AFF).unwrap();
        fs::write(&dic, DIC).unwrap();
        HunspellChecker::open(&aff, &dic, Some(dir.join("personal").join("words.txt"))).unwrap()
    }

    #[test]
    fn unknown_and_repeated_words_are_found() {
        let dir = std::env::temp_dir().join(format!("scratchpad-spell-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut checker = checker(&dir);
        assert_eq!(
            checker.check("the the cats sat on teh mat, it\u{2019}s"),
            [
                Misspelling {
                    range: 4..7,
                    action: Action::Delete
                },
                Misspelling {
                    range: 20..23,
                    action: Action::Suggest
                },
            ]
        );
        assert!(checker.suggest("teh").iter().any(|s| s == "the"));

        checker.add("teh");
        checker.ignore("zzz");
        assert_eq!(checker.check("teh zzz"), []);
        // Added words are there again next time; ignored ones are not.
        let mut again = self::checker(&dir);
        assert_eq!(again.check("teh zzz").len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn latin1_dictionaries_are_read() {
        let dir = std::env::temp_dir().join(format!("scratchpad-latin1-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latin1.dic");
        fs::write(&path, b"1\nna\xefve\n").unwrap();
        assert_eq!(read_text(&path).unwrap(), "1\nna\u{ef}ve\n");
        fs::remove_dir_all(&dir).ok();
    }
}
