//! XP, streaks and streak freezes (docs/architecture.md § 7.3, Milestone I).
//!
//! The clocks run in Tokyo (UTC+9): from midnight to 09:00 local time the UTC
//! date is still yesterday's, so code that used UTC dates for "today" would
//! fail here.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use vocab_core::{
    gamification::{
        apply_streak_freezes, award_xp, compute_streak, on_session_completed, reason,
        stats_summary, StatsSummary, StreakSummary, MAX_STREAK_FREEZES, STREAK_FREEZES_KEY,
        XP_SESSION_COMPLETE,
    },
    store::{DailySessionRow, SrsStateRow, Store, StoreError},
    time::{Clock, FixedClock},
    CaptureOrigin, Card,
};

fn tokyo() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("valid test date")
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

/// A Tokyo clock at `hh:mm` local time on `date`.
fn tokyo_clock(date: &str, hour: u32, minute: u32) -> FixedClock {
    let local = day(date).and_hms_opt(hour, minute, 0).unwrap();
    let now = tokyo()
        .from_local_datetime(&local)
        .unwrap()
        .with_timezone(&Utc);
    FixedClock::new(now, tokyo())
}

fn dates(texts: &[&str]) -> BTreeSet<NaiveDate> {
    texts.iter().map(|text| day(text)).collect()
}

/// Marks the session of `date` complete, as the daily queue does.
fn complete(store: &Store, date: NaiveDate) {
    store
        .upsert_daily_session(&DailySessionRow {
            session_date: date,
            requested_new_count: 3,
            reviews_due_count: 5,
            reviews_completed: 5,
            new_items_completed: 3,
            completed_at: Some(at("2026-11-01T00:00:00.000Z")),
        })
        .unwrap();
}

fn open_session(store: &Store, date: NaiveDate) {
    store
        .upsert_daily_session(&DailySessionRow {
            session_date: date,
            requested_new_count: 3,
            reviews_due_count: 5,
            reviews_completed: 1,
            new_items_completed: 0,
            completed_at: None,
        })
        .unwrap();
}

fn freezes(store: &Store) -> Option<String> {
    store.get_state(STREAK_FREEZES_KEY).unwrap()
}

fn give_freezes(store: &Store, count: u32) {
    store
        .set_state(STREAK_FREEZES_KEY, &count.to_string())
        .unwrap();
}

/// Session-complete ledger entries per local date.
fn session_bonuses(store: &Store, date: NaiveDate) -> i64 {
    store.xp_for_date(date).unwrap() / XP_SESSION_COMPLETE
}

mod streaks {
    use super::*;

    #[test]
    fn compute_streak_counts_consecutive_covered_days_ending_today_or_yesterday() {
        let today = "2026-11-20";
        type Case<'a> = (&'a str, &'a [&'a str], &'a [&'a str], (u32, u32));
        let cases: [Case; 13] = [
            ("nothing yet", &[], &[], (0, 0)),
            ("today only", &["2026-11-20"], &[], (1, 1)),
            ("yesterday, today still open", &["2026-11-19"], &[], (1, 1)),
            (
                "yesterday and the day before",
                &["2026-11-18", "2026-11-19"],
                &[],
                (2, 2),
            ),
            ("two days ago is broken", &["2026-11-18"], &[], (0, 1)),
            (
                "a gap ends the earlier run",
                &[
                    "2026-11-15",
                    "2026-11-16",
                    "2026-11-17",
                    "2026-11-19",
                    "2026-11-20",
                ],
                &[],
                (2, 3),
            ),
            (
                "a freeze bridges a missed day",
                &["2026-11-17", "2026-11-18", "2026-11-20"],
                &["2026-11-19"],
                (4, 4),
            ),
            (
                "two freezes bridge two days",
                &["2026-11-16", "2026-11-19"],
                &["2026-11-17", "2026-11-18"],
                (4, 4),
            ),
            (
                "a freeze counts like a completed day",
                &["2026-11-18"],
                &["2026-11-19"],
                (2, 2),
            ),
            (
                "the longest run may lie in the past",
                &[
                    "2026-10-01",
                    "2026-10-02",
                    "2026-10-03",
                    "2026-10-04",
                    "2026-10-05",
                    "2026-11-19",
                    "2026-11-20",
                ],
                &["2026-10-06"],
                (2, 6),
            ),
            (
                "a day both completed and frozen counts once",
                &["2026-11-19", "2026-11-20"],
                &["2026-11-19"],
                (2, 2),
            ),
            (
                "days after today are ignored",
                &["2026-11-20", "2026-11-21", "2026-11-22"],
                &["2026-11-23"],
                (1, 1),
            ),
            (
                "a run across a month boundary",
                &["2026-10-30", "2026-10-31", "2026-11-01"],
                &[],
                (0, 3),
            ),
        ];
        for (name, completed, frozen, expected) in cases {
            assert_eq!(
                compute_streak(&dates(completed), &dates(frozen), day(today)),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn compute_streak_handles_year_boundaries() {
        let completed = dates(&["2026-12-30", "2026-12-31", "2027-01-01"]);
        assert_eq!(
            compute_streak(&completed, &BTreeSet::new(), day("2027-01-01")),
            (3, 3)
        );
        assert_eq!(
            compute_streak(&completed, &BTreeSet::new(), day("2027-01-02")),
            (3, 3)
        );
        assert_eq!(
            compute_streak(&completed, &BTreeSet::new(), day("2027-01-03")),
            (0, 3)
        );
    }
}

mod freezes {
    use super::*;

    #[test]
    fn a_missed_day_is_frozen_when_a_freeze_is_available() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-08"));
        complete(&store, day("2026-11-09"));
        give_freezes(&store, 1);
        // 08:30 in Tokyo on Nov 11 is still Nov 10 in UTC.
        let clock = tokyo_clock("2026-11-11", 8, 30);
        assert_eq!(clock.now().date_naive(), day("2026-11-10"));

        assert_eq!(
            apply_streak_freezes(&store, &clock).unwrap(),
            [day("2026-11-10")]
        );
        assert_eq!(
            store.list_streak_freeze_dates().unwrap(),
            [day("2026-11-10")]
        );
        assert_eq!(freezes(&store).as_deref(), Some("0"));
        // The gap is closed: a second call spends nothing.
        assert_eq!(apply_streak_freezes(&store, &clock).unwrap(), []);
        assert_eq!(freezes(&store).as_deref(), Some("0"));
    }

    #[test]
    fn two_freezes_bridge_two_missed_days() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-08"));
        give_freezes(&store, 2);
        let clock = tokyo_clock("2026-11-11", 12, 0);
        assert_eq!(
            apply_streak_freezes(&store, &clock).unwrap(),
            [day("2026-11-09"), day("2026-11-10")]
        );
        assert_eq!(freezes(&store).as_deref(), Some("0"));
        let summary = stats_summary(&store, &clock).unwrap().streak;
        assert_eq!((summary.current, summary.completed_today), (3, false));
    }

    #[test]
    fn freezes_are_kept_when_they_cannot_close_the_whole_gap() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-07"));
        give_freezes(&store, 2);
        let clock = tokyo_clock("2026-11-11", 12, 0);
        assert_eq!(apply_streak_freezes(&store, &clock).unwrap(), []);
        assert!(store.list_streak_freeze_dates().unwrap().is_empty());
        assert_eq!(freezes(&store).as_deref(), Some("2"));
    }

    #[test]
    fn today_is_never_frozen() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-10"));
        give_freezes(&store, 2);
        // Yesterday was completed, today is still open: nothing to bridge.
        let clock = tokyo_clock("2026-11-11", 23, 59);
        assert_eq!(apply_streak_freezes(&store, &clock).unwrap(), []);
        // The next morning, only the day that is over gets frozen.
        clock.advance(Duration::hours(10));
        assert_eq!(clock.today(), day("2026-11-12"));
        assert_eq!(
            apply_streak_freezes(&store, &clock).unwrap(),
            [day("2026-11-11")]
        );
        assert_eq!(freezes(&store).as_deref(), Some("1"));
    }

    #[test]
    fn a_missed_day_before_todays_session_is_frozen_too() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-09"));
        complete(&store, day("2026-11-11"));
        give_freezes(&store, 1);
        let clock = tokyo_clock("2026-11-11", 20, 0);
        assert_eq!(
            apply_streak_freezes(&store, &clock).unwrap(),
            [day("2026-11-10")]
        );
        let summary = stats_summary(&store, &clock).unwrap().streak;
        assert_eq!(
            (summary.current, summary.longest, summary.completed_today),
            (3, 3, true)
        );
    }

    #[test]
    fn nothing_is_spent_without_freezes_or_without_a_streak() {
        let store = Store::open_in_memory().unwrap();
        let clock = tokyo_clock("2026-11-11", 12, 0);
        give_freezes(&store, 2);
        assert_eq!(apply_streak_freezes(&store, &clock).unwrap(), []);
        assert_eq!(freezes(&store).as_deref(), Some("2"));

        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-09"));
        assert_eq!(apply_streak_freezes(&store, &clock).unwrap(), []);
        assert_eq!(freezes(&store), None);
        assert_eq!(stats_summary(&store, &clock).unwrap().streak.current, 0);
    }

    #[test]
    fn an_open_session_does_not_count_as_a_streak_day() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-08"));
        open_session(&store, day("2026-11-09"));
        give_freezes(&store, 2);
        let clock = tokyo_clock("2026-11-10", 12, 0);
        assert_eq!(
            apply_streak_freezes(&store, &clock).unwrap(),
            [day("2026-11-09")]
        );
    }

    #[test]
    fn a_stored_inventory_that_is_not_a_count_is_an_error() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-09"));
        store.set_state(STREAK_FREEZES_KEY, "two").unwrap();
        let clock = tokyo_clock("2026-11-11", 12, 0);
        assert!(matches!(
            apply_streak_freezes(&store, &clock),
            Err(StoreError::Invalid(message)) if message.contains(STREAK_FREEZES_KEY)
        ));
        // A count above the cap is read as the cap.
        store.set_state(STREAK_FREEZES_KEY, " 9 ").unwrap();
        assert_eq!(
            stats_summary(&store, &clock)
                .unwrap()
                .streak
                .freezes_available,
            1
        );
    }
}

mod sessions {
    use super::*;

    #[test]
    fn completing_a_session_awards_the_bonus_once_per_local_day() {
        let store = Store::open_in_memory().unwrap();
        // 07:00 in Tokyo on Nov 11: Nov 10 in UTC.
        let clock = tokyo_clock("2026-11-11", 7, 0);
        complete(&store, day("2026-11-11"));

        let summary = on_session_completed(&store, &clock).unwrap();
        assert_eq!(
            summary,
            StreakSummary {
                current: 1,
                longest: 1,
                freezes_available: 0,
                completed_today: true,
            }
        );
        assert_eq!(
            store.xp_for_date(day("2026-11-11")).unwrap(),
            XP_SESSION_COMPLETE
        );
        assert_eq!(store.xp_for_date(day("2026-11-10")).unwrap(), 0);

        clock.advance(Duration::hours(16));
        assert_eq!(clock.today(), day("2026-11-11"));
        assert_eq!(on_session_completed(&store, &clock).unwrap(), summary);
        assert_eq!(store.xp_total().unwrap(), XP_SESSION_COMPLETE);
    }

    #[test]
    fn an_open_session_earns_nothing() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-10"));
        open_session(&store, day("2026-11-11"));
        let clock = tokyo_clock("2026-11-11", 12, 0);
        let summary = on_session_completed(&store, &clock).unwrap();
        assert_eq!((summary.current, summary.completed_today), (1, false));
        assert_eq!(store.xp_total().unwrap(), 0);
    }

    #[test]
    fn other_xp_on_the_same_day_does_not_count_as_the_bonus() {
        let store = Store::open_in_memory().unwrap();
        let clock = tokyo_clock("2026-11-11", 12, 0);
        award_xp(&store, 10, reason::REVIEW, &clock).unwrap();
        award_xp(&store, 50, reason::LESSON, &clock).unwrap();
        complete(&store, day("2026-11-11"));
        on_session_completed(&store, &clock).unwrap();
        assert_eq!(store.xp_for_date(day("2026-11-11")).unwrap(), 110);
    }

    #[test]
    fn every_seventh_day_of_a_streak_earns_a_freeze_up_to_the_cap() {
        let store = Store::open_in_memory().unwrap();
        let first = day("2026-11-01");
        let mut held = Vec::new();
        for offset in 0..22 {
            let date = first + Duration::days(offset);
            let clock = tokyo_clock(&date.to_string(), 21, 30);
            complete(&store, date);
            let summary = on_session_completed(&store, &clock).unwrap();
            assert_eq!(summary.current, offset as u32 + 1);
            // Calling again on the milestone day earns nothing more.
            assert_eq!(on_session_completed(&store, &clock).unwrap(), summary);
            held.push(summary.freezes_available);
            assert_eq!(session_bonuses(&store, date), 1, "{date}");
        }
        let mut expected = vec![0; 6];
        expected.extend([1; 7]);
        expected.extend([2; 9]);
        assert_eq!(held, expected);
        assert_eq!(freezes(&store).as_deref(), Some("2"));
        assert_eq!(MAX_STREAK_FREEZES, 2);
    }

    #[test]
    fn a_freeze_earned_after_a_week_saves_the_streak_from_one_missed_day() {
        let store = Store::open_in_memory().unwrap();
        let first = day("2026-11-01");
        for offset in 0..7 {
            let date = first + Duration::days(offset);
            complete(&store, date);
            on_session_completed(&store, &tokyo_clock(&date.to_string(), 20, 0)).unwrap();
        }
        // Nov 8 is missed; Nov 9's session uses the freeze.
        complete(&store, day("2026-11-09"));
        let summary = on_session_completed(&store, &tokyo_clock("2026-11-09", 20, 0)).unwrap();
        assert_eq!(
            store.list_streak_freeze_dates().unwrap(),
            [day("2026-11-08")]
        );
        assert_eq!(
            summary,
            StreakSummary {
                current: 9,
                longest: 9,
                freezes_available: 0,
                completed_today: true,
            }
        );
    }
}

mod xp_and_stats {
    use super::*;

    fn learned_card(store: &Store, due: &str) -> i64 {
        let card = Card::new(
            "学习",
            "lernen",
            "zh-CN",
            "de",
            at("2026-10-01T00:00:00.000Z"),
        );
        let id = store
            .insert_capture(&card, CaptureOrigin::Curriculum, false)
            .unwrap();
        store
            .upsert_srs_state(&SrsStateRow {
                card_id: id,
                algorithm: "fsrs".into(),
                stability: Some(1.0),
                difficulty: Some(5.0),
                interval_days: 1,
                repetitions: 1,
                lapses: 0,
                fsrs_state: "review".into(),
                elapsed_days: 0,
                due_date: at(due),
                last_reviewed_at: None,
                last_rating: Some(3),
            })
            .unwrap();
        id
    }

    #[test]
    fn award_xp_records_the_local_date_and_the_instant() {
        let store = Store::open_in_memory().unwrap();
        // 20:00 UTC on Oct 31 is 05:00 on Nov 1 in Tokyo.
        let clock = FixedClock::new(at("2026-10-31T20:00:00.000Z"), tokyo());
        let first = award_xp(&store, 10, reason::REVIEW, &clock).unwrap();
        let second = award_xp(&store, 5, reason::EXERCISE, &clock).unwrap();
        assert!(second > first);
        assert_eq!(store.xp_for_date(day("2026-11-01")).unwrap(), 15);
        assert_eq!(store.xp_for_date(day("2026-10-31")).unwrap(), 0);
        assert_eq!(store.xp_total().unwrap(), 15);
    }

    #[test]
    fn stats_count_learned_cards_and_what_is_due_before_local_midnight() {
        let store = Store::open_in_memory().unwrap();
        // Nov 11 in Tokyo ends at 15:00 UTC.
        let clock = tokyo_clock("2026-11-11", 9, 0);
        learned_card(&store, "2026-11-01T00:00:00.000Z"); // overdue
        learned_card(&store, "2026-11-11T14:59:59.999Z"); // 23:59:59.999 local
        learned_card(&store, "2026-11-11T15:00:00.000Z"); // local midnight
        learned_card(&store, "2026-12-01T00:00:00.000Z");
        let only_captured = Card::new(
            "中文",
            "Chinesisch",
            "zh-CN",
            "de",
            at("2026-10-01T00:00:00.000Z"),
        );
        store
            .insert_capture(&only_captured, CaptureOrigin::Capture, false)
            .unwrap();
        award_xp(
            &store,
            15,
            reason::NEW_ITEM,
            &tokyo_clock("2026-11-10", 12, 0),
        )
        .unwrap();
        award_xp(&store, 10, reason::REVIEW, &clock).unwrap();
        complete(&store, day("2026-11-10"));
        complete(&store, day("2026-11-11"));

        assert_eq!(
            stats_summary(&store, &clock).unwrap(),
            StatsSummary {
                xp_total: 25,
                xp_today: 10,
                streak: StreakSummary {
                    current: 2,
                    longest: 2,
                    freezes_available: 0,
                    completed_today: true,
                },
                learned_count: 4,
                due_today: 2,
            }
        );
    }

    #[test]
    fn stats_spend_freezes_that_are_due_before_reporting_the_streak() {
        let store = Store::open_in_memory().unwrap();
        complete(&store, day("2026-11-09"));
        give_freezes(&store, 1);
        let stats = stats_summary(&store, &tokyo_clock("2026-11-11", 6, 0)).unwrap();
        assert_eq!(
            store.list_streak_freeze_dates().unwrap(),
            [day("2026-11-10")]
        );
        assert_eq!(
            stats.streak,
            StreakSummary {
                current: 2,
                longest: 2,
                freezes_available: 0,
                completed_today: false,
            }
        );
    }

    #[test]
    fn summaries_serialize_with_snake_case_fields() {
        let stats = StatsSummary {
            xp_total: 120,
            xp_today: 15,
            streak: StreakSummary {
                current: 3,
                longest: 9,
                freezes_available: 1,
                completed_today: false,
            },
            learned_count: 42,
            due_today: 7,
        };
        assert_eq!(
            serde_json::to_value(stats).unwrap(),
            serde_json::json!({
                "xp_total": 120,
                "xp_today": 15,
                "streak": {
                    "current": 3,
                    "longest": 9,
                    "freezes_available": 1,
                    "completed_today": false
                },
                "learned_count": 42,
                "due_today": 7
            })
        );
    }
}
