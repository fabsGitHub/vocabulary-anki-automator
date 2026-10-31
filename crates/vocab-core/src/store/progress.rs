//! Daily sessions (Milestone L2), the XP ledger and streak freezes
//! (Milestone I): what the learner did on which local date.
//!
//! All three are keyed by the local calendar date (`YYYY-MM-DD`) rather than
//! an instant, because "today", streaks and per-day XP are about the
//! learner's calendar, not UTC.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{named_params, Row};
use serde::{Deserialize, Serialize};

use super::{codec, Store, StoreResult};

/// One local day's review session (`daily_sessions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailySessionRow {
    pub session_date: NaiveDate,
    /// New items the learner asked for; 0 means reviews only.
    pub requested_new_count: u32,
    /// Reviews due when the session was set up.
    pub reviews_due_count: u32,
    pub reviews_completed: u32,
    pub new_items_completed: u32,
    /// When both targets were first met; `None` while the session is open.
    pub completed_at: Option<DateTime<Utc>>,
}

impl Store {
    pub fn get_daily_session(&self, date: NaiveDate) -> StoreResult<Option<DailySessionRow>> {
        let mut statement = self.conn().prepare_cached(
            "SELECT session_date, requested_new_count, reviews_due_count, reviews_completed, \
             new_items_completed, completed_at FROM daily_sessions WHERE session_date = ?1",
        )?;
        let mut rows =
            statement.query_and_then([codec::format_date(date)], daily_session_from_row)?;
        rows.next().transpose()
    }

    /// Inserts the session for `row.session_date` or replaces every field of
    /// the existing one.
    pub fn upsert_daily_session(&self, row: &DailySessionRow) -> StoreResult<()> {
        self.conn()
            .prepare_cached(
                "INSERT INTO daily_sessions (session_date, requested_new_count, \
                 reviews_due_count, reviews_completed, new_items_completed, completed_at) \
                 VALUES (:session_date, :requested_new_count, :reviews_due_count, \
                 :reviews_completed, :new_items_completed, :completed_at) \
                 ON CONFLICT (session_date) DO UPDATE SET \
                 requested_new_count = excluded.requested_new_count, \
                 reviews_due_count = excluded.reviews_due_count, \
                 reviews_completed = excluded.reviews_completed, \
                 new_items_completed = excluded.new_items_completed, \
                 completed_at = excluded.completed_at",
            )?
            .execute(named_params! {
                ":session_date": codec::format_date(row.session_date),
                ":requested_new_count": row.requested_new_count,
                ":reviews_due_count": row.reviews_due_count,
                ":reviews_completed": row.reviews_completed,
                ":new_items_completed": row.new_items_completed,
                ":completed_at": row.completed_at.map(codec::format_timestamp),
            })?;
        Ok(())
    }

    /// The dates of every completed session, oldest first.
    pub fn list_completed_session_dates(&self) -> StoreResult<Vec<NaiveDate>> {
        self.list_dates(
            "SELECT session_date FROM daily_sessions WHERE completed_at IS NOT NULL \
             ORDER BY session_date ASC",
        )
    }

    /// Appends an XP entry and returns its id. `date` is the local date the
    /// XP counts towards, `at` the instant it was earned.
    pub fn insert_xp(
        &self,
        date: NaiveDate,
        amount: i64,
        reason: &str,
        at: DateTime<Utc>,
    ) -> StoreResult<i64> {
        Ok(self
            .conn()
            .prepare_cached(
                "INSERT INTO xp_ledger (session_date, amount, reason, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .insert((
                codec::format_date(date),
                amount,
                reason,
                codec::format_timestamp(at),
            ))?)
    }

    pub fn xp_total(&self) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM xp_ledger",
            [],
            |row| row.get(0),
        )?)
    }

    /// XP counted towards the local date `date`: entries are matched by the
    /// date they were recorded for, not by when they were earned.
    pub fn xp_for_date(&self, date: NaiveDate) -> StoreResult<i64> {
        Ok(self.conn().query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM xp_ledger WHERE session_date = ?1",
            [codec::format_date(date)],
            |row| row.get(0),
        )?)
    }

    /// Marks `date` as covered by a streak freeze; marking it again is a no-op.
    pub fn insert_streak_freeze(&self, date: NaiveDate) -> StoreResult<()> {
        self.conn().execute(
            "INSERT INTO streak_freezes (freeze_date) VALUES (?1) \
             ON CONFLICT (freeze_date) DO NOTHING",
            [codec::format_date(date)],
        )?;
        Ok(())
    }

    /// Every frozen date, oldest first.
    pub fn list_streak_freeze_dates(&self) -> StoreResult<Vec<NaiveDate>> {
        self.list_dates("SELECT freeze_date FROM streak_freezes ORDER BY freeze_date ASC")
    }

    fn list_dates(&self, sql: &str) -> StoreResult<Vec<NaiveDate>> {
        let mut statement = self.conn().prepare_cached(sql)?;
        let dates =
            statement.query_and_then([], |row| codec::parse_date(&row.get::<_, String>(0)?))?;
        dates.collect()
    }
}

fn daily_session_from_row(row: &Row<'_>) -> StoreResult<DailySessionRow> {
    let session_date: String = row.get("session_date")?;
    let decode = || -> StoreResult<DailySessionRow> {
        Ok(DailySessionRow {
            session_date: codec::parse_date(&session_date)?,
            requested_new_count: row.get("requested_new_count")?,
            reviews_due_count: row.get("reviews_due_count")?,
            reviews_completed: row.get("reviews_completed")?,
            new_items_completed: row.get("new_items_completed")?,
            completed_at: codec::parse_optional_timestamp(row.get("completed_at")?)?,
        })
    };
    decode().map_err(|err| err.in_row("daily_sessions", &session_date))
}
