//! The ETL's error type.
//!
//! Hand-written rather than derived because this crate deliberately has no
//! dependency beyond what the scaffold approved. The variants exist so tests
//! (and the CLI's exit code) can tell an integrity failure, which must stop a
//! regeneration cold, from bad input or a flaky network.

use std::{fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum Error {
    /// The command line could not be understood.
    Usage(String),
    /// Reading or writing `path` failed.
    Io {
        path: PathBuf,
        source: io::Error,
    },
    /// `path` is not the JSON this tool expects.
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// A downloaded file differs from the pinned manifest, or the manifest
    /// does not describe the pinned sources. Never retried or worked around.
    Integrity(String),
    /// A download failed (transport error or non-success status).
    Http(String),
    /// The translation provider failed in a way that ends the backfill.
    Translation(vocab_core::ProviderError),
    /// Source data the ETL cannot turn into a curriculum, or a curriculum
    /// that fails the spot checks.
    Data(String),
    /// An Anki package this tool cannot read.
    Apkg(String),
    Sqlite(rusqlite::Error),
    Zip(zip::result::ZipError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Error::Json {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(message) => write!(f, "{message}"),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Json { path, source } => write!(f, "{}: invalid JSON: {source}", path.display()),
            Error::Integrity(message) => write!(f, "integrity check failed: {message}"),
            Error::Http(message) => write!(f, "download failed: {message}"),
            Error::Translation(source) => write!(f, "translation failed: {source}"),
            Error::Data(message) => write!(f, "{message}"),
            Error::Apkg(message) => write!(f, "{message}"),
            Error::Sqlite(source) => write!(f, "Anki collection: {source}"),
            Error::Zip(source) => write!(f, "Anki package: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Json { source, .. } => Some(source),
            Error::Translation(source) => Some(source),
            Error::Sqlite(source) => Some(source),
            Error::Zip(source) => Some(source),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(source: rusqlite::Error) -> Self {
        Error::Sqlite(source)
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(source: zip::result::ZipError) -> Self {
        Error::Zip(source)
    }
}
