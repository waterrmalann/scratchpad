//! In-document search (PLAN §29, ADR 0100): the matches of the find bar's query, kept up to date
//! with the text, and stepping through them.

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use scratchpad_editor::search::{self, CaseSensitivity};
use scratchpad_editor::{Bias, ByteOffset, Selection};

use super::{Autoscroll, EditorView};

/// How long the query and the text must stay unchanged before they are searched again, so that
/// neither typing in the note nor typing the query waits for a search (PLAN rule 8).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);
/// Larger documents are copied and searched on the background executor: a 10 MB note takes
/// tens of milliseconds.
pub const BACKGROUND_SEARCH_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Next,
    Previous,
}

/// What the find bar shows about the matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindStatus {
    /// The index of the match that is selected, if any.
    pub current: Option<usize>,
    pub total: usize,
}

pub(super) struct Find {
    query: String,
    case: CaseSensitivity,
    /// Sorted, non-overlapping matches in buffer version `version`. Between searches they follow
    /// edits; after a query change they are the previous query's until the new search lands, so
    /// the highlights do not flicker while typing.
    pub(super) matches: Vec<Range<ByteOffset>>,
    version: u64,
    /// Whether a search has finished since the find bar opened.
    searched: bool,
    /// The query changed: select the first match from the cursor when the search lands.
    select_first: bool,
    /// Next or previous was asked for while a search was pending; done when it lands.
    pending_step: Option<Direction>,
    /// The pending search. Replacing it cancels the previous one, so only the latest query and
    /// text are ever searched to the end.
    task: Option<Task<()>>,
    #[cfg(feature = "test-support")]
    background_searches: usize,
}

impl Find {
    /// The match that is the selection. The selection may reach further, to the end of a grapheme
    /// cluster (see `select_found`), so only the start has to agree.
    fn current(&self, selection: Selection) -> Option<usize> {
        if selection.is_empty() {
            return None;
        }
        let index = self
            .matches
            .partition_point(|m| m.start < selection.start());
        let found = self.matches.get(index)?;
        (found.start == selection.start()).then_some(index)
    }
}

impl EditorView {
    /// Highlights the matches of `query` and, once they are found, selects the first one from the
    /// cursor. Searches after [`SEARCH_DEBOUNCE`], and again after every change to the text,
    /// until [`end_find`](Self::end_find).
    pub fn find(&mut self, query: &str, case: CaseSensitivity, cx: &mut Context<Self>) {
        if self
            .find
            .as_ref()
            .is_some_and(|find| find.query == query && find.case == case)
        {
            return;
        }
        let version = self.editor.buffer().version();
        let find = self.find.get_or_insert_with(|| Find {
            query: String::new(),
            case,
            matches: Vec::new(),
            version,
            searched: false,
            select_first: false,
            pending_step: None,
            task: None,
            #[cfg(feature = "test-support")]
            background_searches: 0,
        });
        find.query = query.to_owned();
        find.case = case;
        find.select_first = true;
        find.pending_step = None;
        if query.is_empty() {
            find.matches.clear();
            find.task = None;
            cx.notify();
        } else {
            self.schedule_search(cx);
        }
    }

    /// Stops searching and removes the highlights. The selection stays.
    pub fn end_find(&mut self, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            cx.notify();
        }
    }

    /// The matches found for the query; `None` while there is no query or before its first
    /// search has finished.
    pub fn find_status(&self) -> Option<FindStatus> {
        let find = self
            .find
            .as_ref()
            .filter(|find| find.searched && !find.query.is_empty())?;
        Some(FindStatus {
            current: find.current(self.editor.selection()),
            total: find.matches.len(),
        })
    }

    /// Selects the next or previous match from the selection, wrapping around, and scrolls it
    /// into view. While a search is pending, this happens when it lands.
    pub fn select_match(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        if find.task.is_some() {
            find.pending_step = Some(direction);
        } else {
            self.step(direction, cx);
        }
    }

    /// Number of searches that ran on the background executor.
    #[cfg(feature = "test-support")]
    pub fn background_search_count(&self) -> usize {
        self.find
            .as_ref()
            .map_or(0, |find| find.background_searches)
    }

    fn step(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(find) = &self.find else {
            return;
        };
        let selection = self.editor.selection();
        let index = match direction {
            Direction::Next => search::next_match(&find.matches, selection.end()),
            Direction::Previous => search::prev_match(&find.matches, selection.start()),
        };
        if let Some(index) = index {
            let found = find.matches[index].clone();
            self.select_found(found, cx);
        }
    }

    /// Selects a match, widened to whole grapheme clusters: "e" matches the start of "e\u{301}".
    /// Selections never end inside a cluster, and one cut short there could end where the next
    /// step searches from and find the same match again, forever.
    fn select_found(&mut self, found: Range<ByteOffset>, cx: &mut Context<Self>) {
        let end = self.editor.buffer().clip_offset(found.end, Bias::Right);
        self.editor.set_selection(Selection::new(found.start, end));
        self.autoscroll = Some(Autoscroll::Center);
        self.selection_changed(cx);
    }

    /// Called after every change to the text: moves the matches along with it until it is
    /// searched again. A `new_document` has nothing in common with the old matches.
    pub(super) fn find_text_changed(&mut self, new_document: bool, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        // Someone typing in the note (or opening another) does not want the cursor taken to a
        // match, not even by a step asked for before.
        find.select_first = false;
        find.pending_step = None;
        let buffer = self.editor.buffer();
        match buffer.changes_since(find.version).filter(|_| !new_document) {
            Some(changes) => {
                for change in changes {
                    search::adjust_matches(&mut find.matches, change);
                }
            }
            None => {
                find.matches.clear();
                // Not "No results" before the search has even run.
                find.searched = false;
            }
        }
        find.version = buffer.version();
        if !find.query.is_empty() {
            self.schedule_search(cx);
        }
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        find.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let snapshot = this.update(cx, |this, _| {
                let find = this.find.as_mut()?;
                let buffer = this.editor.buffer();
                let background = buffer.len() > BACKGROUND_SEARCH_BYTES;
                #[cfg(feature = "test-support")]
                if background {
                    find.background_searches += 1;
                }
                // A rope clone is cheap; turning it into one string is not, for large notes.
                Some((buffer.clone(), find.query.clone(), find.case, background))
            });
            let Ok(Some((buffer, query, case, background))) = snapshot else {
                return;
            };
            let version = buffer.version();
            let search = move || search::find_all(&buffer.normalized_text(), &query, case);
            let matches = if background {
                cx.background_executor()
                    .spawn(async move { search() })
                    .await
            } else {
                search()
            };
            this.update(cx, |this, cx| this.search_finished(matches, version, cx))
                .ok();
        }));
    }

    fn search_finished(
        &mut self,
        matches: Vec<Range<ByteOffset>>,
        version: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(find) = &mut self.find else {
            return;
        };
        // Every change searches again, which cancels this search, so this does not happen; if
        // it did, the matches could be for other text.
        if version != self.editor.buffer().version() {
            self.schedule_search(cx);
            return;
        }
        find.task = None;
        find.matches = matches;
        find.version = version;
        find.searched = true;
        let select_first = std::mem::take(&mut find.select_first);
        match find.pending_step.take() {
            Some(direction) => self.step(direction, cx),
            None if select_first => {
                let start = self.editor.selection().start();
                if let Some(index) = search::next_match(&find.matches, start) {
                    let found = find.matches[index].clone();
                    self.select_found(found, cx);
                }
            }
            None => {}
        }
        cx.notify();
    }
}
