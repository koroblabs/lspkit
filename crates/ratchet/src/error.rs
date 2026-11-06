//! Why a baseline could not be read or written.

use std::{
    fmt::{self, Display},
    path::PathBuf,
};

/// Why a baseline could not be read or written. It displays as the message a command prints, path
/// first: `orthra-baseline.toml: x: not a count`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The file could not be read or written.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The text is not a baseline: not TOML, or a count that is not a non-negative integer. `path`
    /// is the file it came from, if any.
    Malformed {
        path: Option<PathBuf>,
        reason: String,
    },
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Malformed {
                path: Some(path),
                reason,
            } => write!(f, "{}: {reason}", path.display()),
            Error::Malformed { path: None, reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Malformed { .. } => None,
        }
    }
}

/// The message, so `?` works in a function returning `Result<_, String>`.
impl From<Error> for String {
    fn from(error: Error) -> String {
        error.to_string()
    }
}
