//! The Windows Spell Checking API (`ISpellChecker`, Windows 8 and later): the checker Windows
//! itself uses, with the user's dictionary and the system's autocorrect list.
//!
//! COM calls are `unsafe` by the bindings' design; this module is the only place that makes
//! them. Everything here runs on the spell-check thread, which joins the multithreaded
//! apartment once and for good.
#![allow(unsafe_code)]

use windows::System::UserProfile::GlobalizationPreferences;
use windows::Win32::Globalization::{
    CORRECTIVE_ACTION_DELETE, CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE,
    GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use crate::{Action, Backend, Misspelling};

/// Used when none of the user's languages has a dictionary installed.
const FALLBACK_LANGUAGE: &str = "en-US";
/// Suggestions offered for a word; Windows lists about this many itself.
const MAX_SUGGESTIONS: usize = 6;

pub struct WindowsChecker {
    checker: ISpellChecker,
}

impl WindowsChecker {
    /// The checker for the user's language (or US English), if Windows has one.
    pub fn new() -> Option<Self> {
        // S_FALSE (already initialised) is fine too.
        if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
            tracing::warn!(%error, "could not initialise COM for the spell checker");
            return None;
        }
        let factory: ISpellCheckerFactory =
            match unsafe { CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER) } {
                Ok(factory) => factory,
                Err(error) => {
                    tracing::warn!(%error, "the Windows spell checker is not available");
                    return None;
                }
            };
        let language = user_languages()
            .into_iter()
            .chain(user_locale())
            .chain([FALLBACK_LANGUAGE.to_owned()])
            .find(|language| {
                let tag = HSTRING::from(language.as_str());
                unsafe { factory.IsSupported(PCWSTR(tag.as_ptr())) }
                    .is_ok_and(|supported| supported.as_bool())
            })?;
        let tag = HSTRING::from(language.as_str());
        let checker = unsafe { factory.CreateSpellChecker(PCWSTR(tag.as_ptr())) }
            .inspect_err(
                |error| tracing::warn!(%error, language, "could not create a spell checker"),
            )
            .ok()?;
        tracing::info!(language, "spell checking with Windows");
        Some(Self { checker })
    }
}

impl Backend for WindowsChecker {
    fn check(&mut self, text: &str) -> Vec<Misspelling> {
        let wide = HSTRING::from(text);
        let Ok(errors) = (unsafe { self.checker.Check(PCWSTR(wide.as_ptr())) }) else {
            return Vec::new();
        };
        // Windows counts UTF-16 units; the app counts bytes.
        let bytes = utf16_to_bytes(text);
        let mut found = Vec::new();
        loop {
            let mut error = None;
            if unsafe { errors.Next(&mut error) }.is_err() {
                break;
            }
            // S_FALSE, the end of the list, also comes back without an error.
            let Some(error) = error else {
                break;
            };
            let fields = unsafe { (error.StartIndex(), error.Length(), error.CorrectiveAction()) };
            let (Ok(start), Ok(length), Ok(action)) = fields else {
                continue;
            };
            let (start, end) = (start as usize, start as usize + length as usize);
            let (Some(&start), Some(&end)) = (bytes.get(start), bytes.get(end)) else {
                continue;
            };
            let action = match action {
                CORRECTIVE_ACTION_GET_SUGGESTIONS => Action::Suggest,
                CORRECTIVE_ACTION_REPLACE => match unsafe { error.Replacement() } {
                    Ok(replacement) => Action::Replace(take_string(replacement)),
                    Err(_) => Action::Suggest,
                },
                CORRECTIVE_ACTION_DELETE => Action::Delete,
                _ => continue,
            };
            found.push(Misspelling {
                range: start..end,
                action,
            });
        }
        found
    }

    fn suggest(&mut self, word: &str) -> Vec<String> {
        let wide = HSTRING::from(word);
        let Ok(suggestions) = (unsafe { self.checker.Suggest(PCWSTR(wide.as_ptr())) }) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        while found.len() < MAX_SUGGESTIONS {
            let mut next = [PWSTR::null()];
            let mut fetched = 0;
            let result = unsafe { suggestions.Next(&mut next, Some(&mut fetched)) };
            if result.is_err() || fetched == 0 {
                break;
            }
            found.push(take_string(next[0]));
        }
        found
    }

    fn add(&mut self, word: &str) {
        let wide = HSTRING::from(word);
        if let Err(error) = unsafe { self.checker.Add(PCWSTR(wide.as_ptr())) } {
            tracing::warn!(%error, "could not add a word to the dictionary");
        }
    }

    fn ignore(&mut self, word: &str) {
        let wide = HSTRING::from(word);
        unsafe { self.checker.Ignore(PCWSTR(wide.as_ptr())) }.ok();
    }
}

/// The user's languages, most preferred first, as BCP 47 tags (`en-IN`): the list in Settings >
/// Time & language > Language & region, which Windows' own apps check spelling in. It is not
/// the display language (`GetUserPreferredUILanguages`), which is often only US English.
fn user_languages() -> Vec<String> {
    GlobalizationPreferences::Languages()
        .map(|languages| languages.into_iter().map(|tag| tag.to_string()).collect())
        .inspect_err(|error| tracing::warn!(%error, "could not read the user's languages"))
        .unwrap_or_default()
}

/// The locale of the user's regional format, e.g. `en-GB`: a guess at their language when
/// none of their languages has a checker.
fn user_locale() -> Option<String> {
    // LOCALE_NAME_MAX_LENGTH
    let mut name = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(&mut name) };
    // The length counts the terminating NUL; 0 is failure.
    let length = usize::try_from(length).ok()?.checked_sub(1)?;
    String::from_utf16(&name[..length]).ok()
}

/// Copies a string COM allocated for us, and frees it.
fn take_string(string: PWSTR) -> String {
    if string.is_null() {
        return String::new();
    }
    let text = unsafe { string.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(string.0 as *const _)) };
    text
}

/// The byte offset of each UTF-16 unit of `text`, and of its end. A unit inside a character
/// (the second of a surrogate pair) maps to the character's start.
fn utf16_to_bytes(text: &str) -> Vec<usize> {
    let mut bytes = Vec::with_capacity(text.len() + 1);
    for (byte, c) in text.char_indices() {
        bytes.extend(std::iter::repeat_n(byte, c.len_utf16()));
    }
    bytes.push(text.len());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_units_map_to_byte_offsets() {
        // "é" is 2 bytes / 1 unit, "😀" 4 bytes / 2 units.
        assert_eq!(utf16_to_bytes("aé😀b"), [0, 1, 3, 3, 7, 8]);
    }

    /// Needs Windows' English dictionary, which every Windows 10 and 11 install has.
    #[test]
    fn windows_finds_misspellings_and_suggests_words() {
        let Some(mut checker) = WindowsChecker::new() else {
            eprintln!("no Windows spell checker here; skipped");
            return;
        };
        let found = checker.check("This sentance has a mistake.");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].range, 5..13);
        assert!(checker.suggest("sentance").iter().any(|s| s == "sentence"));
        // The autocorrect list, where the language has one (US English, for one, has none),
        // and repeated words, as the app's word list for tests has them.
        let found = checker.check("recieve");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].range, 0..7);
        assert!(
            matches!(&found[0].action, Action::Suggest)
                || found[0].action == Action::Replace("receive".into()),
            "{found:?}"
        );
        assert_eq!(
            checker.check("the the cat"),
            [Misspelling {
                range: 4..7,
                action: Action::Delete
            }]
        );
    }
}
