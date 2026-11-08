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
use std::sync::{Arc, OnceLock};
use std::time::Duration;

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

enum Request {
    /// Replies with the misspellings of every segment, in document offsets. A newer check
    /// supersedes this one: its reply is then dropped unsent.
    Check {
        segments: Vec<Segment>,
        reply: flume::Sender<Vec<Misspelling>>,
    },
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

/// Hands work to a spell checker that runs on a thread of its own. Cheap to clone.
#[derive(Clone)]
pub struct Speller {
    requests: flume::Sender<Request>,
    /// Set once the checker has started: whether there is one for the user's language.
    available: Arc<OnceLock<bool>>,
    /// Answers requests on the calling thread, for tests (see [`Speller::inline`]).
    inline: Option<Rc<RefCell<Worker>>>,
}

impl Speller {
    /// Starts a thread that makes a backend with `make` (on that thread) and answers requests.
    /// Requests sent before it is ready wait for it; without a backend, nothing is misspelled.
    pub fn start(make: impl FnOnce() -> Option<Box<dyn Backend>> + Send + 'static) -> Self {
        let (requests, receiver) = flume::unbounded();
        let available = Arc::new(OnceLock::new());
        let ready = available.clone();
        let started = std::thread::Builder::new()
            .name("spell-check".into())
            .spawn(move || {
                let backend = make();
                ready.set(backend.is_some()).ok();
                if backend.is_none() {
                    tracing::info!("no spell checker for the user's language");
                }
                Worker::new(backend).run(receiver);
            });
        if let Err(error) = started {
            tracing::warn!(%error, "could not start the spell checker");
            available.set(false).ok();
        }
        Self {
            requests,
            available,
            inline: None,
        }
    }

    /// A speller that answers each request at once on the calling thread, so tests see the
    /// replies without waiting for another thread.
    pub fn inline(backend: Box<dyn Backend>) -> Self {
        let (requests, _) = flume::unbounded();
        let available = Arc::new(OnceLock::new());
        available.set(true).ok();
        Self {
            requests,
            available,
            inline: Some(Rc::new(RefCell::new(Worker::new(Some(backend))))),
        }
    }

    /// Whether there is a checker for the user's language; `None` while it is starting.
    pub fn available(&self) -> Option<bool> {
        self.available.get().copied()
    }

    /// Checks `segments`; the reply never comes if another check is sent before this one is done.
    pub fn check(&self, segments: Vec<Segment>) -> flume::Receiver<Vec<Misspelling>> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Check { segments, reply });
        receiver
    }

    /// What the checker says about one word, waiting at most `timeout`: for autocorrect, which
    /// must decide before the next keystroke. `None` if the word is fine or the answer is late.
    pub fn check_word(&self, word: &str, timeout: Duration) -> Option<Action> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Word {
            word: word.to_owned(),
            reply,
        });
        receiver.recv_timeout(timeout).ok().flatten()
    }

    /// Replacements for `word`, best first.
    pub fn suggest(&self, word: &str) -> flume::Receiver<Vec<String>> {
        let (reply, receiver) = flume::bounded(1);
        self.send(Request::Suggest {
            word: word.to_owned(),
            reply,
        });
        receiver
    }

    pub fn add(&self, word: &str) {
        self.send(Request::Add(word.to_owned()));
    }

    pub fn ignore(&self, word: &str) {
        self.send(Request::Ignore(word.to_owned()));
    }

    fn send(&self, request: Request) {
        match &self.inline {
            Some(worker) => worker.borrow_mut().handle(request, None),
            // Only fails if the thread is gone; then there is nobody to answer.
            None => _ = self.requests.send(request),
        }
    }
}

/// The spell-check thread's state.
struct Worker {
    backend: Option<Box<dyn Backend>>,
    /// The newest check waiting while the current one finishes.
    pending: VecDeque<Request>,
}

impl Worker {
    fn new(backend: Option<Box<dyn Backend>>) -> Self {
        Self {
            backend,
            pending: VecDeque::new(),
        }
    }

    fn run(mut self, requests: flume::Receiver<Request>) {
        loop {
            let request = match self.pending.pop_front() {
                Some(request) => request,
                None => match requests.recv() {
                    Ok(request) => request,
                    Err(_) => return,
                },
            };
            self.handle(request, Some(&requests));
        }
    }

    /// Answers `request`. During a check, requests that arrive on `incoming` between segments
    /// are answered first, and a newer check ends this one.
    fn handle(&mut self, request: Request, incoming: Option<&flume::Receiver<Request>>) {
        let Some(backend) = &mut self.backend else {
            // Dropping the reply senders answers "nothing" to everyone.
            if let Request::Check { reply, .. } = request {
                _ = reply.send(Vec::new());
            }
            return;
        };
        match request {
            Request::Check { segments, reply } => {
                let mut found = Vec::new();
                for segment in segments {
                    if let Some(incoming) = incoming {
                        while let Ok(next) = incoming.try_recv() {
                            match next {
                                Request::Check { .. } => {
                                    self.pending.clear();
                                    self.pending.push_back(next);
                                }
                                quick => quick_reply(backend.as_mut(), quick),
                            }
                        }
                        if !self.pending.is_empty() {
                            return;
                        }
                    }
                    found.extend(check_segment(backend.as_mut(), &segment));
                }
                _ = reply.send(found);
            }
            quick => quick_reply(backend.as_mut(), quick),
        }
    }
}

fn quick_reply(backend: &mut dyn Backend, request: Request) {
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
        Request::Check { .. } => unreachable!("checks are not quick"),
    }
}

/// The misspellings of a segment that are worth showing, in document offsets.
fn check_segment(backend: &mut dyn Backend, segment: &Segment) -> Vec<Misspelling> {
    backend
        .check(&segment.text)
        .into_iter()
        .filter(|m| {
            segment
                .text
                .get(m.range.clone())
                .is_some_and(worth_flagging)
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

/// A checker that knows only the words it is given: for tests, and as a stand-in where there is
/// no system checker.
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
/// them ("don't"), as Unicode word boundaries find them.
pub(crate) fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    use unicode_segmentation::UnicodeSegmentation;
    text.split_word_bound_indices().filter_map(|(start, word)| {
        let trimmed = word.trim_matches(|c| matches!(c, '\'' | '\u{2019}'));
        let start = start + word.find(trimmed).unwrap_or(0);
        trimmed
            .chars()
            .any(char::is_alphabetic)
            .then_some((start, trimmed))
    })
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

    fn speller() -> Speller {
        Speller::inline(Box::new(
            WordList::new(["the", "cat", "sat", "on", "mat"]).with_autocorrect("teh", "the"),
        ))
    }

    #[test]
    fn misspellings_are_reported_in_document_offsets() {
        let speller = speller();
        let found = speller
            .check(vec![
                Segment {
                    offset: 10,
                    text: "the cat szt".into(),
                },
                Segment {
                    offset: 100,
                    text: "on teh mat".into(),
                },
            ])
            .recv()
            .unwrap();
        assert_eq!(
            found,
            [
                Misspelling {
                    range: 18..21,
                    action: Action::Suggest
                },
                Misspelling {
                    range: 103..106,
                    action: Action::Replace("the".into())
                },
            ]
        );
    }

    #[test]
    fn repeated_words_are_reported_once() {
        let mut list = WordList::new(["the", "cat"]);
        assert_eq!(
            list.check("the the cat"),
            [Misspelling {
                range: 4..7,
                action: Action::Delete
            }]
        );
        assert_eq!(list.check("the\nthe cat"), []);
    }

    #[test]
    fn names_numbers_and_abbreviations_are_not_flagged() {
        for word in ["NASA", "v2", "foo_bar", "example.com", "a/b", "me@x"] {
            assert!(!worth_flagging(word), "{word}");
        }
        for word in ["teh", "Teh", "naïve", "I"] {
            assert!(worth_flagging(word), "{word}");
        }
        let speller = speller();
        let found = speller
            .check(vec![Segment {
                offset: 0,
                text: "the NASA cat v2".into(),
            }])
            .recv()
            .unwrap();
        assert_eq!(found, []);
    }

    #[test]
    fn single_words_are_checked_for_autocorrect() {
        let speller = speller();
        let wait = Duration::from_millis(100);
        assert_eq!(
            speller.check_word("teh", wait),
            Some(Action::Replace("the".into()))
        );
        assert_eq!(speller.check_word("cat", wait), None);
        assert_eq!(speller.check_word("szt", wait), Some(Action::Suggest));
    }

    #[test]
    fn added_and_ignored_words_are_no_longer_misspelled() {
        let speller = speller();
        speller.add("Scratchpad");
        speller.ignore("gpui");
        let found = speller
            .check(vec![Segment {
                offset: 0,
                text: "scratchpad gpui".into(),
            }])
            .recv()
            .unwrap();
        assert_eq!(found, []);
    }

    #[test]
    fn suggestions_are_close_words() {
        let speller = speller();
        assert_eq!(
            speller.suggest("szt").recv().unwrap(),
            ["sat", "cat", "mat"]
        );
    }

    #[test]
    fn words_keep_inner_apostrophes() {
        let found: Vec<_> = words("'don't' stop, it’s 42 x").collect();
        assert_eq!(found, [(1, "don't"), (8, "stop"), (14, "it’s"), (24, "x")]);
    }

    #[test]
    fn a_thread_speller_answers_like_an_inline_one() {
        let speller = Speller::start(|| Some(Box::new(WordList::new(["cat"]))));
        let found = speller
            .check(vec![Segment {
                offset: 0,
                text: "cat dgo".into(),
            }])
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(speller.available(), Some(true));
    }

    #[test]
    fn a_newer_check_supersedes_an_older_one() {
        let (requests, receiver) = flume::unbounded();
        let (old_reply, old) = flume::bounded(1);
        let (new_reply, new) = flume::bounded(1);
        // The newer check is already waiting when the older one starts.
        requests
            .send(Request::Check {
                segments: vec![Segment {
                    offset: 0,
                    text: "dgo".into(),
                }],
                reply: new_reply,
            })
            .unwrap();
        drop(requests);
        let mut worker = Worker::new(Some(Box::new(WordList::new(["cat"]))));
        let old_check = Request::Check {
            segments: vec![Segment {
                offset: 0,
                text: "tac".into(),
            }],
            reply: old_reply,
        };
        worker.handle(old_check, Some(&receiver));
        worker.run(receiver);
        assert!(old.recv().is_err(), "the older check is never answered");
        assert_eq!(new.recv().unwrap().len(), 1);
    }
}
