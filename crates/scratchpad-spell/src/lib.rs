//! Spell checking with the system's own checker (ADR 0170): the Windows Spell Checking API, or
//! elsewhere the Hunspell dictionaries the system has installed.
//!
//! A [`Backend`] checks text and knows the user's dictionary. The [`Speller`] owns one on a
//! thread of its own, because the Windows checker is a COM object that must stay on the thread
//! that made it, and so that checking never holds up typing. The app sends it the prose of the
//! note, never code, and gets back the misspelled words.

#[cfg(any(not(windows), test))]
#[cfg_attr(windows, allow(dead_code))]
mod hunspell;
#[cfg(windows)]
mod windows;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// A word the checker does not know, or a repeated one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Misspelling {
    /// Byte range of the word in the text that was checked.
    pub range: Range<usize>,
    pub action: Action,
}

/// What the checker suggests doing about a [`Misspelling`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The word is unknown: [`Speller::suggest`] lists replacements.
    Suggest,
    /// A known typo with one sure correction, from the system's autocorrect list ("teh" to
    /// "the"). Only the Windows checker has such a list.
    Replace(String),
    /// The same word twice in a row: the second should go.
    Delete,
}

/// A spell checker for one language.
pub trait Backend {
    /// The misspellings in `text`, in order. Words the user added or ignored are not reported.
    fn check(&mut self, text: &str) -> Vec<Misspelling>;
    /// Replacements for an unknown `word`, best first.
    fn suggest(&mut self, word: &str) -> Vec<String>;
    /// Adds `word` to the user's dictionary, for good.
    fn add(&mut self, word: &str);
    /// Stops reporting `word` until the app quits.
    fn ignore(&mut self, word: &str);
}

/// The system's spell checker for the user's language, if it has one: on Windows the Spell
/// Checking API, elsewhere Hunspell dictionaries. `personal_dictionary` is where words added
/// outside Windows are kept (Windows keeps its own list).
pub fn system_backend(personal_dictionary: Option<PathBuf>) -> Option<Box<dyn Backend>> {
    #[cfg(windows)]
    {
        let _ = personal_dictionary;
        windows::WindowsChecker::new().map(|checker| Box::new(checker) as Box<dyn Backend>)
    }
    #[cfg(not(windows))]
    {
        hunspell::HunspellChecker::from_system(personal_dictionary)
            .map(|checker| Box::new(checker) as Box<dyn Backend>)
    }
}

/// A run of prose to check, at `offset` in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub offset: usize,
    pub text: String,
}

/// The answer to [`Speller::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// The misspellings of every segment, in document offsets.
    Done(Vec<Misspelling>),
    /// The same requester sent a newer check before this one was done.
    Superseded,
    /// The checker is gone: its thread ended.
    Unavailable,
}

/// A check waiting for, or in, the spell-check thread.
struct Check {
    /// Who asked: a newer check from the same requester supersedes this one, others wait.
    requester: u64,
    segments: Vec<Segment>,
    reply: flume::Sender<Checked>,
}

enum Request {
    Check(Check),
    Word {
        word: String,
        reply: flume::Sender<Option<Action>>,
    },
    Suggest {
        word: String,
        reply: flume::Sender<Vec<String>>,
    },
    Add(String),
    Ignore(String),
}

/// [`Speller::available`], as one atomic: not known yet, yes, no.
const STARTING: u8 = 0;
const AVAILABLE: u8 = 1;
const UNAVAILABLE: u8 = 2;

/// Hands work to a spell checker that runs on a thread of its own. Cheap to clone.
#[derive(Clone)]
pub struct Speller {
    requests: flume::Sender<Request>,
    /// Whether there is a checker for the user's language: [`STARTING`] until it has started,
    /// and [`UNAVAILABLE`] without one or once its thread is gone.
    available: Arc<AtomicU8>,
    /// Answers requests on the calling thread, for tests (see [`Speller::inline`]).
    inline: Option<Rc<RefCell<Inline>>>,
}

/// The worker of an inline speller, with the requests it has not answered yet.
struct Inline {
    worker: Worker,
    requests: flume::Receiver<Request>,
    held: bool,
}

impl Speller {
    /// Starts a thread that makes a backend with `make` (on that thread) and answers requests.
    /// Requests sent before it is ready wait for it; without a backend, nothing is misspelled.
    pub fn start(make: impl FnOnce() -> Option<Box<dyn Backend>> + Send + 'static) -> Self {
        let (requests, receiver) = flume::unbounded();
        let available = Arc::new(AtomicU8::new(STARTING));
        let ready = available.clone();
        let started = std::thread::Builder::new()
            .name("spell-check".into())
            .spawn(move || {
                let backend = make();
                let state = if backend.is_some() {
                    AVAILABLE
                } else {
                    tracing::info!("no spell checker for the user's language");
                    UNAVAILABLE
                };
                // Unless it was already found gone.
                _ = ready.compare_exchange(STARTING, state, Ordering::AcqRel, Ordering::Acquire);
                Worker::new(backend).run(&receiver);
            });
        if let Err(error) = started {
            tracing::warn!(%error, "could not start the spell checker");
            available.store(UNAVAILABLE, Ordering::Release);
        }
        Self {
            requests,
            available,
            inline: None,
        }
    }

    /// A speller that answers each request at once on the calling thread, so tests see the
    /// replies without waiting for another thread. See [`Speller::hold`].
    pub fn inline(backend: Box<dyn Backend>) -> Self {
        let (requests, receiver) = flume::unbounded();
        Self {
            requests,
            available: Arc::new(AtomicU8::new(AVAILABLE)),
            inline: Some(Rc::new(RefCell::new(Inline {
                worker: Worker::new(Some(backend)),
                requests: receiver,
                held: false,
            }))),
        }
    }

    /// For tests: an inline speller leaves the requests it gets from now on unanswered, as a
    /// busy checker would, until [`Speller::release`].
    pub fn hold(&self) {
        if let Some(inline) = &self.inline {
            inline.borrow_mut().held = true;
        }
    }

    /// Answers the requests [`Speller::hold`] kept, in order, as the thread would have.
    pub fn release(&self) {
        if let Some(inline) = &self.inline {
            let mut inline = inline.borrow_mut();
            inline.held = false;
            let Inline {
                worker, requests, ..
            } = &mut *inline;
            worker.answer_waiting(requests);
        }
    }

    /// Whether there is a checker for the user's language; `None` while it is starting.
    pub fn available(&self) -> Option<bool> {
        match self.available.load(Ordering::Acquire) {
            STARTING => None,
            state => Some(state == AVAILABLE),
        }
    }

    /// Checks `segments` for `requester` (one per view). A newer check from the same requester
    /// supersedes this one; checks from others wait their turn.
    pub fn check(
        &self,
        requester: u64,
        segments: Vec<Segment>,
    ) -> impl Future<Output = Checked> + use<> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Check(Check {
            requester,
            segments,
            reply,
        }));
        let available = self.available.clone();
        async move {
            receiver.recv_async().await.unwrap_or_else(|_| {
                gone(&available);
                Checked::Unavailable
            })
        }
    }

    /// What the checker says about one word, for autocorrect: `None` if the word is fine or
    /// the checker is gone.
    pub fn check_word(&self, word: &str) -> impl Future<Output = Option<Action>> + use<> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Word {
            word: word.to_owned(),
            reply,
        });
        let available = self.available.clone();
        async move {
            receiver.recv_async().await.unwrap_or_else(|_| {
                gone(&available);
                None
            })
        }
    }

    /// Replacements for `word`, best first.
    pub fn suggest(&self, word: &str) -> impl Future<Output = Vec<String>> + use<> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Suggest {
            word: word.to_owned(),
            reply,
        });
        let available = self.available.clone();
        async move {
            receiver.recv_async().await.unwrap_or_else(|_| {
                gone(&available);
                Vec::new()
            })
        }
    }

    pub fn add(&self, word: &str) {
        self.send(Request::Add(word.to_owned()));
    }

    pub fn ignore(&self, word: &str) {
        self.send(Request::Ignore(word.to_owned()));
    }

    fn send(&self, request: Request) {
        // Only fails if the thread is gone; the request's reply sender goes with it.
        if self.requests.send(request).is_err() {
            gone(&self.available);
            return;
        }
        if let Some(inline) = &self.inline {
            let mut inline = inline.borrow_mut();
            if !inline.held {
                let Inline {
                    worker, requests, ..
                } = &mut *inline;
                worker.answer_waiting(requests);
            }
        }
    }
}

/// Marks the checker as gone once its thread has ended (it panicked), saying so once.
fn gone(available: &AtomicU8) {
    if available.swap(UNAVAILABLE, Ordering::AcqRel) != UNAVAILABLE {
        tracing::warn!("the spell checker stopped; spelling is no longer checked");
    }
}

/// The spell-check thread's state.
struct Worker {
    backend: Option<Box<dyn Backend>>,
    /// Checks waiting while the current one finishes, oldest first, at most one per requester.
    waiting: VecDeque<Check>,
}

impl Worker {
    fn new(backend: Option<Box<dyn Backend>>) -> Self {
        Self {
            backend,
            waiting: VecDeque::new(),
        }
    }

    /// Answers requests until every [`Speller`] is gone.
    fn run(mut self, requests: &flume::Receiver<Request>) {
        while let Some(check) = self.next_check(requests, |requests| requests.recv().ok()) {
            self.check(check, requests);
        }
    }

    /// Answers everything already sent, without waiting for more.
    fn answer_waiting(&mut self, requests: &flume::Receiver<Request>) {
        while let Some(check) = self.next_check(requests, |requests| requests.try_recv().ok()) {
            self.check(check, requests);
        }
    }

    /// The check to do next, answering the quick requests before it. `None` when `receive`
    /// has nothing more.
    fn next_check(
        &mut self,
        requests: &flume::Receiver<Request>,
        receive: impl Fn(&flume::Receiver<Request>) -> Option<Request>,
    ) -> Option<Check> {
        if let Some(check) = self.waiting.pop_front() {
            return Some(check);
        }
        loop {
            match receive(requests)? {
                Request::Check(check) => return Some(check),
                quick => quick_reply(self.backend.as_mut(), quick),
            }
        }
    }

    /// Answers `check`. Requests that arrive on `incoming` meanwhile are taken between
    /// segments: single words, suggestions and the like are answered at once, other
    /// requesters' checks wait, and a newer check from the same requester ends this one.
    fn check(&mut self, check: Check, incoming: &flume::Receiver<Request>) {
        let Self { backend, waiting } = self;
        let Some(backend) = backend else {
            _ = check.reply.send(Checked::Done(Vec::new()));
            return;
        };
        let mut found = Vec::new();
        for segment in &check.segments {
            while let Ok(next) = incoming.try_recv() {
                match next {
                    Request::Check(newer) => wait(waiting, newer),
                    quick => quick_reply(Some(backend), quick),
                }
            }
            if waiting.iter().any(|w| w.requester == check.requester) {
                _ = check.reply.send(Checked::Superseded);
                return;
            }
            found.extend(check_segment(backend.as_mut(), segment));
        }
        _ = check.reply.send(Checked::Done(found));
    }
}

/// Queues `check`, in place of an older one from the same requester if there is one.
fn wait(waiting: &mut VecDeque<Check>, check: Check) {
    match waiting.iter_mut().find(|w| w.requester == check.requester) {
        Some(older) => {
            let older = std::mem::replace(older, check);
            _ = older.reply.send(Checked::Superseded);
        }
        None => waiting.push_back(check),
    }
}

fn quick_reply(backend: Option<&mut Box<dyn Backend>>, request: Request) {
    let Some(backend) = backend else {
        // Without a checker every word is fine and there is nothing to suggest.
        match request {
            Request::Word { reply, .. } => _ = reply.send(None),
            Request::Suggest { reply, .. } => _ = reply.send(Vec::new()),
            _ => {}
        }
        return;
    };
    match request {
        Request::Word { word, reply } => {
            let action = backend
                .check(&word)
                .into_iter()
                .find(|m| m.range == (0..word.len()) && worth_flagging(&word))
                .map(|m| m.action);
            _ = reply.send(action);
        }
        Request::Suggest { word, reply } => _ = reply.send(backend.suggest(&word)),
        Request::Add(word) => backend.add(&word),
        Request::Ignore(word) => backend.ignore(&word),
        Request::Check(_) => unreachable!("checks are not quick"),
    }
}

/// The misspellings of a segment that are worth showing, in document offsets. Words in an
/// address or a path (a run without spaces holding `@`, `/` or `\`) are left out, however the
/// backend split it.
fn check_segment(backend: &mut dyn Backend, segment: &Segment) -> Vec<Misspelling> {
    let skipped: Vec<Range<usize>> = tokens(&segment.text)
        .filter(|(_, token)| !plain_token(token))
        .map(|(start, token)| start..start + token.len())
        .collect();
    backend
        .check(&segment.text)
        .into_iter()
        .filter(|m| {
            segment
                .text
                .get(m.range.clone())
                .is_some_and(worth_flagging)
                && !skipped
                    .iter()
                    .any(|s| s.start < m.range.end && m.range.start < s.end)
        })
        .map(|m| Misspelling {
            range: m.range.start + segment.offset..m.range.end + segment.offset,
            action: m.action,
        })
        .collect()
}

/// Whether a word the checker does not know reads as a misspelling rather than a name of
/// something: words with digits or underscores, abbreviations in capitals and addresses or file
/// names (with `.`, `/` or `@` inside) are left alone.
pub fn worth_flagging(word: &str) -> bool {
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    let upper = word.chars().filter(|c| c.is_uppercase()).count();
    letters > 0
        && !(letters > 1 && upper == letters)
        && !word
            .chars()
            .any(|c| c.is_ascii_digit() || matches!(c, '_' | '.' | '/' | '@' | '\\'))
}

/// A checker that knows only the words it is given, and corrects only the typos it is given:
/// the system checker's stand-in in tests. The app itself never uses it.
#[derive(Debug, Default, Clone)]
pub struct WordList {
    /// Known words, compared case-insensitively.
    words: HashSet<String>,
    /// Sure corrections, by lower-case typo.
    autocorrect: HashMap<String, String>,
    ignored: HashSet<String>,
}

impl WordList {
    pub fn new<'a>(words: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            words: words.into_iter().map(str::to_lowercase).collect(),
            ..Self::default()
        }
    }

    /// Corrects `typo` to `correction` automatically.
    pub fn with_autocorrect(mut self, typo: &str, correction: &str) -> Self {
        self.autocorrect
            .insert(typo.to_lowercase(), correction.to_owned());
        self
    }

    fn knows(&self, word: &str) -> bool {
        let word = word.to_lowercase();
        self.words.contains(&word) || self.ignored.contains(&word)
    }
}

impl Backend for WordList {
    fn check(&mut self, text: &str) -> Vec<Misspelling> {
        let mut found = Vec::new();
        let mut previous: Option<(usize, &str)> = None;
        for (start, word) in words(text) {
            let range = start..start + word.len();
            let lower = word.to_lowercase();
            let action = if let Some(correction) = self.autocorrect.get(&lower) {
                Some(Action::Replace(correction.clone()))
            } else if !self.knows(word) {
                Some(Action::Suggest)
            } else if previous
                .is_some_and(|(end, previous)| repeats(text, previous, end, word, start))
            {
                Some(Action::Delete)
            } else {
                None
            };
            if let Some(action) = action {
                found.push(Misspelling { range, action });
            }
            previous = Some((start + word.len(), word));
        }
        found
    }

    fn suggest(&mut self, word: &str) -> Vec<String> {
        let word = word.to_lowercase();
        let mut close: Vec<&String> = self
            .words
            .iter()
            .filter(|known| edit_distance(known, &word) <= 2)
            .collect();
        close.sort_by_key(|known| (edit_distance(known, &word), known.as_str()));
        close.into_iter().take(5).cloned().collect()
    }

    fn add(&mut self, word: &str) {
        self.words.insert(word.to_lowercase());
    }

    fn ignore(&mut self, word: &str) {
        self.ignored.insert(word.to_lowercase());
    }
}

/// The words of `text` with their byte offsets: runs of letters, with the apostrophes inside
/// them ("don't"), as Unicode word boundaries find them. Runs without spaces that hold `@`,
/// `/` or `\` (addresses and paths) have no words.
pub(crate) fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    use unicode_segmentation::UnicodeSegmentation;
    tokens(text)
        .filter(|(_, token)| plain_token(token))
        .flat_map(|(at, token)| {
            token
                .split_word_bound_indices()
                .filter_map(move |(start, word)| {
                    let trimmed = word.trim_matches(|c| matches!(c, '\'' | '\u{2019}'));
                    let start = at + start + word.find(trimmed).unwrap_or(0);
                    trimmed
                        .chars()
                        .any(char::is_alphabetic)
                        .then_some((start, trimmed))
                })
        })
}

/// The runs of `text` between whitespace, with their byte offsets.
fn tokens(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split(char::is_whitespace)
        .filter(|token| !token.is_empty())
        .map(move |token| (token.as_ptr() as usize - text.as_ptr() as usize, token))
}

/// Whether a run between spaces can hold words: it is not an e-mail address or a path.
fn plain_token(token: &str) -> bool {
    !token.contains(['@', '/', '\\'])
}

/// Whether `word` at `start` repeats `previous`, which ends at `previous_end`, with only spaces
/// and tabs between them.
pub(crate) fn repeats(
    text: &str,
    previous: &str,
    previous_end: usize,
    word: &str,
    start: usize,
) -> bool {
    previous_end < start
        && previous.to_lowercase() == word.to_lowercase()
        && text[previous_end..start]
            .chars()
            .all(|c| c == ' ' || c == '\t')
}

/// Levenshtein distance between two short words, by chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_numbers_and_abbreviations_are_not_flagged() {
        for word in ["NASA", "v2", "foo_bar", "example.com"] {
            assert!(!worth_flagging(word), "{word}");
        }
        for word in ["teh", "Teh", "naïve", "I"] {
            assert!(worth_flagging(word), "{word}");
        }
    }

    #[test]
    fn words_keep_inner_apostrophes() {
        let found: Vec<_> = words("'don't' stop, it’s 42 x").collect();
        assert_eq!(found, [(1, "don't"), (8, "stop"), (14, "it’s"), (24, "x")]);
    }

    #[test]
    fn addresses_and_paths_have_no_words() {
        let text = "mail alice@example.com, see notes/todo or C:\\temp\\fiel now";
        let found: Vec<&str> = words(text).map(|(_, word)| word).collect();
        assert_eq!(found, ["mail", "see", "or", "now"]);
        assert_eq!(
            words("a\tb  c").collect::<Vec<_>>(),
            [(0, "a"), (2, "b"), (5, "c")]
        );
    }
}
