//! Versioned schema migrations, tracked in `PRAGMA user_version`.
//!
//! Migration `n` (1-based) turns a version `n - 1` database into version `n`.
//! Each runs in its own `IMMEDIATE` transaction that re-reads `user_version`
//! first, so when Automator and Learn start against an old file at the same
//! moment, the second waits for the first one's step and then skips it. The
//! version bump is part of that transaction: a step either applies
//! completely or not at all.
//!
//! The SQL is append-only. Once a version has shipped, databases in the field
//! have run it, so changing it would leave them on a schema the code no
//! longer describes; schema changes go into a new version instead.

use rusqlite::{Connection, TransactionBehavior};

use super::{StoreError, StoreResult};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 3;

const MIGRATIONS: [&str; SCHEMA_VERSION as usize] = [V1, V2, V3];

/// Milestone F: the capture history.
const V1: &str = "
CREATE TABLE captures (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  front           TEXT NOT NULL,
  back            TEXT NOT NULL,
  example_source  TEXT NOT NULL DEFAULT '',
  example_target  TEXT NOT NULL DEFAULT '',
  notes           TEXT NOT NULL DEFAULT '',
  tags            TEXT NOT NULL DEFAULT '[]',          -- JSON array of strings
  source_lang     TEXT NOT NULL,
  target_lang     TEXT NOT NULL,
  captured_at     TEXT NOT NULL,                       -- RFC 3339 UTC, millis
  synced_to_anki  INTEGER NOT NULL DEFAULT 0 CHECK (synced_to_anki IN (0, 1))
);
CREATE INDEX idx_captures_captured_at ON captures(captured_at);
";

/// Milestones J, K and L: script metadata and origin on captures, FSRS
/// state, the HSK curriculum, daily sessions and app state.
const V2: &str = "
ALTER TABLE captures ADD COLUMN pinyin TEXT;
ALTER TABLE captures ADD COLUMN hsk_level INTEGER;
ALTER TABLE captures ADD COLUMN script_kind TEXT NOT NULL DEFAULT 'none';   -- ScriptMetadata::kind()
ALTER TABLE captures ADD COLUMN script_variant TEXT;                        -- 'simplified' | 'traditional' | NULL
ALTER TABLE captures ADD COLUMN origin TEXT NOT NULL DEFAULT 'capture';     -- CaptureOrigin
ALTER TABLE captures ADD COLUMN audio_front BLOB;
CREATE INDEX idx_captures_script_front ON captures(script_kind, front);

CREATE TABLE srs_state (
  card_id           INTEGER PRIMARY KEY REFERENCES captures(id) ON DELETE CASCADE,
  algorithm         TEXT NOT NULL DEFAULT 'fsrs',
  stability         REAL,
  difficulty        REAL,
  interval_days     INTEGER NOT NULL DEFAULT 0,      -- FSRS scheduled_days
  repetitions       INTEGER NOT NULL DEFAULT 0,      -- FSRS reps
  lapses            INTEGER NOT NULL DEFAULT 0,
  fsrs_state        TEXT NOT NULL DEFAULT 'new',     -- new | learning | review | relearning
  elapsed_days      INTEGER NOT NULL DEFAULT 0,
  due_date          TEXT NOT NULL,                   -- RFC 3339 UTC instant (learning steps are sub-day)
  last_reviewed_at  TEXT,
  last_rating       INTEGER CHECK (last_rating BETWEEN 1 AND 4)
);
CREATE INDEX idx_srs_state_due ON srs_state(due_date);

CREATE TABLE hsk_curriculum (
  hanzi                    TEXT PRIMARY KEY,
  traditional              TEXT,
  hsk_level                INTEGER NOT NULL,
  frequency                INTEGER,
  curriculum_order         INTEGER NOT NULL UNIQUE,
  pinyin                   TEXT NOT NULL,
  pos                      TEXT,
  meaning_en               TEXT,
  meaning_de               TEXT,
  cloze_sentence           TEXT,
  example_sentence_source  TEXT,
  example_sentence_pinyin  TEXT,
  example_sentence_target  TEXT,
  vocab_audio_path         TEXT,
  sentence_audio_path      TEXT,
  image_path               TEXT
);

CREATE TABLE daily_sessions (
  session_date          TEXT PRIMARY KEY,            -- local YYYY-MM-DD
  requested_new_count   INTEGER NOT NULL,
  reviews_due_count     INTEGER NOT NULL,
  reviews_completed     INTEGER NOT NULL DEFAULT 0,
  new_items_completed   INTEGER NOT NULL DEFAULT 0,
  completed_at          TEXT
);

CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// Milestone I: review history, the XP ledger and streak freezes.
const V3: &str = "
CREATE TABLE review_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  card_id         INTEGER NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
  rating          INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
  reviewed_at     TEXT NOT NULL,
  session_date    TEXT NOT NULL,
  state_before    TEXT NOT NULL,
  scheduled_days  INTEGER NOT NULL,
  elapsed_days    INTEGER NOT NULL
);
CREATE INDEX idx_review_log_card ON review_log(card_id);
CREATE TABLE xp_ledger (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_date  TEXT NOT NULL,
  amount        INTEGER NOT NULL,
  reason        TEXT NOT NULL,                       -- review | new_item | session_complete | exercise | lesson
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_xp_ledger_date ON xp_ledger(session_date);
CREATE TABLE streak_freezes (freeze_date TEXT PRIMARY KEY);
";

/// Brings `conn` to [`SCHEMA_VERSION`]. A database that is already there is
/// left alone without opening a transaction.
pub(super) fn migrate(conn: &mut Connection) -> StoreResult<()> {
    if supported_version(conn)? == SCHEMA_VERSION {
        return Ok(());
    }
    for (version, sql) in (1..).zip(MIGRATIONS) {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Another process may have migrated while this one waited for the lock.
        if supported_version(&tx)? >= version {
            continue;
        }
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        log::info!("migrated the database to schema version {version}");
    }
    Ok(())
}

/// The database's `user_version`, or [`StoreError::Invalid`] when this build
/// cannot work with it.
pub(super) fn supported_version(conn: &Connection) -> StoreResult<u32> {
    let found: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match u32::try_from(found) {
        Ok(version) if version <= SCHEMA_VERSION => Ok(version),
        _ => Err(StoreError::Invalid(format!(
            "schema version {found} is not one this build supports (0 to {SCHEMA_VERSION}); \
             a newer app version may have upgraded the database"
        ))),
    }
}
