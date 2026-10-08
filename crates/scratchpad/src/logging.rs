use std::fs::{self, File};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

/// Installs the global `tracing` subscriber.
///
/// Debug builds log to stderr. Release builds have no console (see `main.rs`), so they log to
/// `<local data dir>/Scratchpad/scratchpad.log` (`%LOCALAPPDATA%` on Windows) and also record
/// panics there. The previous run's log is kept as `scratchpad.prev.log`, so the log of a
/// crash survives the relaunch that follows it. If the file cannot be created, logs go to
/// stderr.
///
/// The filter comes from `SCRATCHPAD_LOG`, then `RUST_LOG`, defaulting to `info`. GPUI logs
/// through the `log` crate; those records are bridged into the same subscriber.
pub fn init() {
    let filter = EnvFilter::try_from_env("SCRATCHPAD_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    // Ignore failure: a subscriber may already be installed (e.g. by a test harness).
    let _ = match release_log_file() {
        Some(file) => {
            log_panics();
            // Unbuffered, so every line is on disk before a `panic = "abort"` crash.
            builder
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .try_init()
        }
        None => builder.with_writer(std::io::stderr).try_init(),
    };
}

fn release_log_file() -> Option<File> {
    if cfg!(debug_assertions) {
        return None;
    }
    let dir = dirs::data_local_dir()?.join("Scratchpad");
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("scratchpad.log");
    let _ = fs::rename(&path, dir.join("scratchpad.prev.log"));
    File::create(path).ok()
}

/// The default hook prints to stderr, which release builds do not have.
fn log_panics() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("{info}");
        default_hook(info);
    }));
}
