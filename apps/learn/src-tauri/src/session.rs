//! The day's review session (Milestones L2 and L3) as the review screen
//! drives it, on top of `vocab_core::srs` and `vocab_core::gamification`.
//!
//! The core decides how progress, completion, XP and streaks are counted
//! (docs/architecture.md § 7.1, § 7.3); this module only combines its calls
//! the way a session needs them:
//!
//! - **A rating is one transaction**: the FSRS review, its XP, the session
//!   progress it counts as and, when it completes the session, the
//!   completion bonus and the streak. A crash can never keep a review without
//!   its progress, or XP for a review that was rolled back.
//! - **Progress is counted on reviews.** A card's first review (its log's
//!   `state_before` is `new`) counts as a new item and earns `XP_NEW_ITEM`;
//!   every other review counts as a review and earns `XP_REVIEW`. Introducing
//!   a word (`start_new_item`) counts nothing, so introducing it twice cannot
//!   count twice.
//! - **Completion is noticed by the write that causes it**: a rating, or an
//!   answer to the prompt that lowers the quota to what is already done.
//!   [`track_completion`] compares today's session before and after the
//!   write and calls `gamification::on_session_completed` exactly when
//!   `completed_at` first appears.
//! - **The queue offers what is left of today's quota**: the session's
//!   `requested_new_count`, minus the new items already done, minus the cards
//!   still waiting for their first review (introduced but not rated yet,
//!   whose rating will count as a new item). Introducing a word therefore
//!   moves it from the new part of the queue to the review part instead of
//!   pulling in one word more. Before today's prompt is answered there is no
//!   session, and the queue holds reviews only, as after a dismissed prompt.
//!   This rule is Learn's, not the core's, so the web build's adapter has to
//!   apply the same one for both platforms to show the same queue.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use vocab_core::{
    gamification::{
        award_xp, on_session_completed, reason, stats_summary, StreakSummary, XP_NEW_ITEM,
        XP_REVIEW,
    },
    srs::{
        build_daily_queue_from, record_session_progress, submit_review, QueueItem, ReviewRating,
        SessionProgress, SrsScheduler,
    },
    store::{DailySessionRow, SrsStateRow, Store, StoreResult},
    time::{local_day_bounds, Clock},
};

/// `fsrs_state` of a card that has never been reviewed.
const NEVER_REVIEWED: &str = "new";

/// What a rating did (`ReviewResult` in apps/learn/src/platform/types.ts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewResult {
    /// The card's scheduling state after the review.
    pub next: SrsStateRow,
    /// XP this rating earned, the session-completion bonus included.
    pub xp_awarded: i64,
    /// Today's session after counting the review.
    pub session: Option<DailySessionRow>,
    /// True only for the rating that completed today's session.
    pub session_completed: bool,
    /// The streak after this rating, a freeze it earned included.
    pub streak: StreakSummary,
}

/// Today's queue: every card due before the end of the local day, then the
/// new curriculum words that are left of today's quota (see the module docs).
pub fn daily_queue(store: &Store, clock: &dyn Clock) -> StoreResult<Vec<QueueItem>> {
    let today = clock.today();
    let (_, end_of_today) = local_day_bounds(today, clock.local_offset());
    let due = store.list_srs_due_before(end_of_today)?;
    let new_quota = remaining_new_quota(store.get_daily_session(today)?.as_ref(), &due);
    let curriculum = store.curriculum_in_order()?;
    let learned_hanzi = store.learned_chinese_fronts()?;
    Ok(build_daily_queue_from(
        &due,
        &curriculum,
        &learned_hanzi,
        new_quota,
    ))
}

/// How many new curriculum words today's queue still offers, given the
/// states due today.
fn remaining_new_quota(session: Option<&DailySessionRow>, due: &[SrsStateRow]) -> u32 {
    let Some(session) = session else {
        return 0;
    };
    let waiting = due
        .iter()
        .filter(|state| state.fsrs_state == NEVER_REVIEWED)
        .count();
    session
        .requested_new_count
        .saturating_sub(session.new_items_completed)
        .saturating_sub(u32::try_from(waiting).unwrap_or(u32::MAX))
}

/// Runs `write`, a core call that saves and returns today's session, and
/// lets gamification award the completion bonus if `write` is what completed
/// the session. Returns the session and, in that case, the streak after the
/// bonus.
pub fn track_completion(
    store: &Store,
    clock: &dyn Clock,
    write: impl FnOnce(&Store) -> StoreResult<DailySessionRow>,
) -> StoreResult<(DailySessionRow, Option<StreakSummary>)> {
    store.with_transaction(|store| {
        let before = store.get_daily_session(clock.today())?;
        let session = write(store)?;
        let streak = if just_completed(before.as_ref(), &session) {
            Some(on_session_completed(store, clock)?)
        } else {
            None
        };
        Ok((session, streak))
    })
}

/// Whether `after` is the first save of its session that has `completed_at`.
fn just_completed(before: Option<&DailySessionRow>, after: &DailySessionRow) -> bool {
    let was_completed = before.is_some_and(|before| {
        before.session_date == after.session_date && before.completed_at.is_some()
    });
    after.completed_at.is_some() && !was_completed
}

/// Rates the scheduled card `card_id` and does everything the rating counts
/// for, in one transaction (see the module docs). Fails, writing nothing,
/// when the core refuses the review (`StoreError::NotFound` for a card
/// without `srs_state`).
pub fn rate(
    store: &Store,
    scheduler: &SrsScheduler,
    card_id: i64,
    rating: ReviewRating,
    clock: &dyn Clock,
) -> StoreResult<ReviewResult> {
    store.with_transaction(|store| {
        let today = clock.today();
        let xp_before = store.xp_for_date(today)?;
        let outcome = submit_review(store, scheduler, card_id, rating, clock)?;
        let (progress, xp, why) = if outcome.log.state_before == NEVER_REVIEWED {
            (SessionProgress::NewItem, XP_NEW_ITEM, reason::NEW_ITEM)
        } else {
            (SessionProgress::Review, XP_REVIEW, reason::REVIEW)
        };
        award_xp(store, xp, why, clock)?;
        let (session, completed) = track_completion(store, clock, |store| {
            record_session_progress(store, progress, clock)
        })?;
        let session_completed = completed.is_some();
        let streak = match completed {
            Some(streak) => streak,
            None => stats_summary(store, clock)?.streak,
        };
        Ok(ReviewResult {
            next: outcome.next,
            xp_awarded: xp_earned_since(store, today, xp_before)?,
            session: Some(session),
            session_completed,
            streak,
        })
    })
}

/// XP recorded for `date` beyond `before`: what the current transaction
/// awarded, whichever core call it came from.
fn xp_earned_since(store: &Store, date: NaiveDate, before: i64) -> StoreResult<i64> {
    Ok(store.xp_for_date(date)? - before)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use vocab_core::{
        gamification::XP_SESSION_COMPLETE,
        srs::{dismiss_daily_prompt, introduce_new_item, submit_daily_quota},
        time::FixedClock,
        CaptureOrigin, Card, CurriculumItem,
    };

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 9, 0, 0).unwrap()
    }

    fn word(hanzi: &str, order: i64) -> CurriculumItem {
        CurriculumItem {
            hanzi: hanzi.into(),
            traditional: None,
            hsk_level: 1,
            frequency: None,
            curriculum_order: order,
            pinyin: "x".into(),
            pos: None,
            meaning_en: Some(format!("meaning of {hanzi}")),
            meaning_de: None,
            cloze_sentence: None,
            example_sentence_source: None,
            example_sentence_pinyin: None,
            example_sentence_target: None,
            vocab_audio_path: None,
            sentence_audio_path: None,
            image_path: None,
        }
    }

    fn store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .replace_curriculum(&[word("一", 0), word("二", 1), word("三", 2), word("四", 3)])
            .unwrap();
        store
    }

    fn new_items(queue: &[QueueItem]) -> Vec<&str> {
        queue
            .iter()
            .filter_map(|item| match item {
                QueueItem::New { hanzi, .. } => Some(hanzi.as_str()),
                QueueItem::Review { .. } => None,
            })
            .collect()
    }

    #[test]
    fn the_queue_offers_what_is_left_of_the_quota() {
        let store = store();
        let clock = FixedClock::utc(now());
        let scheduler = SrsScheduler::default();
        assert!(new_items(&daily_queue(&store, &clock).unwrap()).is_empty());

        submit_daily_quota(&store, 2, &clock).unwrap();
        assert_eq!(
            new_items(&daily_queue(&store, &clock).unwrap()),
            ["一", "二"]
        );

        // Introduced, not rated yet: it waits among the reviews, and the new
        // part does not grow to make up for it.
        let first = introduce_new_item(&store, &scheduler, "一", "en", &clock).unwrap();
        let queue = daily_queue(&store, &clock).unwrap();
        assert_eq!(new_items(&queue), ["二"]);
        assert!(queue.contains(&QueueItem::Review {
            card_id: first,
            due_date: now()
        }));

        rate(&store, &scheduler, first, ReviewRating::Good, &clock).unwrap();
        assert_eq!(new_items(&daily_queue(&store, &clock).unwrap()), ["二"]);

        let second = introduce_new_item(&store, &scheduler, "二", "en", &clock).unwrap();
        rate(&store, &scheduler, second, ReviewRating::Good, &clock).unwrap();
        assert!(new_items(&daily_queue(&store, &clock).unwrap()).is_empty());
    }

    #[test]
    fn a_dismissed_prompt_offers_no_new_words() {
        let store = store();
        let clock = FixedClock::utc(now());
        dismiss_daily_prompt(&store, &clock).unwrap();
        assert!(daily_queue(&store, &clock).unwrap().is_empty());
    }

    #[test]
    fn the_rating_that_completes_the_session_earns_the_bonus_once() {
        let store = store();
        let clock = FixedClock::utc(now());
        let scheduler = SrsScheduler::default();
        let card = Card::new("Haus", "house", "de", "en", now());
        let reviewed = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();
        store
            .upsert_srs_state(&scheduler.initial_state(reviewed, now()))
            .unwrap();
        // The first rating turns the card from new into learning; the session
        // needs one review on top (reviews_due_count is 1).
        submit_daily_quota(&store, 0, &clock).unwrap();

        let first = rate(&store, &scheduler, reviewed, ReviewRating::Again, &clock).unwrap();
        assert_eq!(first.xp_awarded, XP_NEW_ITEM);
        assert!(!first.session_completed);
        let session = first.session.unwrap();
        assert_eq!(
            (session.new_items_completed, session.reviews_completed),
            (1, 0)
        );

        let second = rate(&store, &scheduler, reviewed, ReviewRating::Good, &clock).unwrap();
        assert_eq!(second.xp_awarded, XP_REVIEW + XP_SESSION_COMPLETE);
        assert!(second.session_completed);
        assert_eq!(second.streak.current, 1);
        assert!(second.streak.completed_today);

        let third = rate(&store, &scheduler, reviewed, ReviewRating::Good, &clock).unwrap();
        assert_eq!(third.xp_awarded, XP_REVIEW, "the bonus is paid once a day");
        assert!(!third.session_completed);
        assert_eq!(third.streak, second.streak);
    }

    #[test]
    fn a_refused_review_writes_nothing() {
        let store = store();
        let clock = FixedClock::utc(now());
        let card = Card::new("Haus", "house", "de", "en", now());
        let unscheduled = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();

        assert!(rate(
            &store,
            &SrsScheduler::default(),
            unscheduled,
            ReviewRating::Good,
            &clock
        )
        .is_err());
        assert_eq!(store.xp_total().unwrap(), 0);
        assert_eq!(store.get_daily_session(clock.today()).unwrap(), None);
    }

    #[test]
    fn completion_is_judged_per_session_date() {
        let open = DailySessionRow {
            session_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            requested_new_count: 1,
            reviews_due_count: 0,
            reviews_completed: 0,
            new_items_completed: 0,
            completed_at: None,
        };
        let done = DailySessionRow {
            new_items_completed: 1,
            completed_at: Some(now()),
            ..open.clone()
        };
        assert!(just_completed(Some(&open), &done));
        assert!(just_completed(None, &done));
        assert!(!just_completed(Some(&done), &done));
        assert!(!just_completed(Some(&open), &open));
        let yesterday = DailySessionRow {
            session_date: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
            ..done.clone()
        };
        assert!(just_completed(Some(&yesterday), &done));
    }
}
