//! The native Open and Save As dialogs (ADRs 0145-0146).

use std::path::{Path, PathBuf};

use gpui::{App, PathPromptOptions, Task};

/// Stands in for the native Open dialog, which GPUI's test platform does not implement: while
/// set, File > Open picks this file (`None`: the dialog is cancelled). The Save As dialog needs
/// no seam: tests answer it with `simulate_new_path_selection`.
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

/// The file the user picks to save to, starting in `dir` with `name` filled in; `None` if they
/// cancel. The dialog itself asks before replacing an existing file.
pub fn pick_new_path(dir: &Path, name: &str, cx: &mut App) -> Task<Option<PathBuf>> {
    let picked = cx.prompt_for_new_path(dir, Some(name));
    cx.spawn(async move |_| match picked.await {
        Ok(Ok(path)) => path,
        Ok(Err(error)) => {
            tracing::warn!("could not show the save dialog: {error:#}");
            None
        }
        Err(_) => None,
    })
}
