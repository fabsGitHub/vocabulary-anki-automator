//! Duolingo-style mechanics (Milestone I): an XP ledger, a daily streak and
//! streak freezes.
//!
//! All of it is tied to completed review sessions (`daily_sessions` rows with
//! `completed_at`), never to raw captures: clipping fifty words off a web page
//! is not studying. Days are the learner's local calendar days
//! ([`Clock::today`]), so a session finished at 23:30 counts for that evening
//! even where it is already tomorrow in UTC.
//!
//! - **Streak.** The number of consecutive local dates ending today — or
//!   yesterday, while today's session is still open — that have a completed
//!   session or a freeze. A frozen day counts like a completed one.
//! - **Freezes are spent lazily.** Nothing runs at midnight, so missed days are
//!   found the next time the app asks: [`apply_streak_freezes`], which
//!   [`stats_summary`] and [`on_session_completed`] run first, freezes the days
//!   between the last completed or frozen day and yesterday. Today is never
//!   frozen, because the learner can still study. Freezes are spent only when
//!   they cover the whole gap; spending them on a gap that stays open would
//!   lose them without saving the streak.
//! - **Freezes are earned** one at a time, whenever a completed session brings
//!   the streak to a multiple of seven, and at most [`MAX_STREAK_FREEZES`] are
//!   held. The inventory is a number in `app_state`, which both apps share.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::{
    store::{Store, StoreError, StoreResult},
    time::{format_date, local_day_bounds, Clock},
};

pub const XP_REVIEW: i64 = 10;
pub const XP_NEW_ITEM: i64 = 15;
/// Awarded once per local day, for the first completed session.
pub const XP_SESSION_COMPLETE: i64 = 50;
pub const XP_EXERCISE_CORRECT: i64 = 5;
pub const XP_LESSON_COMPLETE: i64 = 20;

/// Most streak freezes held at once.
pub const MAX_STREAK_FREEZES: u32 = 2;
/// A freeze is earned each time the streak reaches a multiple of this.
pub const STREAK_FREEZE_INTERVAL: u32 = 7;
/// `app_state` key holding the number of freezes available.
pub const STREAK_FREEZES_KEY: &str = "streak_freezes_available";

/// `xp_ledger.reason` values (docs/architecture.md § 6.1).
pub mod reason {
    pub const REVIEW: &str = "review";
    pub const NEW_ITEM: &str = "new_item";
    pub const SESSION_COMPLETE: &str = "session_complete";
    pub const EXERCISE: &str = "exercise";
    pub const LESSON: &str = "lesson";
}

/// The learner's streak as of today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreakSummary {
    /// Consecutive covered days ending today, or yesterday while today is
    /// still open; 0 once a day was missed.
    pub current: u32,
    /// The longest run of covered days ever, the current one included.
    pub longest: u32,
    pub freezes_available: u32,
    /// Whether today's daily session is complete.
    pub completed_today: bool,
}

/// Everything the stats screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsSummary {
    pub xp_total: i64,
    /// XP counted towards today's local date.
    pub xp_today: i64,
    pub streak: StreakSummary,
    /// Cards with an `srs_state` row.
    pub learned_count: i64,
    /// Cards due before the end of the local day, overdue ones included.
    pub due_today: i64,
}

/// Records `amount` XP for `reason` (see [`reason`]) on today's local date
/// and returns the ledger row's id.
pub fn award_xp(store: &Store, amount: i64, reason: &str, clock: &dyn Clock) -> StoreResult<i64> {
    store.insert_xp(clock.today(), amount, reason, clock.now())
}

/// `(current, longest)` streak over the dates that have a completed session
/// or a freeze. Dates after `today` (from a clock that was set back) are
/// ignored.
pub fn compute_streak(
    completed: &BTreeSet<NaiveDate>,
    frozen: &BTreeSet<NaiveDate>,
    today: NaiveDate,
) -> (u32, u32) {
    let mut longest = 0;
    let mut run = 0;
    let mut last: Option<NaiveDate> = None;
    for &date in completed.union(frozen).filter(|date| **date <= today) {
        let continues = last.and_then(|last| last.succ_opt()) == Some(date);
        run = if continues { run + 1 } else { 1 };
        longest = u32::max(longest, run);
        last = Some(date);
    }
    let current = match last {
        Some(last) if last == today || last.succ_opt() == Some(today) => run,
        _ => 0,
    };
    (current, longest)
}

/// Freezes the days missed since the last completed or frozen day, up to and
/// including yesterday, and returns them; nothing when there is no gap, when
/// there is nothing before it, or when the available freezes do not cover
/// all of it.
pub fn apply_streak_freezes(store: &Store, clock: &dyn Clock) -> StoreResult<Vec<NaiveDate>> {
    store.with_transaction(|store| {
        let today = clock.today();
        let completed = store.list_completed_session_dates()?;
        let frozen = store.list_streak_freeze_dates()?;
        let Some(last) = completed
            .into_iter()
            .chain(frozen)
            .filter(|date| *date < today)
            .max()
        else {
            return Ok(Vec::new());
        };
        let gap = today.signed_duration_since(last).num_days() - 1;
        let Ok(missed) = u32::try_from(gap) else {
            return Ok(Vec::new());
        };
        if missed == 0 {
            return Ok(Vec::new());
        }
        let held = freezes_available(store)?;
        if missed > held {
            return Ok(Vec::new());
        }
        let days: Vec<NaiveDate> = last.iter_days().skip(1).take(missed as usize).collect();
        for day in &days {
            store.insert_streak_freeze(*day)?;
        }
        set_freezes_available(store, held - missed)?;
        Ok(days)
    })
}

/// To be called once today's daily session has been marked complete. The
/// first call of a local day awards [`XP_SESSION_COMPLETE`] and, when the
/// streak has just reached a multiple of [`STREAK_FREEZE_INTERVAL`], one
/// freeze (unless [`MAX_STREAK_FREEZES`] are already held). Later calls that
/// day, and calls while the session is still open, change nothing.
pub fn on_session_completed(store: &Store, clock: &dyn Clock) -> StoreResult<StreakSummary> {
    store.with_transaction(|store| {
        apply_streak_freezes(store, clock)?;
        let today = clock.today();
        let mut summary = streak_summary(store, today)?;
        if !summary.completed_today || session_bonus_awarded(store, today)? {
            return Ok(summary);
        }
        store.insert_xp(
            today,
            XP_SESSION_COMPLETE,
            reason::SESSION_COMPLETE,
            clock.now(),
        )?;
        if summary.current % STREAK_FREEZE_INTERVAL == 0
            && summary.freezes_available < MAX_STREAK_FREEZES
        {
            summary.freezes_available += 1;
            set_freezes_available(store, summary.freezes_available)?;
        }
        Ok(summary)
    })
}

/// XP, streak and learning counts as of `clock`, after spending any freezes
/// that are due ([`apply_streak_freezes`]).
pub fn stats_summary(store: &Store, clock: &dyn Clock) -> StoreResult<StatsSummary> {
    store.with_transaction(|store| {
        apply_streak_freezes(store, clock)?;
        let today = clock.today();
        let (_, end_of_today) = local_day_bounds(today, clock.local_offset());
        let due_today = store.list_srs_due_before(end_of_today)?.len();
        Ok(StatsSummary {
            xp_total: store.xp_total()?,
            xp_today: store.xp_for_date(today)?,
            streak: streak_summary(store, today)?,
            learned_count: store.count_srs_states()?,
            due_today: i64::try_from(due_today).unwrap_or(i64::MAX),
        })
    })
}

fn streak_summary(store: &Store, today: NaiveDate) -> StoreResult<StreakSummary> {
    let completed: BTreeSet<NaiveDate> =
        store.list_completed_session_dates()?.into_iter().collect();
    let frozen: BTreeSet<NaiveDate> = store.list_streak_freeze_dates()?.into_iter().collect();
    let (current, longest) = compute_streak(&completed, &frozen, today);
    Ok(StreakSummary {
        current,
        longest,
        freezes_available: freezes_available(store)?,
        completed_today: completed.contains(&today),
    })
}

/// Whether the once-a-day session bonus was already recorded for `date`.
fn session_bonus_awarded(store: &Store, date: NaiveDate) -> StoreResult<bool> {
    Ok(store.conn().query_row(
        "SELECT EXISTS (SELECT 1 FROM xp_ledger WHERE session_date = ?1 AND reason = ?2)",
        (format_date(date), reason::SESSION_COMPLETE),
        |row| row.get(0),
    )?)
}

/// The stored inventory, capped at [`MAX_STREAK_FREEZES`]; 0 when unset.
fn freezes_available(store: &Store) -> StoreResult<u32> {
    let Some(text) = store.get_state(STREAK_FREEZES_KEY)? else {
        return Ok(0);
    };
    text.trim()
        .parse::<u32>()
        .map(|held| held.min(MAX_STREAK_FREEZES))
        .map_err(|_| {
            StoreError::Invalid(format!(
                "app_state {STREAK_FREEZES_KEY} is not a count: {text:?}"
            ))
        })
}

fn set_freezes_available(store: &Store, held: u32) -> StoreResult<()> {
    store.set_state(STREAK_FREEZES_KEY, &held.to_string())
}
