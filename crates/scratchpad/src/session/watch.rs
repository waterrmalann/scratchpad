//! Feeds changes that other programs make in the notes folder to the session (PLAN §30).

use std::iter;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, Task};
use scratchpad_core::NoteWatcher;

use super::Session;

/// How often queued filesystem events are collected. Events of one burst (an editor's save is
/// often several) mostly arrive together.
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Watches `dir` and hands its events to [`Session::disk_events`] until the task is dropped.
pub fn start(dir: PathBuf, cx: &mut Context<Session>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let watcher = match NoteWatcher::start(&dir) {
            Ok(watcher) => watcher,
            Err(error) => {
                // The app works without it; changes by other programs show after a restart.
                tracing::warn!(%error, "not watching the notes folder");
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
