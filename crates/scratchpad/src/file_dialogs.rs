//! The native Open dialog (ADR 0145).

use std::path::PathBuf;

use gpui::{App, PathPromptOptions, Task};

/// Stands in for the native Open dialog, which GPUI's test platform does not implement: while
/// set, File > Open picks this file (`None`: the dialog is cancelled).
#[cfg(feature = "test-support")]
pub struct PickFileForTests(pub Option<PathBuf>);

#[cfg(feature = "test-support")]
impl gpui::Global for PickFileForTests {}

/// The file the user picks to open, `None` if they cancel.
pub fn pick_file(cx: &mut App) -> Task<Option<PathBuf>> {
    #[cfg(feature = "test-support")]
    if let Some(PickFileForTests(path)) = cx.try_global::<PickFileForTests>() {
        return Task::ready(path.clone());
    }
    // GPUI's Windows dialog offers no file type filter, so every file is shown.
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |_| match picked.await {
        Ok(Ok(paths)) => paths.and_then(|mut paths| paths.pop()),
        Ok(Err(error)) => {
            tracing::warn!("could not show the open dialog: {error:#}");
            None
        }
        Err(_) => None,
    })
}
