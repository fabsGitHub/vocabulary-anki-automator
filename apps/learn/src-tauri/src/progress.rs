//! XP outside review sessions (Milestones I and M): correct quiz answers and
//! completed lessons. The amounts and reasons are vocab-core's
//! (`gamification::XP_*`, `gamification::reason::*`); what this module adds
//! is who may earn them.
//!
//! - **Exercises** earn `XP_EXERCISE_CORRECT` for a correct answer about a
//!   learned card and nothing otherwise. A card that is not learned (no
//!   `srs_state` row, the store's definition, § 6.1) has no exercises, so a
//!   result for it is refused rather than paid.
//! - **Lessons** earn `XP_LESSON_COMPLETE` once per lesson per local day.
//!   The day of each lesson's last paid completion is kept in `app_state`
//!   under [`lesson_key`], because the XP ledger records only a reason, not
//!   which lesson. Only lessons that exist ([`lessons_from_store`] with
//!   [`CARDS_PER_LESSON`]) can be completed.

use vocab_core::{
    gamification::{award_xp, reason, XP_EXERCISE_CORRECT, XP_LESSON_COMPLETE},
    lessons::lessons_from_store,
    store::Store,
    time::{format_date, Clock},
};

use crate::error::Error;

/// Cards per lesson. Lesson ids name their first card, so this also decides
/// the ids; the web build must use the same number.
pub const CARDS_PER_LESSON: usize = 5;

/// Prefix of the `app_state` keys that remember lesson completions.
pub const LESSON_COMPLETED_PREFIX: &str = "learn.lesson_completed.";

/// `app_state` key holding the local date (`YYYY-MM-DD`) on which the lesson
/// `lesson_id` last earned XP.
pub fn lesson_key(lesson_id: &str) -> String {
    format!("{LESSON_COMPLETED_PREFIX}{lesson_id}")
}

/// Records the result of an exercise about `card_id`; returns the XP
/// awarded.
pub fn record_exercise(
    store: &Store,
    card_id: i64,
    correct: bool,
    clock: &dyn Clock,
) -> Result<i64, Error> {
    if store.get_srs_state(card_id)?.is_none() {
        return Err(Error::NotLearned(card_id));
    }
    if !correct {
        return Ok(0);
    }
    award_xp(store, XP_EXERCISE_CORRECT, reason::EXERCISE, clock)?;
    Ok(XP_EXERCISE_CORRECT)
}

/// Records that the learner finished lesson `lesson_id`; returns the XP
/// awarded, which is 0 when the lesson already earned it today.
pub fn complete_lesson(store: &Store, lesson_id: &str, clock: &dyn Clock) -> Result<i64, Error> {
    let exists = lessons_from_store(store, CARDS_PER_LESSON)?
        .iter()
        .any(|lesson| lesson.id == lesson_id);
    if !exists {
        return Err(Error::NoSuchLesson(lesson_id.to_string()));
    }
    let key = lesson_key(lesson_id);
    let today = format_date(clock.today());
    Ok(store.with_transaction(|store| {
        if store.get_state(&key)?.as_deref() == Some(today.as_str()) {
            return Ok(0);
        }
        award_xp(store, XP_LESSON_COMPLETE, reason::LESSON, clock)?;
        store.set_state(&key, &today)?;
        Ok(XP_LESSON_COMPLETE)
    })?)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, FixedOffset, TimeZone, Utc};
    use vocab_core::{srs::SrsScheduler, time::FixedClock, CaptureOrigin, Card};

    use super::*;

    fn clock() -> FixedClock {
        FixedClock::utc(Utc.with_ymd_and_hms(2026, 10, 31, 9, 0, 0).unwrap())
    }

    #[test]
    fn only_correct_answers_about_learned_cards_earn_xp() {
        let store = Store::open_in_memory().unwrap();
        let clock = clock();
        let card = Card::new("Haus", "house", "de", "en", clock.now());
        let learned = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();
        let unlearned = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();
        store
            .upsert_srs_state(&SrsScheduler::default().initial_state(learned, clock.now()))
            .unwrap();

        assert_eq!(record_exercise(&store, learned, true, &clock).unwrap(), 5);
        assert_eq!(record_exercise(&store, learned, false, &clock).unwrap(), 0);
        assert!(matches!(
            record_exercise(&store, unlearned, true, &clock),
            Err(Error::NotLearned(id)) if id == unlearned
        ));
        assert_eq!(store.xp_total().unwrap(), 5);
    }

    #[test]
    fn a_lesson_pays_once_per_day() {
        let store = Store::open_in_memory().unwrap();
        let clock = clock();
        let mut card = Card::new("serendipity", "Zufall", "en", "de", clock.now());
        card.example_source = "Pure serendipity.".into();
        card.example_target = "Reiner Zufall.".into();
        let id = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();
        let lesson = format!("lesson-{id}");

        assert_eq!(complete_lesson(&store, &lesson, &clock).unwrap(), 20);
        assert_eq!(complete_lesson(&store, &lesson, &clock).unwrap(), 0);
        assert_eq!(
            store.get_state(&lesson_key(&lesson)).unwrap().as_deref(),
            Some("2026-10-31")
        );
        clock.advance(Duration::days(1));
        assert_eq!(complete_lesson(&store, &lesson, &clock).unwrap(), 20);
        assert_eq!(store.xp_total().unwrap(), 40);

        assert!(matches!(
            complete_lesson(&store, "lesson-999", &clock),
            Err(Error::NoSuchLesson(_))
        ));
    }

    /// "Once a day" means the learner's day: in Berlin in winter, 23:30 and
    /// 00:30 are two days, though both fall on October 31 in UTC.
    #[test]
    fn a_lesson_day_is_the_local_day() {
        let store = Store::open_in_memory().unwrap();
        let clock = FixedClock::new(
            Utc.with_ymd_and_hms(2026, 10, 31, 22, 30, 0).unwrap(),
            FixedOffset::east_opt(3600).unwrap(),
        );
        let mut card = Card::new("serendipity", "Zufall", "en", "de", clock.now());
        card.example_source = "Pure serendipity.".into();
        card.example_target = "Reiner Zufall.".into();
        let id = store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap();
        let lesson = format!("lesson-{id}");

        assert_eq!(complete_lesson(&store, &lesson, &clock).unwrap(), 20);
        clock.advance(Duration::hours(1));
        assert_eq!(
            complete_lesson(&store, &lesson, &clock).unwrap(),
            20,
            "a new local day"
        );
        assert_eq!(
            store.get_state(&lesson_key(&lesson)).unwrap().as_deref(),
            Some("2026-11-01")
        );
    }
}
