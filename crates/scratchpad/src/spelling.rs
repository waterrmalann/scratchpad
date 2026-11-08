//! The app's spell checker (ADR 0170): the system's, started once at launch on a thread of its
//! own and shared by every window.

use std::path::Path;

use gpui::{App, Global};
use scratchpad_spell::Speller;

struct Spelling(Speller);

impl Global for Spelling {}

/// Starts the system's spell checker. Outside Windows, which keeps its own, words the user adds
/// go to `dictionary.txt` in `config_dir`.
pub fn init(config_dir: Option<&Path>, cx: &mut App) {
    let personal = config_dir.map(|dir| dir.join("dictionary.txt"));
    set_speller(
        Speller::start(move || scratchpad_spell::system_backend(personal)),
        cx,
    );
}

/// Makes `speller` the app's spell checker; tests give it a word list of their own.
pub fn set_speller(speller: Speller, cx: &mut App) {
    cx.set_global(Spelling(speller));
}

/// Whether there is a spell checker for the user's language, or may be: while it starts, the
/// menu offers it.
pub fn is_available(cx: &App) -> bool {
    speller(cx).is_some_and(|speller| speller.available() != Some(false))
}

/// The app's spell checker, if one was started; without it nothing is checked.
pub fn speller(cx: &App) -> Option<Speller> {
    cx.try_global::<Spelling>()
        .map(|spelling| spelling.0.clone())
}
