//! FSRS scheduling state (`srs_state`, Milestone K) and the review history
//! (`review_log`, Milestone I).
//!
//! `srs_state` is one row per card but its own table, so the scheduling
//! algorithm can change without touching card content, and because having a
//! row here is what "learned" means. Its columns mirror rs-fsrs's `Card` so
//! that a state survives a round trip through the scheduler (§ 12 item 3).

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{named_params, Row};
use serde::{Deserialize, Serialize};

use super::{codec, Store, StoreResult};

const SELECT_SRS_STATE: &str = "SELECT card_id, algorithm, stability, difficulty, \
     interval_days, repetitions, lapses, fsrs_state, elapsed_days, due_date, \
     last_reviewed_at, last_rating FROM srs_state";

const UPSERT_SRS_STATE: &str = "INSERT INTO srs_state (card_id, algorithm, stability, \
     difficulty, interval_days, repetitions, lapses, fsrs_state, elapsed_days, due_date, \
     last_reviewed_at, last_rating) VALUES (:card_id, :algorithm, :stability, :difficulty, \
     :interval_days, :repetitions, :lapses, :fsrs_state, :elapsed_days, :due_date, \
     :last_reviewed_at, :last_rating) \
     ON CONFLICT (card_id) DO UPDATE SET algorithm = excluded.algorithm, \
     stability = excluded.stability, difficulty = excluded.difficulty, \
     interval_days = excluded.interval_days, repetitions = excluded.repetitions, \
     lapses = excluded.lapses, fsrs_state = excluded.fsrs_state, \
     elapsed_days = excluded.elapsed_days, due_date = excluded.due_date, \
     last_reviewed_at = excluded.last_reviewed_at, last_rating = excluded.last_rating";

/// The scheduling state of one card (`srs_state`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SrsStateRow {
    /// The `captures.id` this state belongs to.
    pub card_id: i64,
    /// The algorithm that produced the state; `"fsrs"`.
    pub algorithm: String,
    /// FSRS memory stability; `None` before the first review.
    pub stability: Option<f64>,
    /// FSRS difficulty; `None` before the first review.
    pub difficulty: Option<f64>,
    /// Interval set by the last review, in days (FSRS `scheduled_days`).
    pub interval_days: i64,
    /// Number of reviews (FSRS `reps`).
    pub repetitions: i64,
    /// How often the card was forgotten after being learned.
    pub lapses: i64,
    /// `new`, `learning`, `review` or `relearning`.
    pub fsrs_state: String,
    /// Days between the two most recent reviews (FSRS `elapsed_days`).
    pub elapsed_days: i64,
    /// When the card is next due. An instant rather than a date, because
    /// learning steps are minutes apart.
    pub due_date: DateTime<Utc>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
    /// The last rating given: 1 = Again, 2 = Hard, 3 = Good, 4 = Easy.
    pub last_rating: Option<u8>,
}

/// One answered review (`review_log`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewLogRow {
    /// Assigned by the store; ignored by [`Store::insert_review_log`].
    pub id: i64,
    pub card_id: i64,
    /// 1 = Again, 2 = Hard, 3 = Good, 4 = Easy.
    pub rating: u8,
    pub reviewed_at: DateTime<Utc>,
    /// The local date of the daily session the review counted towards.
    pub session_date: NaiveDate,
    /// `fsrs_state` before this review.
    pub state_before: String,
    /// Interval this review scheduled, in days.
    pub scheduled_days: i64,
    /// Days since the previous review.
    pub elapsed_days: i64,
}

impl Store {
    pub fn get_srs_state(&self, card_id: i64) -> StoreResult<Option<SrsStateRow>> {
        let mut statement = self
            .conn()
            .prepare_cached(&format!("{SELECT_SRS_STATE} WHERE card_id = ?1"))?;
        let mut rows = statement.query_and_then([card_id], srs_state_from_row)?;
        rows.next().transpose()
    }

    /// Inserts or replaces the state of `row.card_id`. The card must exist
    /// in `captures` (foreign key), otherwise this fails.
    pub fn upsert_srs_state(&self, row: &SrsStateRow) -> StoreResult<()> {
        let due_date = codec::format_timestamp(row.due_date)?;
        let last_reviewed_at = row
            .last_reviewed_at
            .map(codec::format_timestamp)
            .transpose()?;
        self.conn()
            .prepare_cached(UPSERT_SRS_STATE)?
            .execute(named_params! {
                ":card_id": row.card_id,
                ":algorithm": row.algorithm,
                ":stability": row.stability,
                ":difficulty": row.difficulty,
                ":interval_days": row.interval_days,
                ":repetitions": row.repetitions,
                ":lapses": row.lapses,
                ":fsrs_state": row.fsrs_state,
                ":elapsed_days": row.elapsed_days,
                ":due_date": due_date,
                ":last_reviewed_at": last_reviewed_at,
                ":last_rating": row.last_rating,
            })?;
        Ok(())
    }

    /// Every state due strictly before `before`, ordered by `due_date` and
    /// then `card_id`.
    pub fn list_srs_due_before(&self, before: DateTime<Utc>) -> StoreResult<Vec<SrsStateRow>> {
        let mut statement = self.conn().prepare_cached(&format!(
            "{SELECT_SRS_STATE} WHERE due_date < ?1 ORDER BY due_date ASC, card_id ASC"
        ))?;
        let rows = statement
            .query_and_then([codec::format_exclusive_bound(before)], srs_state_from_row)?;
        rows.collect()
    }

    /// The number of learned cards.
    pub fn count_srs_states(&self) -> StoreResult<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM srs_state", [], |row| row.get(0))?)
    }

    /// Appends a review and returns its id; `row.id` is ignored.
    pub fn insert_review_log(&self, row: &ReviewLogRow) -> StoreResult<i64> {
        let reviewed_at = codec::format_timestamp(row.reviewed_at)?;
        let session_date = codec::format_date(row.session_date)?;
        Ok(self
            .conn()
            .prepare_cached(
                "INSERT INTO review_log (card_id, rating, reviewed_at, session_date, \
                 state_before, scheduled_days, elapsed_days) VALUES (:card_id, :rating, \
                 :reviewed_at, :session_date, :state_before, :scheduled_days, :elapsed_days)",
            )?
            .insert(named_params! {
                ":card_id": row.card_id,
                ":rating": row.rating,
                ":reviewed_at": reviewed_at,
                ":session_date": session_date,
                ":state_before": row.state_before,
                ":scheduled_days": row.scheduled_days,
                ":elapsed_days": row.elapsed_days,
            })?)
    }

    /// The reviews of one card, oldest first.
    pub fn review_log_for_card(&self, card_id: i64) -> StoreResult<Vec<ReviewLogRow>> {
        let mut statement = self.conn().prepare_cached(
            "SELECT id, card_id, rating, reviewed_at, session_date, state_before, \
             scheduled_days, elapsed_days FROM review_log WHERE card_id = ?1 \
             ORDER BY reviewed_at ASC, id ASC",
        )?;
        let rows = statement.query_and_then([card_id], review_log_from_row)?;
        rows.collect()
    }
}

fn srs_state_from_row(row: &Row<'_>) -> StoreResult<SrsStateRow> {
    let card_id: i64 = row.get("card_id")?;
    let decode = || -> StoreResult<SrsStateRow> {
        Ok(SrsStateRow {
            card_id,
            algorithm: row.get("algorithm")?,
            stability: row.get("stability")?,
            difficulty: row.get("difficulty")?,
            interval_days: row.get("interval_days")?,
            repetitions: row.get("repetitions")?,
            lapses: row.get("lapses")?,
            fsrs_state: row.get("fsrs_state")?,
            elapsed_days: row.get("elapsed_days")?,
            due_date: codec::parse_timestamp(&row.get::<_, String>("due_date")?)?,
            last_reviewed_at: codec::parse_optional_timestamp(row.get("last_reviewed_at")?)?,
            last_rating: row.get("last_rating")?,
        })
    };
    decode().map_err(|err| err.in_row("srs_state", card_id))
}

fn review_log_from_row(row: &Row<'_>) -> StoreResult<ReviewLogRow> {
    let id: i64 = row.get("id")?;
    let decode = || -> StoreResult<ReviewLogRow> {
        Ok(ReviewLogRow {
            id,
            card_id: row.get("card_id")?,
            rating: row.get("rating")?,
            reviewed_at: codec::parse_timestamp(&row.get::<_, String>("reviewed_at")?)?,
            session_date: codec::parse_date(&row.get::<_, String>("session_date")?)?,
            state_before: row.get("state_before")?,
            scheduled_days: row.get("scheduled_days")?,
            elapsed_days: row.get("elapsed_days")?,
        })
    };
    decode().map_err(|err| err.in_row("review_log", id))
}
