//! Feeds changes that other programs make in the notes folder, or to a file opened from
//! elsewhere, to the session (PLAN §30).

use std::iter;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, Task};
use scratchpad_core::NoteWatcher;

use super::Session;

/// How often queued filesystem events are collected. Events of one burst (an editor's save is
/// often several) mostly arrive together.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Watches the notes folder `dir` and hands its events to [`Session::disk_events`] until the
/// task is dropped.
pub fn start_folder(dir: PathBuf, cx: &mut Context<Session>) -> Task<()> {
    start(move || NoteWatcher::start(&dir), cx)
}

/// Watches the file at `path` like [`start_folder`] watches the notes folder.
pub fn start_file(path: PathBuf, cx: &mut Context<Session>) -> Task<()> {
    start(move || NoteWatcher::start_file(&path), cx)
}

fn start(
    watch: impl FnOnce() -> scratchpad_core::Result<NoteWatcher> + Send + 'static,
    cx: &mut Context<Session>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        // Off the UI thread: opening a folder on a slow network share can take seconds.
        let watcher = match cx.background_executor().spawn(async move { watch() }).await {
            Ok(watcher) => watcher,
            Err(error) => {
                // The app works without it; changes by other programs show after a restart.
                tracing::warn!(%error, "not watching for changes by other programs");
                return;
            }
        };
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let events: Vec<_> = iter::from_fn(|| watcher.try_recv()).collect();
            if !events.is_empty()
                && this
                    .update(cx, |session, cx| session.disk_events(events, cx))
                    .is_err()
            {
                break;
            }
        }
    })
}
