//! SQLite persistence (Milestones F and K, plus Milestone I's progress
//! tables): one database file that Automator and Learn have open at the same
//! time (docs/architecture.md § 6 and § 12 items 2–4).
//!
//! Sharing one file between two running apps drives most choices here:
//!
//! - **WAL and `busy_timeout`.** Readers never block the writer, and a write
//!   that meets the other app's write waits up to five seconds instead of
//!   failing with `SQLITE_BUSY`.
//! - **`IMMEDIATE` transactions.** [`Store::with_transaction`] takes the write
//!   lock when it begins. A deferred transaction that reads and then writes
//!   has to upgrade its lock, and in WAL mode that upgrade fails at once,
//!   without waiting, if the other app committed in between.
//! - **`foreign_keys = ON` on every connection.** SQLite keeps the setting per
//!   connection and defaults it to off; without it, deleting a capture would
//!   orphan its `srs_state` and `review_log` rows.
//! - **Migrations re-read `user_version` inside their transaction**, so two
//!   apps upgrading the file at the same moment apply each step once, and an
//!   app older than the file's schema refuses to touch it.
//!
//! Instants are stored as RFC 3339 UTC text with exactly three fractional
//! digits (`2026-10-31T12:00:00.000Z`) because SQL compares them as text, and
//! only a fixed width makes text order chronological. Precision below a
//! millisecond is truncated on write. Calendar dates are `YYYY-MM-DD`.

mod captures;
mod codec;
mod curriculum;
mod progress;
mod reviews;
mod schema;
mod state;

use std::{
    ffi::OsString,
    fmt::Display,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rusqlite::{Connection, ErrorCode, TransactionBehavior};

pub use curriculum::{curriculum_fingerprint, CURRICULUM_FINGERPRINT_KEY};
pub use progress::DailySessionRow;
pub use reviews::{ReviewLogRow, SrsStateRow};
pub use schema::SCHEMA_VERSION;

/// Environment variable that overrides [`default_db_path`]: E2E runs and
/// manual testing point both apps at a scratch file with it.
pub const DB_PATH_ENV: &str = "VOCAB_DB_PATH";

const APP_DIR_NAME: &str = "VocabularyAutomator";
const DB_FILE_NAME: &str = "vocab.sqlite3";

/// How long a statement waits for the other app's write lock.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Pause between attempts to switch a file that another app is converting
/// at the same moment to WAL.
const WAL_RETRY_INTERVAL: Duration = Duration::from_millis(5);

/// Everything that can go wrong in the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite rejected an operation, including constraint violations such as
    /// a duplicate `curriculum_order` or an `srs_state` row for a missing card.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A JSON column (`captures.tags`) could not be encoded or decoded.
    #[error("invalid JSON in the database: {0}")]
    Json(#[from] serde_json::Error),
    /// Creating the database's directory failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The database holds something this build cannot interpret: a schema
    /// newer than [`SCHEMA_VERSION`], an unknown enum value, a malformed
    /// timestamp.
    #[error("invalid data in the database: {0}")]
    Invalid(String),
    /// The row an update referred to does not exist.
    #[error("not found: {0}")]
    NotFound(String),
}

impl StoreError {
    /// Prefixes an [`StoreError::Invalid`] message with the row it came from,
    /// so a corrupt shared file can be repaired by hand.
    fn in_row(self, table: &str, key: impl Display) -> Self {
        match self {
            StoreError::Invalid(message) => {
                StoreError::Invalid(format!("{table} row {key}: {message}"))
            }
            other => other,
        }
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Where both apps keep their shared database: `$VOCAB_DB_PATH` when set and
/// not blank, else `<data dir>/VocabularyAutomator/vocab.sqlite3`
/// (`~/Library/Application Support/...` on macOS, `%APPDATA%\...` on
/// Windows, `~/.local/share/...` on Linux).
pub fn default_db_path() -> PathBuf {
    db_path_from(std::env::var_os(DB_PATH_ENV), dirs::data_dir())
}

/// The precedence rules behind [`default_db_path`], with the environment
/// value and the platform data directory passed in so they can be tested
/// without touching the process environment.
///
/// With no data directory either (no `$HOME`), the path is relative to the
/// working directory; [`Store::open`] then reports whatever I/O error that
/// leads to rather than this function guessing a location.
pub fn db_path_from(env_value: Option<OsString>, data_dir: Option<PathBuf>) -> PathBuf {
    match env_value {
        // A value that is not UTF-8 is still a path, just not a blank one.
        Some(value) if !value.to_str().is_some_and(|text| text.trim().is_empty()) => {
            PathBuf::from(value)
        }
        _ => data_dir
            .unwrap_or_default()
            .join(APP_DIR_NAME)
            .join(DB_FILE_NAME),
    }
}

/// A connection to the shared database, migrated to [`SCHEMA_VERSION`].
///
/// Each app opens its own `Store`. It is `Send` but not `Sync`: share one
/// between threads behind a `Mutex`, or open one per thread.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the database at `path`, creating the file and any missing
    /// parent directories, and migrates it to [`SCHEMA_VERSION`]. Fails with
    /// [`StoreError::Invalid`] when the file's schema is newer than this
    /// build, without modifying it.
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    /// A private, empty in-memory database with the full schema.
    pub fn open_in_memory() -> StoreResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// [`Store::open`] at [`default_db_path`].
    pub fn open_default() -> StoreResult<Self> {
        Self::open(default_db_path())
    }

    fn init(mut conn: Connection) -> StoreResult<Self> {
        // First, so that everything below already waits for the other app.
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // Refuse a newer schema before changing anything about the file.
        schema::supported_version(&conn)?;
        enable_wal(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        conn.set_transaction_behavior(TransactionBehavior::Immediate);
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Runs `f` atomically and returns its result: commits when it returns
    /// `Ok`, rolls back when it returns `Err` or panics. `f` receives this
    /// same `Store`, so it is written with the ordinary methods.
    ///
    /// Calls nest. Inside another `with_transaction`, `f` runs in a
    /// savepoint: an inner `Err` undoes only the inner work, and the outer
    /// closure decides whether to propagate it.
    pub fn with_transaction<T>(&self, f: impl FnOnce(&Store) -> StoreResult<T>) -> StoreResult<T> {
        if !self.conn.is_autocommit() {
            return self.with_savepoint(f);
        }
        let tx = self.conn.unchecked_transaction()?;
        // On the error path `tx` is dropped here, which rolls back.
        let value = f(self)?;
        tx.commit()?;
        Ok(value)
    }

    fn with_savepoint<T>(&self, f: impl FnOnce(&Store) -> StoreResult<T>) -> StoreResult<T> {
        self.conn.execute_batch("SAVEPOINT with_transaction")?;
        match f(self) {
            Ok(value) => {
                self.conn.execute_batch("RELEASE with_transaction")?;
                Ok(value)
            }
            Err(err) => {
                // ROLLBACK TO undoes the work but leaves the savepoint open;
                // RELEASE closes it.
                if let Err(rollback_err) = self
                    .conn
                    .execute_batch("ROLLBACK TO with_transaction; RELEASE with_transaction")
                {
                    log::error!("rolling back a nested transaction failed: {rollback_err}");
                }
                Err(err)
            }
        }
    }

    /// The underlying connection, for vocab-core modules that run their own
    /// queries (`quiz::LearnedVocabularyPool::QUERY`). Kept out of the public
    /// API so the apps write only through the typed methods.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Switches the file to WAL; a no-op once it is.
///
/// Converting a fresh file takes the write lock while already holding a read
/// lock. If another app is converting the same file at that moment, SQLite
/// answers `SQLITE_BUSY` at once instead of consulting the busy timeout,
/// because two connections each waiting for the other's lock would deadlock.
/// That only happens while the file is new, so retrying for as long as the
/// busy timeout would have waited is enough.
fn enable_wal(conn: &Connection) -> StoreResult<()> {
    let give_up = Instant::now() + BUSY_TIMEOUT;
    let journal_mode: String = loop {
        match conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0)) {
            Err(err)
                if err.sqlite_error_code() == Some(ErrorCode::DatabaseBusy)
                    && Instant::now() < give_up =>
            {
                std::thread::sleep(WAL_RETRY_INTERVAL);
            }
            result => break result?,
        }
    };
    // The mode now in effect: `memory` for in-memory databases, the old mode
    // on file systems without shared-memory support. Both still work, only
    // without concurrent readers.
    if !journal_mode.eq_ignore_ascii_case("wal") && !journal_mode.eq_ignore_ascii_case("memory") {
        log::warn!("SQLite kept journal_mode={journal_mode}; readers will block writers");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_prefers_a_non_blank_environment_override() {
        let data_dir = Some(PathBuf::from("/data"));
        let default = PathBuf::from("/data/VocabularyAutomator/vocab.sqlite3");
        let cases: [(Option<&str>, Option<PathBuf>, PathBuf); 6] = [
            (
                Some("/tmp/e2e.sqlite3"),
                data_dir.clone(),
                PathBuf::from("/tmp/e2e.sqlite3"),
            ),
            (Some("relative.db"), None, PathBuf::from("relative.db")),
            (None, data_dir.clone(), default.clone()),
            (Some(""), data_dir.clone(), default.clone()),
            (Some("  "), data_dir.clone(), default),
            (
                None,
                None,
                PathBuf::from("VocabularyAutomator/vocab.sqlite3"),
            ),
        ];
        for (env_value, data_dir, expected) in cases {
            assert_eq!(
                db_path_from(env_value.map(OsString::from), data_dir.clone()),
                expected,
                "env {env_value:?}, data dir {data_dir:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn db_path_keeps_an_override_that_is_not_utf8() {
        use std::os::unix::ffi::OsStringExt;

        let raw = OsString::from_vec(b"/tmp/caf\xe9.db".to_vec());
        assert_eq!(
            db_path_from(Some(raw.clone()), Some(PathBuf::from("/data"))),
            PathBuf::from(raw)
        );
    }

    #[test]
    fn init_sets_foreign_keys_and_busy_timeout_itself() {
        // The bundled SQLite already defaults foreign_keys to ON and rusqlite
        // already sets a 5 s busy timeout. Start from the opposite, as a
        // system SQLite may, to show that `init` does not rely on either.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.busy_timeout(Duration::ZERO).unwrap();

        let store = Store::init(conn).unwrap();

        let pragma = |name: &str| -> i64 {
            store
                .conn()
                .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(pragma("foreign_keys"), 1);
        assert_eq!(pragma("busy_timeout"), 5000);
    }

    #[test]
    fn in_row_adds_context_to_invalid_errors_only() {
        let invalid = StoreError::Invalid("unknown origin \"x\"".into()).in_row("captures", 7);
        assert_eq!(
            invalid.to_string(),
            "invalid data in the database: captures row 7: unknown origin \"x\""
        );
        let not_found = StoreError::NotFound("capture 7".into()).in_row("captures", 7);
        assert!(matches!(not_found, StoreError::NotFound(message) if message == "capture 7"));
    }
}
