# 0170. Spell checking with the system's checker
Date: 2026-11-06
Status: Accepted; amended by 0175, 0180

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
    `windows` crate GPUI already uses; only three binding features are new. The language is
    the first of the user's languages (Settings > Time & language > Language & region, read
    with `GlobalizationPreferences.Languages`) that Windows has a checker for, else the
    regional format's, else US English. Not the display language
    (`GetUserPreferredUILanguages`): that is often US English alone, for users who write in
    another. Its misspellings come with Windows' own corrective action:
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
  in the tree through GPUI), which the UI awaits; it never blocks on one. Each check names its
  requester (the editor view): a newer check from the same view supersedes one waiting or in
  progress, between its segments, and the older one is answered `Superseded`; checks from
  other windows wait their turn instead, so two windows cannot keep cancelling each other.
  Single-word requests are answered between segments. If the thread ends (the checker
  panicked), the next reply finds its channel closed: the checker is then unavailable for the
  rest of the run, logged once, and nothing more is sent to it.
- What is checked: the prose of the lines on screen and 100 lines either side, 300 ms after
  typing stops, and again when scrolling reaches lines not yet checked. At most 256 KB of text
  is read, even when one line on screen is longer (the rest of it is not checked), and runs of
  prose go to the checker in pieces of at most 16 KB, cut at a space. Front matter (from a
  `---` first line to the next `---` or `...` line; amended by ADR 0180: only when the second
  line is a `key:` line or the closing one, so prose between two rules is checked, and found by
  `markdown::front_matter`, shared with typed replacements), code blocks, inline code, link addresses,
  autolinks, bare `https://` and `www.` addresses and Markdown markers are left out, using the
  document's Markdown decorations (ADR 0040). Words with digits, underscores or dots, words in
  capitals (abbreviations), and any word in a run without spaces that holds `@`, `/` or `\`
  (e-mail addresses, paths) are never flagged, whichever backend split the text. Plain text files (ADR 0147) are not checked: they are logs and settings files.
- Misspellings follow edits like find matches (ADR 0100); an edit drops the words it touches
  until the next check. The word at the cursor while it is being typed is not underlined.
- Shown as a wavy underline in the new `misspelling` theme token, red in both themes, below the
  baseline and as thick as the zoom makes the text.
- A right-click on an underlined word opens a menu: the suggestions (or the autocorrect
  replacement, or "Delete repeated word"), then Add to dictionary and Ignore (until the app
  quits). A correction is one undo step. A read-only note offers only Add and Ignore.
  Right-clicks elsewhere still do nothing. (Amended by ADR 0175: these are now the first
  section of the note's context menu, which every right-click, Shift+F10 and the Menu key open.)
- Autocorrect: typing a space or `.,;:!?` after a prose word asks the checker about it, and
  typing goes on. If the answer is a sure replacement and the note has not changed since, the
  word is corrected, as its own undo step, so Ctrl+Z right after brings back what was typed.
  The answer takes well under a millisecond, so in practice it comes before the next
  keystroke; a later one only skips the correction. Nothing is asked while the checker is
  still starting.
- View > Spell check and View > Autocorrect turn them off, are remembered in the config
  (`spell_check`, `autocorrect`, both on when missing), and have no keys.

## Consequences
- Startup is unchanged: `spelling::init` starts the thread before the window is made, but the
  checker (a COM object, or a dictionary to read) is made on that thread, so the first frame
  does not wait for it; checks sent meanwhile wait.
- Only one language at a time, the user's. Windows users change it in Windows' settings.
  Picking the language in Scratchpad's settings, which the issue asked for, is deferred: it
  needs a list of the languages the system has checkers for and a setting to keep the choice.
- There is no "next misspelling" command. (The corrections open from the keyboard since ADR
  0175: Shift+F10 or the Menu key on the word.)
- The end-to-end tests use a word list in place of the system checker, which can hold its
  answers back to play a busy checker (replies after an edit, two windows' checks). Unit tests
  cover pure functions only (which text is prose, how misspellings follow edits); the Windows
  backend has its own test against the real API, and the Hunspell backend one against a small
  dictionary.
