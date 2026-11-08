# 0170. Spell checking with the system's checker
Date: 2026-11-06
Status: Accepted

## Context
Notes are prose, and typos in them go unnoticed. Every Windows app that checks spelling (Edge,
Word, Notepad since 2023) uses the user's language, dictionary and autocorrect list from Windows,
so a word added in one is known in all. Bundling a dictionary would duplicate that and still
miss the user's words.

## Decision
- A new crate, `scratchpad-spell`, with no GPUI and used only by the app (amending ADR 0001):
  a `Backend` trait (check text, suggest, add,
  ignore) and two backends.
  - **Windows**: the Spell Checking API (`ISpellChecker`, Windows 8 and later) through the
    `windows` crate GPUI already uses; only two binding features are new. The language is the
    user's locale, else US English. Its misspellings come with Windows' own corrective action:
    suggest, replace (the autocorrect list, "teh" → "the") or delete (a repeated word). Added
    words go to Windows' user dictionary. The COM calls are the crate's only `unsafe` code, in
    one module.
  - **Elsewhere**: Hunspell dictionaries from `$DICPATH`, `~/.local/share/hunspell` and the
    usual system folders (`hunspell-en-us` and the like), for the language in `LC_ALL`,
    `LC_MESSAGES` or `LANG`, read by `spellbook`, a pure-Rust Hunspell reader. Nothing to build
    or ship, no C library at run time. Hunspell has no autocorrect list, so there it only
    suggests. Added words go to `dictionary.txt` next to the config file, one per line.
  - Without a checker or dictionary, nothing is underlined and View > Spell check is disabled.
- A `Speller` runs the backend on a thread of its own: the COM object must stay on the thread
  that made it, and checking never holds up typing. Requests go over `flume` channels (already
  in the tree through GPUI), which the UI awaits. A newer check supersedes one in progress
  between its segments; single-word requests are answered between segments too.
- What is checked: the prose of the lines on screen and 100 lines either side (at most 256 KB),
  300 ms after typing stops, and again when scrolling reaches lines not yet checked. Code blocks,
  inline code, link addresses, autolinks, bare `https://` and `www.` addresses and Markdown
  markers are left out, using the document's Markdown decorations (ADR 0040). Words with
  digits, underscores, dots, slashes or `@`, and words in capitals (abbreviations) are never
  flagged. Plain text files (ADR 0147) are not checked: they are logs and settings files.
- Misspellings follow edits like find matches (ADR 0100); an edit drops the words it touches
  until the next check. The word at the cursor while it is being typed is not underlined.
- Shown as a wavy underline in the new `misspelling` theme token, red in both themes, below the
  baseline and as thick as the zoom makes the text.
- A right-click on an underlined word opens a menu: the suggestions (or the autocorrect
  replacement, or "Delete repeated word"), then Add to dictionary and Ignore (until the app
  quits). A correction is one undo step. A read-only note offers only Add and Ignore.
  Right-clicks elsewhere still do nothing.
- Autocorrect: typing a space or `.,;:!?` after a prose word that the checker has a sure
  replacement for corrects it, as its own undo step, so Ctrl+Z right after brings back what was
  typed. The UI waits at most 40 ms for the answer (well under a millisecond in practice), and
  a late answer only skips the correction.
- View > Spell check and View > Autocorrect turn them off, are remembered in the config
  (`spell_check`, `autocorrect`, both on when missing), and have no keys.

## Consequences
- Startup is unchanged: the checker is made on its thread after the window opens.
- Only one language at a time, the user's. Windows users change it in Windows' settings.
- There is no keyboard way to open the corrections menu yet, nor a "next misspelling" command.
- The end-to-end tests use a word list in place of the system checker; the Windows backend
  has its own test against the real API, and the Hunspell backend one against a small dictionary.
