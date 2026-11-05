//! Spaced-repetition scheduling (Milestone L): FSRS state transitions, the
//! daily queue builder and the once-per-day onboarding prompt
//! (docs/architecture.md § 7.1).
//!
//! - [`SrsScheduler`] (L1) adapts rs-fsrs to the store's `srs_state` rows. It
//!   is pure, taking the instant and the session date as arguments, so the G+
//!   tests can replay rs-fsrs's own reference cases through it.
//! - [`build_daily_queue`] (L3) lists every review due today before any new
//!   item: the captures and imports nobody has started, then curriculum
//!   words. [`introduce_capture`], [`introduce_new_item`] and
//!   [`submit_review`] are the writes a review session makes with the queue's
//!   items.
//! - [`daily_prompt_state`] and its siblings (L2) decide when Learn asks "how
//!   many new words today?" and keep the answer in `daily_sessions`.
//!
//! Everything that needs "now" or "today" takes a [`Clock`] and reads it once
//! per call, so an instant and the local date it falls on always agree, even
//! at midnight. Instants are cut to whole milliseconds before they are used,
//! because that is all the store keeps: anything computed from a finer
//! instant would no longer match itself once written and read back.

mod daily;
mod queue;
mod scheduler;

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};

pub use daily::{
    daily_prompt_state, dismiss_daily_prompt, record_session_progress, reopen_daily_prompt,
    should_prompt, submit_daily_quota, DailyPromptState, SessionProgress, LAST_PROMPTED_KEY,
};
pub use queue::{
    build_daily_queue, build_daily_queue_from, introduce_capture, introduce_new_item,
    submit_review, QueueItem,
};
pub use scheduler::{
    fsrs_card_from_state, state_from_fsrs_card, InvalidRating, RatingPreview, ReviewOutcome,
    ReviewRating, SrsScheduler,
};

use crate::time::{self, Clock};

/// `at` truncated to a whole millisecond, the precision the store keeps.
fn whole_millis(at: DateTime<Utc>) -> DateTime<Utc> {
    let nanos = at.timestamp_subsec_nanos();
    at.with_nanosecond(nanos - nanos % 1_000_000).unwrap_or(at)
}

/// One reading of a [`Clock`]: the instant, in whole milliseconds, and the
/// user's local date and UTC offset at that instant.
struct LocalNow {
    at: DateTime<Utc>,
    today: NaiveDate,
    offset: FixedOffset,
}

impl LocalNow {
    fn read(clock: &dyn Clock) -> Self {
        let offset = clock.local_offset();
        let at = whole_millis(clock.now());
        Self {
            at,
            today: at.with_timezone(&offset).date_naive(),
            offset,
        }
    }

    /// The first instant of the next local day.
    fn end_of_today(&self) -> DateTime<Utc> {
        time::local_day_bounds(self.today, self.offset).1
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::time::FixedClock;

    #[test]
    fn whole_millis_truncates_and_keeps_whole_milliseconds() {
        let base = Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap();
        let cases = [
            (base, base),
            (
                base + Duration::nanoseconds(123_456_789),
                base + Duration::milliseconds(123),
            ),
            (
                base + Duration::nanoseconds(999_999_999),
                base + Duration::milliseconds(999),
            ),
            (
                base + Duration::milliseconds(5),
                base + Duration::milliseconds(5),
            ),
            (
                DateTime::<Utc>::MAX_UTC,
                DateTime::<Utc>::MAX_UTC - Duration::nanoseconds(999_999),
            ),
            (DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MIN_UTC),
        ];
        for (at, expected) in cases {
            assert_eq!(whole_millis(at), expected, "{at:?}");
        }
    }

    #[test]
    fn local_now_puts_the_instant_on_the_local_calendar() {
        // 04:30 UTC on Nov 1 is still Oct 31 in New York (UTC-5).
        let now = Utc.with_ymd_and_hms(2026, 11, 1, 4, 30, 0).unwrap();
        let clock = FixedClock::new(
            now + Duration::nanoseconds(1_500),
            FixedOffset::west_opt(5 * 3600).unwrap(),
        );
        let local = LocalNow::read(&clock);
        assert_eq!(local.at, now);
        assert_eq!(local.today, NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        assert_eq!(
            local.end_of_today(),
            Utc.with_ymd_and_hms(2026, 11, 1, 5, 0, 0).unwrap()
        );
    }
}
