use std::io;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

/// A failed filesystem operation.
///
/// The `Display` text names the action and the file, e.g. `could not save Meeting.md: Access is
/// denied. (os error 5)`. The app should keep the in-memory buffer and surface this minimally
/// (PLAN §56).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not {action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl Error {
    /// The underlying `io::ErrorKind`, e.g. to tell `NotFound` (note vanished) from
    /// `PermissionDenied`.
    pub fn io_kind(&self) -> io::ErrorKind {
        let Error::Io { source, .. } = self;
        source.kind()
    }
}

/// Adapter for `map_err` that records which operation on which file failed.
pub(crate) fn io_context<'a>(
    action: &'static str,
    path: &'a Path,
) -> impl FnOnce(io::Error) -> Error + 'a {
    move |source| {
        tracing::warn!(action, path = %path.display(), error = %source, "file operation failed");
        Error::Io {
            action,
            path: path.to_owned(),
            source,
        }
    }
}
