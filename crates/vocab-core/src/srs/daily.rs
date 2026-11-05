//! The daily onboarding prompt and the day's session (Milestone L2).
//!
//! Learn asks "how many new words do you want to learn today?" once per
//! local calendar day, before its main view renders. Whether today has been
//! asked lives in the shared database (`app_state` [`LAST_PROMPTED_KEY`]), so
//! the answer survives restarts; the answer itself is today's
//! `daily_sessions` row, which also counts the day's progress.
//!
//! Decisions recorded here (docs/architecture.md § 7.1):
//!
//! - **Dismissing the prompt without answering means "reviews only today"**:
//!   a session with a new-item quota of 0, and no further prompt that day. The
//!   menu action [`reopen_daily_prompt`] shows the prompt again; a later answer
//!   then replaces the quota and keeps the progress made so far. Dismissing a
//!   reopened prompt keeps whatever the day's session already says.
//! - **The day's targets are fixed when the session is set up.**
//!   `reviews_due_count` is the number of reviews in the queue at that moment;
//!   changing the quota later does not recount it, because the reviews done
//!   since would otherwise be counted against a smaller remainder. A due card
//!   that was never rated (introduced on an earlier day, not rated since) is
//!   not a review: its first rating counts as a new item.
//! - **A session is complete as soon as both targets are met and at least
//!   one review or new word was done**, whichever write gets there first:
//!   usually a progress update, but also lowering the quota to no more than is
//!   done. A day with nothing due and no new words asked for (or a dismissed
//!   prompt on such a day) never completes: completion feeds the streak and
//!   the session XP bonus, which are tied to actual review work, not to
//!   opening the app. `completed_at` records the first time and is never
//!   cleared, so asking for more words after finishing does not undo the day.
//! - **Progress before the day's prompt still counts.** A review made after
//!   midnight in a session started the evening before opens the new day's
//!   session with quota 0, without answering the prompt for the user.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::{queue::due_today, LocalNow};
use crate::{
    store::{DailySessionRow, Store, StoreResult},
    time::{self, Clock},
};

/// `app_state` key holding the local date (`YYYY-MM-DD`) on which the daily
/// prompt was last answered or dismissed.
pub const LAST_PROMPTED_KEY: &str = "last_prompted_date";

/// `fsrs_state` of a card that has never been rated.
const NEVER_REVIEWED: &str = "new";

/// Whether the daily prompt is due: it has not been answered or dismissed on
/// the local date `today` (or ever).
pub fn should_prompt(last_prompted: Option<NaiveDate>, today: NaiveDate) -> bool {
    last_prompted != Some(today)
}

/// What Learn's main window needs before it renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyPromptState {
    /// Show the prompt before anything else.
    pub needs_prompt: bool,
    /// The user's local date.
    pub today: NaiveDate,
    /// Today's session, once it has been set up.
    pub session: Option<DailySessionRow>,
}

/// Whether to prompt now, and today's session if there is one. A stored
/// prompt date that does not parse counts as never prompted: the next answer
/// overwrites it.
pub fn daily_prompt_state(store: &Store, clock: &dyn Clock) -> StoreResult<DailyPromptState> {
    let today = LocalNow::read(clock).today;
    Ok(DailyPromptState {
        needs_prompt: should_prompt(last_prompted(store)?, today),
        today,
        session: store.get_daily_session(today)?,
    })
}

/// Records the answer to the prompt: today's session asks for `new_count`
/// new items, and the prompt is not shown again today. The first answer of a
/// day sets the session up, with `reviews_due_count` = the reviews in the
/// queue right now; a later one changes only `requested_new_count`.
pub fn submit_daily_quota(
    store: &Store,
    new_count: u32,
    clock: &dyn Clock,
) -> StoreResult<DailySessionRow> {
    let now = LocalNow::read(clock);
    store.with_transaction(|store| {
        let session = match store.get_daily_session(now.today)? {
            Some(existing) => DailySessionRow {
                requested_new_count: new_count,
                ..existing
            },
            None => open_session(store, &now, new_count)?,
        };
        let session = save_session(store, session, now.at)?;
        mark_prompted(store, now.today)?;
        Ok(session)
    })
}

/// Records closing the prompt without an answer: reviews only today (a new
/// quota of 0), unless today's session was already set up, which then stays
/// as it is. Either way the prompt is not shown again today.
pub fn dismiss_daily_prompt(store: &Store, clock: &dyn Clock) -> StoreResult<DailySessionRow> {
    let now = LocalNow::read(clock);
    store.with_transaction(|store| {
        let session = match store.get_daily_session(now.today)? {
            Some(existing) => existing,
            None => open_session(store, &now, 0)?,
        };
        let session = save_session(store, session, now.at)?;
        mark_prompted(store, now.today)?;
        Ok(session)
    })
}

/// The menu action "set today's goal again": clears [`LAST_PROMPTED_KEY`], so
/// that [`daily_prompt_state`] asks again. Today's session is kept for the
/// next answer to update. The flag is cleared whatever date it holds, so the
/// clock is not consulted.
pub fn reopen_daily_prompt(store: &Store, _clock: &dyn Clock) -> StoreResult<()> {
    store.delete_state(LAST_PROMPTED_KEY)
}

/// The two kinds of progress a session counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionProgress {
    /// A review was submitted.
    Review,
    /// A new curriculum item was introduced.
    NewItem,
}

/// Counts one review or new item towards today's session and returns the
/// session. `completed_at` is set by the update that first meets both
/// targets. Without a session yet (the day changed mid-session), today's is
/// set up with a quota of 0, and the prompt is still shown.
pub fn record_session_progress(
    store: &Store,
    kind: SessionProgress,
    clock: &dyn Clock,
) -> StoreResult<DailySessionRow> {
    let now = LocalNow::read(clock);
    store.with_transaction(|store| {
        let mut session = match store.get_daily_session(now.today)? {
            Some(existing) => existing,
            None => open_session(store, &now, 0)?,
        };
        match kind {
            SessionProgress::Review => {
                session.reviews_completed = session.reviews_completed.saturating_add(1);
            }
            SessionProgress::NewItem => {
                session.new_items_completed = session.new_items_completed.saturating_add(1);
            }
        }
        save_session(store, session, now.at)
    })
}

fn last_prompted(store: &Store) -> StoreResult<Option<NaiveDate>> {
    let stored = store.get_state(LAST_PROMPTED_KEY)?;
    let date = stored.as_deref().and_then(time::parse_date);
    if let (Some(text), None) = (&stored, date) {
        log::warn!("ignoring {LAST_PROMPTED_KEY} {text:?}: not a YYYY-MM-DD date");
    }
    Ok(date)
}

fn mark_prompted(store: &Store, today: NaiveDate) -> StoreResult<()> {
    store.set_state(LAST_PROMPTED_KEY, &time::format_date(today))
}

/// A fresh session for today, with the reviews currently due as its target.
/// A due card that was never rated (a word introduced on an earlier day and
/// left on screen) is not one of them: its first rating counts as a new item
/// (§ 7.1), so as a review target it could never be met.
fn open_session(
    store: &Store,
    now: &LocalNow,
    requested_new_count: u32,
) -> StoreResult<DailySessionRow> {
    let reviews_due = due_today(store, now)?
        .iter()
        .filter(|state| state.fsrs_state != NEVER_REVIEWED)
        .count();
    Ok(DailySessionRow {
        session_date: now.today,
        requested_new_count,
        reviews_due_count: u32::try_from(reviews_due).unwrap_or(u32::MAX),
        reviews_completed: 0,
        new_items_completed: 0,
        completed_at: None,
    })
}

/// Stores `session`, completing it at `at` if this write meets both targets
/// for the first time and some work was done.
fn save_session(
    store: &Store,
    mut session: DailySessionRow,
    at: DateTime<Utc>,
) -> StoreResult<DailySessionRow> {
    let targets_met = session.reviews_completed >= session.reviews_due_count
        && session.new_items_completed >= session.requested_new_count;
    let did_work = session.reviews_completed > 0 || session.new_items_completed > 0;
    if targets_met && did_work && session.completed_at.is_none() {
        session.completed_at = Some(at);
    }
    store.upsert_daily_session(&session)?;
    Ok(session)
}
