# Architecture decision records

One file per decision, named `NNNN-title.md`, with a context, the decision and its consequences. The numbers
leave gaps between topics on purpose, so a later decision can sit next to the ones it relates to. Add a row here
with every new ADR.

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-workspace-layout.md) | Three-crate workspace layout | Accepted |
| [0002](0002-rope-buffer-and-byte-offsets.md) | Rope buffer with byte offsets as the canonical coordinate | Accepted |
| [0003](0003-line-ending-normalization.md) | Normalize line endings to LF in the buffer | Accepted |
| [0004](0004-selection-goal-and-word-motions.md) | Single selection, goal column and word motions | Accepted |
| [0005](0005-undo-history-and-grouping.md) | Operation-based undo history and grouping rules | Accepted |
| [0006](0006-in-document-search-matching.md) | Case-insensitive in-document search on original text | Accepted |
| [0007](0007-editor-engine-benchmarks.md) | Editor engine benchmarks and budgets | Accepted |
| [0010](0010-atomic-write-strategy.md) | Atomic writes: temp file, sync, rename | Accepted |
| [0011](0011-delete-to-recycle-bin.md) | Delete moves notes to the OS recycle bin | Accepted |
| [0012](0012-in-memory-note-search.md) | Note search: in-memory substring scan with a small text cache | Accepted |
| [0013](0013-note-naming-and-encoding.md) | Note naming, collisions and text encoding | Accepted |
| [0014](0014-config-and-recovery-storage.md) | Config and recovery data live outside the notes folder as small JSON files | Accepted |
| [0020](0020-e2e-testing-strategy.md) | End-to-end testing strategy | Accepted |
| [0021](0021-build-profiles.md) | Build profiles and panic strategy | Accepted |
| [0022](0022-theme-and-typography.md) | Theme tokens and typography | Accepted |
| [0030](0030-virtualized-editor-rendering.md) | Virtualized editor rendering with an anchor scroll position | Accepted; amended by 0132 |
| [0031](0031-soft-wrapping-and-visual-navigation.md) | Soft wrapping and navigation by visual rows | Accepted; amended by 0126, 0130, 0132 |
| [0032](0032-editor-input-clipboard-and-ime.md) | Editor input: keys, clipboard, IME and mouse | Accepted |
| [0040](0040-markdown-decorations-with-pulldown-cmark.md) | Markdown decorations parsed with pulldown-cmark | Accepted |
| [0041](0041-block-regions-and-incremental-markdown-parsing.md) | Block regions with lazy, incremental Markdown parsing | Accepted |
| [0042](0042-live-preview-styled-lines-and-marker-reveal.md) | Styled lines and marker reveal for live preview | Accepted |
| [0043](0043-markdown-editing-commands.md) | Markdown editing commands edit the text | Accepted; amended by 0126 |
| [0044](0044-conservative-bracket-pairing.md) | Conservative, stateless bracket and quote pairing | Accepted |
| [0050](0050-notes-model-and-new-note-lifecycle.md) | Notes model, background listing and the new-note lifecycle | Accepted |
| [0051](0051-sidebar-note-list.md) | Sidebar note list: uniform virtual rows and iCloud-style interactions | Accepted; amended by 0135, 0136 |
| [0052](0052-single-line-text-input.md) | A minimal single-line text input | Accepted |
| [0053](0053-sidebar-search.md) | Sidebar search runs in the background and only the latest query wins | Accepted |
| [0054](0054-error-toasts.md) | File errors are shown as a transient toast | Accepted |
| [0060](0060-open-note-session-and-ordered-file-queue.md) | The open note's session and one ordered queue for its file operations | Accepted |
| [0061](0061-new-note-files-and-title-renames.md) | When new notes get a file and when files follow their title | Accepted |
| [0062](0062-autosave-timing-and-flush-points.md) | Autosave timing, flush points and failed saves | Accepted |
| [0063](0063-changes-by-other-programs.md) | Changes made to notes by other programs | Accepted |
| [0064](0064-crash-recovery-snapshots.md) | Crash recovery: snapshots of unsaved text | Accepted |
| [0065](0065-settings-persistence-and-startup-order.md) | Remembered settings and the startup order | Accepted |
| [0066](0066-notes-that-are-not-utf8.md) | Notes that are not valid UTF-8 open read-only until the user agrees | Accepted |
| [0070](0070-markdown-typography-in-the-editor-view.md) | Markdown typography in the editor view | Accepted; amended by 0130 |
| [0071](0071-live-preview-layout-and-column-mapping.md) | Live preview layout and column mapping | Accepted; amended by 0125 |
| [0072](0072-markdown-shortcuts-tasks-and-links.md) | Markdown shortcuts, task boxes and links | Accepted |
| [0080](0080-a-small-settings-panel.md) | A small settings panel with two settings | Accepted |
| [0081](0081-changing-the-notes-folder.md) | Changing the notes folder | Accepted |
| [0082](0082-config-dir-override.md) | SCRATCHPAD_CONFIG_DIR keeps trial runs away from the user's settings | Accepted |
| [0090](0090-app-icon-and-executable-resources.md) | App icon and executable resources | Accepted |
| [0091](0091-per-user-msi-installer.md) | Per-user MSI installer built with WiX | Accepted |
| [0092](0092-file-associations-deferred.md) | File associations are deferred until the app opens file arguments | Accepted |
| [0100](0100-find-in-the-open-note.md) | Find in the open note | Accepted |
| [0110](0110-startup-time-is-gpui-platform-init.md) | Startup time is GPUI's platform initialisation; load fonts alongside it | Accepted |
| [0111](0111-saves-and-recovery-snapshots-share-the-rope.md) | Saves and recovery snapshots share the rope instead of copying the text | Accepted |
| [0112](0112-undo-history-budget.md) | A size budget for the undo history | Accepted |
| [0113](0113-where-idle-memory-goes.md) | Where idle memory goes, and what we keep after use | Accepted |
| [0114](0114-measuring-and-guarding-performance.md) | Measuring performance and guarding against regressions | Accepted |
| [0120](0120-data-safety-review-and-accepted-risks.md) | Data safety before V1: what the session guarantees and the risks accepted | Accepted |
| [0125](0125-cursor-targets-settle-on-revealed-markers.md) | Cursor targets settle on the markers they reveal | Accepted |
| [0126](0126-keyboard-conventions-for-markdown-text.md) | Keyboard conventions for Markdown text | Accepted |
| [0130](0130-the-text-uses-the-full-window-width.md) | The text uses the full window width | Accepted |
| [0131](0131-zooming-the-note.md) | Zooming the note | Accepted |
| [0132](0132-turning-word-wrap-off.md) | Turning word wrap off | Accepted |
| [0135](0135-collapsible-sidebar-and-narrow-windows.md) | A collapsible sidebar that floats over the note in narrow windows | Accepted |
| [0136](0136-confirm-before-deleting-a-note.md) | Confirm before deleting a note | Accepted |
| [0137](0137-notepad-style-status-bar.md) | A Notepad-style status bar | Accepted |
| [0140](0140-replace-in-the-find-bar.md) | Replace in the find bar | Accepted |
| [0141](0141-go-to-line.md) | Go to line | Accepted |
