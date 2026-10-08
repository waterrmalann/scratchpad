//! Shared setup for end-to-end tests. See docs/adrs/0020-e2e-testing-strategy.md.
#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use std::cell::RefCell;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use chrono::{Local, NaiveTime, TimeZone};
use gpui::{
    Entity, Global, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Point, Subscription,
    TestAppContext, VisualTestContext,
};
use scratchpad::AppWindow;
use scratchpad::notes::{DraftId, Notes, NotesEvent, NotesLocation};
use tempfile::TempDir;

/// Keeps the notes folder of [`open_main_window`] alive as long as the app.
struct TempNotesDir(#[allow(dead_code)] TempDir);

impl Global for TempNotesDir {}

/// Initialises the app exactly as `main` does and opens the real main window in GPUI's
/// headless test platform, on an empty temporary notes folder.
///
/// Returns the root view and a window-bound context for simulating input
/// (`simulate_keystrokes`, `simulate_input`, `dispatch_action`, ...). The context lives until
/// the end of the test.
pub fn open_main_window(cx: &mut TestAppContext) -> (Entity<AppWindow>, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_owned();
    cx.set_global(TempNotesDir(dir));
    open_main_window_in(&path, cx)
}

/// Like [`open_main_window`] but on the notes in `dir`. Deleted notes go to [`trash_dir`]
/// instead of the real recycle bin. Waits until the notes have been listed.
pub fn open_main_window_in<'a>(
    dir: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<AppWindow>, &'a mut VisualTestContext) {
    cx.update(scratchpad::init);
    let location = location(dir);
    let window = cx.update(|cx| scratchpad::open_main_window(location, cx).expect("open window"));
    let root = window.root(cx).expect("main window root view");
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    // Like the real app's window after launch; focus-out events need an active window.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (root, cx)
}

/// The notes in `dir`, deleted into [`trash_dir`] instead of the real recycle bin.
pub fn location(dir: &Path) -> NotesLocation {
    NotesLocation {
        dir: dir.to_owned(),
        deleter: Some(move_to_test_trash),
    }
}

/// Where [`open_main_window_in`] puts deleted notes: a `.trash` folder that the note list
/// ignores, next to the note.
pub fn trash_dir(notes_dir: &Path) -> PathBuf {
    notes_dir.join(".trash")
}

fn move_to_test_trash(path: &Path) -> io::Result<()> {
    let trash = trash_dir(path.parent().unwrap());
    fs::create_dir_all(&trash)?;
    fs::rename(path, trash.join(path.file_name().unwrap()))
}

pub fn notes(root: &Entity<AppWindow>, cx: &mut VisualTestContext) -> Entity<Notes> {
    root.read_with(cx, |root, _| root.notes().clone())
}

/// Collects every event the notes model emits from now on.
pub struct EventLog {
    events: Rc<RefCell<Vec<NotesEvent>>>,
    _subscription: Subscription,
}

impl EventLog {
    pub fn new(notes: &Entity<Notes>, cx: &mut VisualTestContext) -> Self {
        let events = Rc::new(RefCell::new(Vec::new()));
        let log = events.clone();
        let subscription = cx.update(|_, cx| {
            cx.subscribe(notes, move |_, event: &NotesEvent, _| {
                log.borrow_mut().push(event.clone())
            })
        });
        Self {
            events,
            _subscription: subscription,
        }
    }

    /// The events since the last call.
    pub fn take(&self) -> Vec<NotesEvent> {
        self.events.take()
    }

    /// The draft opened since the last call, which must be the only event.
    pub fn take_draft(&self) -> DraftId {
        match self.take()[..] {
            [NotesEvent::OpenDraft(draft)] => draft,
            ref events => panic!("expected a single OpenDraft, got {events:?}"),
        }
    }
}

/// Writes `<title>.md` with the given modification time (see [`days_ago`]).
pub fn write_note(dir: &Path, title: &str, text: &str, modified: SystemTime) -> PathBuf {
    let path = dir.join(format!("{title}.md"));
    fs::write(&path, text).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    path
}

/// The given hour (local time) `days` days before today.
pub fn days_ago(days: u64, hour: u32) -> SystemTime {
    let date = Local::now().date_naive() - chrono::Days::new(days);
    let time = date.and_time(NaiveTime::from_hms_opt(hour, 0, 0).unwrap());
    Local.from_local_datetime(&time).earliest().unwrap().into()
}

/// Titles of the notes in the folder (not the app's view of it), sorted.
pub fn titles_on_disk(dir: &Path) -> Vec<String> {
    let mut titles: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    titles.sort();
    titles
}

/// The centre of the element tagged with `.debug_selector(|| selector)`, which must have been
/// rendered. GPUI never forgets recorded bounds, so this cannot prove an element is gone.
pub fn center_of(selector: &str, cx: &mut VisualTestContext) -> Point<gpui::Pixels> {
    let selector: &'static str = selector.to_owned().leak();
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was never rendered"))
        .center()
}

pub fn click(selector: &str, cx: &mut VisualTestContext) {
    let position = center_of(selector, cx);
    cx.simulate_click(position, Modifiers::none());
}

pub fn double_click(selector: &str, cx: &mut VisualTestContext) {
    let position = center_of(selector, cx);
    cx.simulate_click(position, Modifiers::none());
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
    });
}

pub fn right_click(selector: &str, cx: &mut VisualTestContext) {
    let position = center_of(selector, cx);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
}
