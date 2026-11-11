# 0180. Typed sequences become symbols, and Backspace takes them back
Date: 2026-11-11
Status: Accepted; amends ADR 0044, 0150 and 0170

## Context
Issue #4 asks for `->` to become → as it is typed, with a dozen more sequences, the way Notion and
Word do it: on the last character, taken back by Backspace, left alone in code. Markdown makes this
harder than in a word processor: `---` is a thematic break and a front matter fence, `<!--` opens a
comment, `<...>` is an autolink, and notes are full of addresses, paths and command lines. Typing
already has rules of its own (bracket pairing and wrapping, ADR 0044; autocorrect, ADR 0170), which
must keep working.

## Decision
- **The table** is data, `scratchpad_editor::replacements::RULES`: `->` →, `<-` ←, `<->` ↔, `=>` ⇒,
  `<=` ≤, `<=>` ⇔, `>=` ≥, `!=` ≠, `--` —, `...` …, `(c)` ©, `(r)` ®, `(tm)` ™, `+-` ±. Each rule has
  a guard for the conditions only it has. `<=>` is not in the issue's table; see below.
- **Where.** Only in `Editor::insert_typed`, the path for single keystrokes in Markdown notes, and
  only with no selection and no IME composition. Paste, IME commits, programmatic edits
  (`insert_text`, `replace_range`) and plain text files (ADR 0147) never replace. Typing over a
  selection wraps or replaces it as before (ADR 0044).
- **When.** After the typed char is in, or after a `)` stepped over the paired closer (so `(c)`
  typed with pairing works), the text before the cursor is matched against the table; the longest
  rule it ends with whose guards pass wins. Only chars that end a rule (`>`, `-`, `=`, `.`, `)`)
  look.
- **Undo and Backspace.** The replacement is its own undo step after the typing, so `Ctrl+Z` right
  after it gives back `->`, and the next typing is a new step. The editor remembers the last
  replacement (its range, its rule, the buffer version after it). While the cursor is right after
  it and nothing changed, Backspace puts the original text back in one step (an edit, which `Ctrl+Z`
  undoes to the symbol again). The next char typed after that replaces nothing, so `--`, taken
  back, then `-` gives `---`; turning the setting off and on in between does not change that. Moving
  the cursor, any other edit, or turning the feature off forgets the last replacement. So does undo:
  after undo and redo, `←` is just text, and `>` typed after it gives `←>` (redo does not bring back
  the memory of what it replaced; Backspace then deletes as usual).
- **Longer rules after a shorter one (`<-` then `>`).** Holding `<-` back until the next key would
  leave the arrow unreplaced at the end of a line, so `<-` is replaced at once. When the next char
  would make a longer rule out of the original text (`<-` + `>` is `<->`), the replacement is taken
  back first, by undoing it, and the char is typed onto `<-`; `<->` then becomes ↔. Undo after it
  gives `<->`, not `←>`. `<=` then `>` works the same way: `<=>` → ⇔ is added to the table, the
  double arrow as `<->` is the single one, rather than leaving `≤>` or holding `≤` back. (`<=>` as
  the comparison operator is code, where nothing is replaced.) A replacement made inside an explicit
  transaction is not the last undo step, so it is put back by replacing the text instead.
- **Exclusions**, besides the ones above:
  - Code: `MarkdownState::in_code` over the matched text (code spans, fenced and indented blocks,
    unclosed fences), and a code span still open before the match on its line: a run of backticks
    with no run as long after it, as CommonMark pairs them, not counting backslash-escaped ones
    outside a span. Up to 16 KB of the line before the match is read for this.
  - Front matter, closed or still being typed, as `markdown::front_matter` finds it (shared with
    spell checking, ADR 0170): a `---` first line whose next line is `key:` or a closing line. It is
    closed by a `---` or `...` line; while there is none above the cursor, it lasts only while its
    lines look like front matter (`key:` lines, indented lines, `- ` items, `#` comments), so a
    leading `---` rule followed by a paragraph does not stop replacements in the note. At most
    1,000 lines and 64 KB are read, in one piece.
  - HTML comments: nothing is replaced when a `<!--` comes after the last `-->` in the 4 KB before
    the match, nor in the `--` that completes `<!--`. So neither `<!--` nor `-->` breaks.
  - Addresses, paths, autolinks and tags: not when the run of non-whitespace the match ends holds
    `/`, `\` or `@` (so `://` too) or a `](` with no `)` after it (a link destination being
    typed), or starts with `www.` or with `<` and a letter. This keeps `...` and `--` in URLs,
    paths and destinations such as `[draft](notes--v2.md)`, and `->` in `<https://…>`.
  - `--` (guard `Dash`): held off while the line up to the cursor is only `-`, `|`, `:` and
    whitespace, since it can still become `---` (a thematic break, a front matter fence or a setext
    underline) or a table's delimiter row. Once a char other than a dash or whitespace follows a held
    `--` and a space on a line of dashes and spaces, the line can no longer be a rule, and the `--`
    becomes a dash then, as its own undo step: `- -- item` and `-- signed` give `- — item` and
    `— signed`. So a held `--` on a line of only `-`, `|`, `:` and whitespace stays as typed (a
    table's delimiter row, `| -- |`), but after text in a table row, `--` becomes a dash
    (`| a | --` gives `| a | —`).
  - `--flag`: a `--` that starts a word (after nothing, whitespace or an opening bracket or quote)
    is taken back when a letter or digit is typed right after it, so `--verbose` and `--2` type as
    they are while `well--yes` and ` -- ` become dashes.
  - `(c)`, `(r)`, `(tm)` (guard `Symbol`): only at the start of a word, so a call `f(c)` or a link
    `[x](c)` stays. `(c)` and `(r)` are also kept when `(b)` or `(q)` comes before them on their line
    or the line before: an enumeration `(a) … (b) … (c)`.
  - `+-` (guard `NotAfter('+')`): not right after a `+`, so `C++-style` stays.
- **Autocorrect.** Both act on keystrokes and each makes its own undo step. Autocorrect asks about a
  word after a space or `.,;:!?` and corrects it only if the text is unchanged when the answer comes,
  and its correction moves the cursor, which forgets the last replacement. So `teh...` gives
  `the…` (the first `.` corrects, the third replaces), Backspace then gives `the...`, and neither
  ever rewrites the other's text. If the checker answered only after the dots were replaced, the
  correction is skipped, as for any edit in between.
- **Setting.** View > Auto-replace symbols, on by default, remembered in the config as
  `auto_replace` (missing means on), with no key, like Autocorrect. The engine holds the flag
  (`Editor::set_typed_replacements`) and the view carries it to each note it opens.

## Consequences
- The state is small and checked against the buffer version, so undo, redo, other edits and cursor
  moves cannot leave a stale replacement for Backspace to "take back".
- A keystroke that can end a rule reads a few hundred bytes around the cursor and asks the Markdown
  structure about code (last, as the dearest check): 15-45 µs for typing `->` and replacing it in a
  10 MB note, ~35 µs in front matter 1,000 lines long (docs/performance.md). Other keystrokes are
  unchanged.
- The comment check looks back 4 KB only, and does not know about code: a `<!--` in a code span
  within that distance stops replacements until a `-->`.
- A held `--` is told only by the text, so a `-- ` pasted on a line of its own also becomes a dash
  when a letter is typed after it.
- Heuristics can miss: `-->` typed in prose becomes `—>`, a `--` cell in a table body row of
  dashes only stays `--`, `(c)` in some other enumeration is
  replaced, and a `/` in a word (`and/or...`) keeps the dots. Backspace fixes each in one key.
- Unit tests cover the pure matching (`find`, `in_html_comment`); the editor tests type through
  `insert_typed` (every rule, longest match, Backspace, undo, every exclusion, the setting) and the
  property tests type the trigger chars; end-to-end tests cover the window, paste, plain text
  files, autocorrect together with it, and the View menu setting across a restart.
