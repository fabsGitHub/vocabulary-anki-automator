//! G+ tests for FSRS scheduling (`vocab_core::srs`, docs/architecture.md
//! § 7.1, Milestone L1).
//!
//! The reference cases are rs-fsrs 1.2.1's own published tests (the crate's
//! `src/tests.rs`), replayed through `SrsScheduler` and `submit_review` instead
//! of rs-fsrs's API. The card's state lives in an in-memory store and is read
//! back before every single review, so a lossy row conversion, an instant the
//! store cannot keep, or a field `submit_review` forgets to write turns into a
//! wrong upstream number. Time only ever comes from a `FixedClock`.

use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use rs_fsrs::{Parameters, State, FSRS};
use vocab_core::{
    srs::{
        fsrs_card_from_state, state_from_fsrs_card, submit_review, InvalidRating, RatingPreview,
        ReviewOutcome, ReviewRating, SrsScheduler,
    },
    store::{ReviewLogRow, SrsStateRow, Store, StoreError},
    time::{Clock, FixedClock},
    CaptureOrigin, Card,
};
use ReviewRating::{Again, Easy, Good, Hard};

/// rs-fsrs's `TEST_RATINGS`.
const TEST_RATINGS: [ReviewRating; 13] = [
    Good, Good, Good, Good, Good, Good, Again, Again, Good, Good, Good, Good, Good,
];

/// rs-fsrs's custom `WEIGHTS`.
const WEIGHTS: [f64; 19] = [
    0.4197, 1.1869, 3.0412, 15.2441, 7.1434, 0.6477, 1.0007, 0.0674, 1.6597, 0.1712, 1.1178,
    2.0225, 0.0904, 0.3025, 2.1214, 0.2498, 2.9466, 0.4891, 0.6468,
];

/// Where every rs-fsrs reference case starts: a new card at this instant.
fn reference_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2022, 11, 29, 12, 30, 0).unwrap()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("valid test date")
}

fn with_weights() -> Parameters {
    Parameters {
        w: WEIGHTS,
        ..Parameters::default()
    }
}

fn long_term_with_weights() -> Parameters {
    Parameters {
        enable_short_term: false,
        ..with_weights()
    }
}

/// rs-fsrs's `round_float`.
fn round(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

fn insert_card(store: &Store, front: &str) -> i64 {
    let card = Card::new(front, "Bedeutung", "zh-CN", "de", reference_start());
    store
        .insert_capture(&card, CaptureOrigin::Curriculum, false)
        .expect("insert capture")
}

/// A reviewed card's state, valid for FSRS, for tests that edit one field.
fn review_state(card_id: i64) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(12.5),
        difficulty: Some(6.25),
        interval_days: 9,
        repetitions: 4,
        lapses: 1,
        fsrs_state: "review".into(),
        elapsed_days: 3,
        due_date: at("2026-11-09T08:00:00.000Z"),
        last_reviewed_at: Some(at("2026-10-31T08:00:00.000Z")),
        last_rating: Some(3),
    }
}

/// One card in an in-memory store, rated only through `submit_review`.
struct Replay {
    store: Store,
    scheduler: SrsScheduler,
    clock: FixedClock,
    card_id: i64,
}

impl Replay {
    fn new(scheduler: SrsScheduler) -> Self {
        Self::with_clock(scheduler, FixedClock::utc(reference_start()))
    }

    fn with_clock(scheduler: SrsScheduler, clock: FixedClock) -> Self {
        let store = Store::open_in_memory().expect("in-memory store");
        let card_id = insert_card(&store, "学习");
        store
            .upsert_srs_state(&scheduler.initial_state(card_id, clock.now()))
            .expect("store the initial state");
        Self {
            store,
            scheduler,
            clock,
            card_id,
        }
    }

    fn stored(&self) -> SrsStateRow {
        self.store
            .get_srs_state(self.card_id)
            .expect("read the state")
            .expect("the card has a state")
    }

    /// Rates the stored state at the clock's instant through `submit_review`
    /// and checks that the store now holds exactly what was computed, and that
    /// rating the reloaded state in memory computes the same.
    fn review(&self, rating: ReviewRating) -> ReviewOutcome {
        let before = self.stored();
        let outcome = submit_review(
            &self.store,
            &self.scheduler,
            self.card_id,
            rating,
            &self.clock,
        )
        .expect("submit_review");
        assert_eq!(
            self.stored(),
            outcome.next,
            "the stored state differs from the computed one"
        );
        let in_memory =
            self.scheduler
                .review(&before, rating, self.clock.now(), self.clock.today());
        assert_eq!(in_memory.next, outcome.next);
        assert_eq!(
            ReviewLogRow {
                id: outcome.log.id,
                ..in_memory.log
            },
            outcome.log
        );
        outcome
    }

    fn preview(&self) -> Vec<RatingPreview> {
        self.scheduler.preview(&self.stored(), self.clock.now())
    }
}

/// The preview entry for `rating` announced exactly what the review did.
fn assert_preview_matches(
    previews: &[RatingPreview],
    outcome: &ReviewOutcome,
    rating: ReviewRating,
) {
    let ratings: Vec<ReviewRating> = previews.iter().map(|preview| preview.rating).collect();
    assert_eq!(
        ratings,
        ReviewRating::ALL,
        "one preview per rating, in order"
    );
    let preview = &previews[usize::from(u8::from(rating)) - 1];
    assert_eq!(
        (preview.due_date, preview.interval_days),
        (outcome.next.due_date, outcome.next.interval_days),
        "preview of {rating:?}"
    );
}

mod rs_fsrs_reference_cases {
    use super::*;

    /// rs-fsrs `test_basic_scheduler_interval`.
    #[test]
    fn basic_scheduler_intervals() {
        let replay = Replay::new(SrsScheduler::default());
        let mut intervals = Vec::new();
        for rating in TEST_RATINGS {
            let outcome = replay.review(rating);
            intervals.push(outcome.next.interval_days);
            replay.clock.set(outcome.next.due_date);
        }
        assert_eq!(intervals, [0, 4, 15, 48, 136, 351, 0, 0, 7, 13, 24, 43, 77]);
    }

    /// rs-fsrs `test_basic_scheduler_state`: the state each review started
    /// from, as the review log records it.
    #[test]
    fn basic_scheduler_states_before_each_review() {
        let replay = Replay::new(SrsScheduler::new(with_weights()));
        let mut states_before = Vec::new();
        for rating in TEST_RATINGS {
            let previews = replay.preview();
            let outcome = replay.review(rating);
            assert_preview_matches(&previews, &outcome, rating);
            states_before.push(outcome.log.state_before);
            replay.clock.set(outcome.next.due_date);
        }
        assert_eq!(
            states_before,
            [
                "new",
                "learning",
                "review",
                "review",
                "review",
                "review",
                "review",
                "relearning",
                "relearning",
                "review",
                "review",
                "review",
                "review"
            ]
        );
    }

    /// rs-fsrs `test_basic_scheduler_memo_state`: reviews at fixed gaps rather
    /// than at the due dates, then the memory state of a final Good.
    #[test]
    fn memory_state_after_reviews_at_fixed_gaps() {
        let replay = Replay::new(SrsScheduler::new(with_weights()));
        let ratings = [Again, Good, Good, Good, Good, Good];
        let gaps_in_days = [0, 0, 1, 3, 8, 21];
        for (rating, gap) in ratings.into_iter().zip(gaps_in_days) {
            let previews = replay.preview();
            let outcome = replay.review(rating);
            assert_preview_matches(&previews, &outcome, rating);
            replay.clock.advance(Duration::days(gap));
        }
        let previews = replay.preview();
        let last = replay.review(Good);
        assert_preview_matches(&previews, &last, Good);
        assert_eq!(round(last.next.stability.unwrap(), 4), 71.4554);
        assert_eq!(round(last.next.difficulty.unwrap(), 4), 5.0976);
    }

    /// rs-fsrs `test_long_term_scheduler`.
    #[test]
    fn long_term_scheduler_intervals_stability_and_difficulty() {
        let replay = Replay::new(SrsScheduler::new(long_term_with_weights()));
        let (mut intervals, mut stabilities, mut difficulties) = (vec![], vec![], vec![]);
        for rating in TEST_RATINGS {
            let previews = replay.preview();
            let outcome = replay.review(rating);
            assert_preview_matches(&previews, &outcome, rating);
            intervals.push(outcome.next.interval_days);
            stabilities.push(round(outcome.next.stability.unwrap(), 4));
            difficulties.push(round(outcome.next.difficulty.unwrap(), 4));
            replay.clock.set(outcome.next.due_date);
        }
        assert_eq!(
            intervals,
            [3, 13, 48, 155, 445, 1158, 17, 3, 9, 27, 74, 190, 457]
        );
        assert_eq!(
            stabilities,
            [
                3.0412, 13.0913, 48.1585, 154.9373, 445.0556, 1158.0778, 16.6306, 2.9888, 9.4633,
                26.9474, 73.9723, 189.7037, 457.4379
            ]
        );
        assert_eq!(
            difficulties,
            [
                4.4909, 4.2666, 4.0575, 3.8624, 3.6804, 3.5108, 5.219, 6.8122, 6.4314, 6.0763,
                5.7452, 5.4363, 5.1483
            ]
        );
    }

    /// rs-fsrs `test_get_retrievability`: each first rating's card, stored and
    /// read back, recalled at its own due date.
    #[test]
    fn retrievability_of_each_first_rating_at_its_due_date() {
        let scheduler = SrsScheduler::default();
        let store = Store::open_in_memory().unwrap();
        let clock = FixedClock::utc(reference_start());
        let expected = [1.0, 1.0, 1.0, 0.902_620_8];
        let mut retrievability = Vec::new();
        for (index, rating) in ReviewRating::ALL.into_iter().enumerate() {
            let card_id = insert_card(&store, &format!("word {index}"));
            store
                .upsert_srs_state(&scheduler.initial_state(card_id, clock.now()))
                .unwrap();
            let previews =
                scheduler.preview(&store.get_srs_state(card_id).unwrap().unwrap(), clock.now());
            let outcome = submit_review(&store, &scheduler, card_id, rating, &clock).unwrap();
            assert_preview_matches(&previews, &outcome, rating);

            let stored = store.get_srs_state(card_id).unwrap().unwrap();
            assert_eq!(stored, outcome.next);
            let card = fsrs_card_from_state(&stored).unwrap();
            retrievability.push(round(card.get_retrievability(card.due), 7));
        }
        assert_eq!(retrievability, expected);
    }

    /// Beyond the published numbers: every field of every state matches bare
    /// rs-fsrs fed the same history, including reviews early, late and minutes
    /// apart, with fuzzing on (its seed comes from the review instant, so it
    /// survives the store only if the instant does).
    #[test]
    fn every_field_matches_bare_rs_fsrs_along_an_irregular_history() {
        // (rating, minutes since the previous review)
        let history = [
            (Good, 0),
            (Again, 3),
            (Hard, 7),
            (Good, 12),
            (Easy, 60 * 24 * 5),
            (Good, 60 * 24 * 30),
            (Again, 60 * 24 * 90),
            (Again, 2),
            (Hard, 30),
            (Good, 60 * 24),
            (Easy, 60 * 24 * 400),
            (Hard, 60 * 24 * 3),
            (Good, 60 * 24 * 2 + 17),
        ];
        let parameter_sets = [
            Parameters::default(),
            with_weights(),
            long_term_with_weights(),
            Parameters {
                enable_fuzz: true,
                ..Parameters::default()
            },
        ];
        for params in parameter_sets {
            let fsrs = FSRS::new(params.clone());
            let replay = Replay::new(SrsScheduler::new(params));
            let mut bare = rs_fsrs::Card {
                due: reference_start(),
                last_review: reference_start(),
                ..rs_fsrs::Card::default()
            };
            for (step, (rating, gap)) in history.into_iter().enumerate() {
                replay.clock.advance(Duration::minutes(gap));
                bare = fsrs.next(bare, replay.clock.now(), rating.into()).card;
                let ours = replay.review(rating).next;
                assert_eq!(fsrs_card_from_state(&ours).unwrap(), bare, "step {step}");
            }
        }
    }
}

mod edge_cases {
    use super::*;

    #[test]
    fn first_review_of_a_new_card_for_each_rating() {
        let scheduler = SrsScheduler::default();
        let now = at("2026-10-31T09:15:00.000Z");
        let new = scheduler.initial_state(1, now);
        // (rating, state, due after, interval, stability, difficulty to 4 dp),
        // from the FSRS formulas with rs-fsrs's default weights.
        let cases = [
            (Again, "learning", Duration::minutes(1), 0, 0.4072, 7.2102),
            (Hard, "learning", Duration::minutes(5), 0, 1.1829, 6.5085),
            (Good, "learning", Duration::minutes(10), 0, 3.1262, 5.3146),
            (Easy, "review", Duration::days(15), 15, 15.4722, 3.2829),
        ];
        for (rating, state, due_after, interval, stability, difficulty) in cases {
            let outcome = scheduler.review(&new, rating, now, day("2026-10-31"));
            let next = &outcome.next;
            assert_eq!(next.fsrs_state, state, "{rating:?}");
            assert_eq!(next.due_date, now + due_after, "{rating:?}");
            assert_eq!(next.interval_days, interval, "{rating:?}");
            assert_eq!(next.stability, Some(stability), "{rating:?}");
            assert_eq!(round(next.difficulty.unwrap(), 4), difficulty, "{rating:?}");
            assert_eq!(
                (next.repetitions, next.lapses, next.elapsed_days),
                (1, 0, 0),
                "{rating:?}"
            );
            assert_eq!(next.last_reviewed_at, Some(now), "{rating:?}");
            assert_eq!(next.last_rating, Some(u8::from(rating)), "{rating:?}");
            assert_eq!(next.algorithm, "fsrs");
            assert_eq!(
                outcome.log,
                ReviewLogRow {
                    id: 0,
                    card_id: 1,
                    rating: u8::from(rating),
                    reviewed_at: now,
                    session_date: day("2026-10-31"),
                    state_before: "new".into(),
                    scheduled_days: interval,
                    elapsed_days: 0,
                },
                "{rating:?}"
            );
        }
    }

    #[test]
    fn again_on_a_review_card_relearns_and_counts_a_lapse() {
        let replay = Replay::with_clock(
            SrsScheduler::default(),
            FixedClock::utc(at("2026-10-31T09:15:00.000Z")),
        );
        let learned = replay.review(Easy).next;
        assert_eq!((learned.fsrs_state.as_str(), learned.lapses), ("review", 0));

        replay.clock.set(learned.due_date);
        let lapse = replay.review(Again);
        let next = &lapse.next;
        assert_eq!(next.fsrs_state, "relearning");
        assert_eq!(next.lapses, 1);
        assert_eq!(next.repetitions, 2);
        assert_eq!(next.interval_days, 0);
        assert_eq!(next.due_date, learned.due_date + Duration::minutes(5));
        assert_eq!(next.elapsed_days, 15);
        // Forgetting shrinks stability and raises difficulty.
        assert_eq!(round(next.stability.unwrap(), 4), 2.8534);
        assert_eq!(round(next.difficulty.unwrap(), 4), 5.3632);
        assert_eq!(lapse.log.state_before, "review");
        assert_eq!(lapse.log.elapsed_days, 15);

        // Relearning it graduates back to review; the lapse stays counted.
        replay.clock.set(next.due_date);
        let relearned = replay.review(Good).next;
        assert_eq!(
            (relearned.fsrs_state.as_str(), relearned.lapses),
            ("review", 1)
        );
        assert!(relearned.interval_days >= 1);
    }

    #[test]
    fn preview_lists_all_four_ratings_as_review_would_schedule_them() {
        let scheduler = SrsScheduler::default();
        let start = at("2026-10-31T09:15:00.000Z");
        let date = day("2026-10-31");
        let new = scheduler.initial_state(1, start);
        let learning = scheduler.review(&new, Good, start, date).next;
        let review = scheduler
            .review(&learning, Good, learning.due_date, date)
            .next;
        let relearning = scheduler.review(&review, Again, review.due_date, date).next;
        let states = [new, learning, review, relearning];
        assert_eq!(
            states
                .iter()
                .map(|s| s.fsrs_state.as_str())
                .collect::<Vec<_>>(),
            ["new", "learning", "review", "relearning"]
        );

        for state in &states {
            // On time, early, and a week late.
            for now in [
                state.due_date,
                state.due_date - Duration::minutes(3),
                state.due_date + Duration::days(7),
            ] {
                let previews = scheduler.preview(state, now);
                let ratings: Vec<ReviewRating> = previews.iter().map(|p| p.rating).collect();
                assert_eq!(ratings, ReviewRating::ALL);
                for preview in &previews {
                    let outcome = scheduler.review(state, preview.rating, now, date);
                    assert_eq!(
                        (preview.due_date, preview.interval_days),
                        (outcome.next.due_date, outcome.next.interval_days),
                        "{} card, {:?} at {now}",
                        state.fsrs_state,
                        preview.rating
                    );
                }
                assert!(
                    previews.windows(2).all(|w| w[0].due_date < w[1].due_date),
                    "a better rating always waits longer: {previews:?}"
                );
            }
        }
    }

    #[test]
    fn submit_review_without_a_state_fails_and_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        let scheduler = SrsScheduler::default();
        let clock = FixedClock::utc(at("2026-10-31T09:15:00.000Z"));
        let card_id = insert_card(&store, "学习");

        for id in [card_id, card_id + 100] {
            assert!(
                matches!(
                    submit_review(&store, &scheduler, id, Good, &clock),
                    Err(StoreError::NotFound(_))
                ),
                "card {id}"
            );
            assert_eq!(store.get_srs_state(id).unwrap(), None);
            assert!(store.review_log_for_card(id).unwrap().is_empty());
        }
        assert_eq!(store.count_srs_states().unwrap(), 0);
    }

    #[test]
    fn submit_review_refuses_a_state_fsrs_cannot_interpret_and_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        let scheduler = SrsScheduler::default();
        let clock = FixedClock::utc(at("2026-11-09T08:00:00.000Z"));
        let card_id = insert_card(&store, "学习");
        let valid = review_state(card_id);
        let new = scheduler.initial_state(card_id, at("2026-10-31T08:00:00.000Z"));
        let cases = [
            (
                "another algorithm",
                SrsStateRow {
                    algorithm: "sm2".into(),
                    ..valid.clone()
                },
            ),
            (
                "an unknown state",
                SrsStateRow {
                    fsrs_state: "suspended".into(),
                    ..valid.clone()
                },
            ),
            (
                "no stability",
                SrsStateRow {
                    stability: None,
                    ..valid.clone()
                },
            ),
            // NaN is stored as NULL, so it reads back as no stability.
            (
                "a NaN stability",
                SrsStateRow {
                    stability: Some(f64::NAN),
                    ..valid.clone()
                },
            ),
            (
                "a zero difficulty",
                SrsStateRow {
                    difficulty: Some(0.0),
                    ..valid.clone()
                },
            ),
            (
                "no last review",
                SrsStateRow {
                    last_reviewed_at: None,
                    ..valid.clone()
                },
            ),
            (
                "a new card with a stability",
                SrsStateRow {
                    stability: Some(3.0),
                    ..new.clone()
                },
            ),
            (
                "negative lapses",
                SrsStateRow {
                    lapses: -1,
                    ..valid.clone()
                },
            ),
            // rs-fsrs would overflow adding this review, or the lapse that
            // computing the Again outcome adds.
            (
                "repetitions at i32::MAX",
                SrsStateRow {
                    repetitions: i64::from(i32::MAX),
                    ..valid.clone()
                },
            ),
            (
                "lapses at i32::MAX",
                SrsStateRow {
                    lapses: i64::from(i32::MAX),
                    ..valid.clone()
                },
            ),
        ];
        for (what, row) in cases {
            store.upsert_srs_state(&row).unwrap();
            let stored = store.get_srs_state(card_id).unwrap();
            match submit_review(&store, &scheduler, card_id, Good, &clock) {
                Err(StoreError::Invalid(message)) => assert!(
                    message.starts_with(&format!("srs_state row {card_id}: ")),
                    "{what}: {message}"
                ),
                other => panic!("{what}: expected StoreError::Invalid, got {other:?}"),
            }
            assert_eq!(store.get_srs_state(card_id).unwrap(), stored, "{what}");
            assert!(
                store.review_log_for_card(card_id).unwrap().is_empty(),
                "{what}"
            );
        }

        // The same card with a valid state reviews fine.
        store.upsert_srs_state(&valid).unwrap();
        assert!(submit_review(&store, &scheduler, card_id, Good, &clock).is_ok());
    }

    #[test]
    fn a_review_whose_log_cannot_be_stored_leaves_the_state_unchanged() {
        // 23:30 UTC on the last day of 9999 is already year 10000 at UTC+1, a
        // session date the store refuses. The new state itself is storable,
        // so this fails only after it was written.
        let clock = FixedClock::new(
            at("9999-12-31T23:30:00.000Z"),
            FixedOffset::east_opt(3600).unwrap(),
        );
        let replay = Replay::with_clock(SrsScheduler::default(), clock);
        let initial = replay.stored();

        let result = submit_review(
            &replay.store,
            &replay.scheduler,
            replay.card_id,
            Again,
            &replay.clock,
        );
        assert!(matches!(result, Err(StoreError::Invalid(_))), "{result:?}");
        assert_eq!(replay.stored(), initial);
        assert!(replay
            .store
            .review_log_for_card(replay.card_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn each_review_appends_one_log_row_with_what_happened() {
        // 23:30 UTC is already the next day in Berlin (UTC+1).
        let clock = FixedClock::new(
            at("2026-10-31T23:30:00.000Z"),
            FixedOffset::east_opt(3600).unwrap(),
        );
        let replay = Replay::with_clock(SrsScheduler::default(), clock);

        let first = replay.review(Good);
        replay.clock.set(first.next.due_date);
        let second = replay.review(Good);
        let graduated = second.next.interval_days;
        assert!(graduated > 0);
        // Three days late.
        replay.clock.set(second.next.due_date + Duration::days(3));
        let third = replay.review(Hard);

        let expected = [
            ReviewLogRow {
                id: first.log.id,
                card_id: replay.card_id,
                rating: 3,
                reviewed_at: at("2026-10-31T23:30:00.000Z"),
                session_date: day("2026-11-01"),
                state_before: "new".into(),
                scheduled_days: 0,
                elapsed_days: 0,
            },
            ReviewLogRow {
                id: second.log.id,
                card_id: replay.card_id,
                rating: 3,
                reviewed_at: at("2026-10-31T23:40:00.000Z"),
                session_date: day("2026-11-01"),
                state_before: "learning".into(),
                // The interval this review set, not the previous one.
                scheduled_days: graduated,
                elapsed_days: 0,
            },
            ReviewLogRow {
                id: third.log.id,
                card_id: replay.card_id,
                rating: 2,
                reviewed_at: second.next.due_date + Duration::days(3),
                session_date: (second.next.due_date + Duration::days(3))
                    .with_timezone(&FixedOffset::east_opt(3600).unwrap())
                    .date_naive(),
                state_before: "review".into(),
                scheduled_days: third.next.interval_days,
                elapsed_days: graduated + 3,
            },
        ];
        assert_eq!(
            replay.store.review_log_for_card(replay.card_id).unwrap(),
            expected
        );
        assert_eq!([first.log, second.log, third.log], expected);
        let ids: HashSet<i64> = expected.iter().map(|log| log.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&0), "stored logs carry their row id");
        assert_ne!(third.next.interval_days, graduated);
    }

    #[test]
    fn sub_millisecond_instants_are_cut_so_stored_and_in_memory_states_agree() {
        let start = at("2026-10-31T08:00:00.000Z") + Duration::nanoseconds(123_456_789);
        let replay = Replay::with_clock(SrsScheduler::default(), FixedClock::utc(start));
        let mut in_memory = replay.scheduler.initial_state(replay.card_id, start);
        assert_eq!(in_memory, replay.stored());
        assert_eq!(in_memory.due_date, at("2026-10-31T08:00:00.123Z"));

        for (rating, gap) in [
            (Good, Duration::minutes(10) + Duration::nanoseconds(987_654)),
            (Good, Duration::nanoseconds(400_001)),
            (Again, Duration::days(3) + Duration::nanoseconds(1)),
            (Good, Duration::minutes(5) + Duration::nanoseconds(999_999)),
            (Easy, Duration::days(1) + Duration::nanoseconds(250_500)),
        ] {
            replay.clock.advance(gap);
            let now = replay.clock.now();
            let previews = replay.scheduler.preview(&in_memory, now);
            let memory_outcome =
                replay
                    .scheduler
                    .review(&in_memory, rating, now, replay.clock.today());
            let stored_outcome = replay.review(rating);
            assert_eq!(
                memory_outcome.next, stored_outcome.next,
                "{rating:?} at {now}"
            );

            in_memory = memory_outcome.next;
            assert_eq!(in_memory.due_date.nanosecond() % 1_000_000, 0);
            let reviewed_at = in_memory.last_reviewed_at.unwrap();
            assert_eq!(reviewed_at.nanosecond() % 1_000_000, 0);
            assert!(now - reviewed_at < Duration::milliseconds(1));
            assert_eq!(memory_outcome.log.reviewed_at, reviewed_at);
            assert!(previews
                .iter()
                .all(|p| p.due_date.nanosecond() % 1_000_000 == 0));
        }
    }

    #[test]
    fn a_clock_set_back_reviews_at_the_previous_review_instead() {
        let scheduler = SrsScheduler::default();
        let reviewed = at("2026-10-31T08:00:00.000Z");
        let date = day("2026-10-31");
        let state = scheduler
            .review(&scheduler.initial_state(1, reviewed), Easy, reviewed, date)
            .next;
        assert_eq!(state.fsrs_state, "review");

        for set_back in [Duration::hours(5), Duration::days(3), Duration::days(400)] {
            let outcome = scheduler.review(&state, Good, reviewed - set_back, date);
            assert_eq!(
                outcome,
                scheduler.review(&state, Good, reviewed, date),
                "set back by {set_back}"
            );
            assert_eq!(outcome.log.reviewed_at, reviewed);
            assert_eq!(outcome.log.elapsed_days, 0);
            let stability = outcome.next.stability.unwrap();
            assert!(stability.is_finite() && stability >= state.stability.unwrap());
            assert_eq!(
                scheduler.preview(&state, reviewed - set_back),
                scheduler.preview(&state, reviewed)
            );
        }
    }
}

mod conversion {
    use super::*;

    #[test]
    fn stored_states_convert_to_rs_fsrs_cards_and_back_unchanged() {
        let row = SrsStateRow {
            fsrs_state: "relearning".into(),
            last_rating: Some(1),
            ..review_state(7)
        };
        let card = fsrs_card_from_state(&row).unwrap();
        assert_eq!(
            card,
            rs_fsrs::Card {
                due: at("2026-11-09T08:00:00.000Z"),
                stability: 12.5,
                difficulty: 6.25,
                elapsed_days: 3,
                scheduled_days: 9,
                reps: 4,
                lapses: 1,
                state: State::Relearning,
                last_review: at("2026-10-31T08:00:00.000Z"),
            }
        );
        assert_eq!(state_from_fsrs_card(7, &card, Some(1)), row);

        let now = at("2026-10-31T09:15:00.000Z");
        let new = SrsScheduler::default().initial_state(3, now);
        assert_eq!(
            new,
            SrsStateRow {
                card_id: 3,
                algorithm: "fsrs".into(),
                stability: None,
                difficulty: None,
                interval_days: 0,
                repetitions: 0,
                lapses: 0,
                fsrs_state: "new".into(),
                elapsed_days: 0,
                due_date: now,
                last_reviewed_at: None,
                last_rating: None,
            }
        );
        let new_card = fsrs_card_from_state(&new).unwrap();
        assert_eq!(
            new_card,
            rs_fsrs::Card {
                due: now,
                last_review: now,
                ..rs_fsrs::Card::default()
            }
        );
        assert_eq!(state_from_fsrs_card(3, &new_card, None), new);
    }

    #[test]
    fn every_state_a_review_history_produces_survives_the_round_trip() {
        let replay = Replay::new(SrsScheduler::default());
        let mut rows = vec![replay.stored()];
        for rating in [
            Good, Good, Again, Good, Easy, Hard, Again, Again, Good, Easy,
        ] {
            rows.push(replay.review(rating).next);
            replay.clock.advance(Duration::days(2));
        }
        let states: HashSet<&str> = rows.iter().map(|row| row.fsrs_state.as_str()).collect();
        assert_eq!(
            states,
            HashSet::from(["new", "learning", "review", "relearning"])
        );
        for row in &rows {
            let card = fsrs_card_from_state(row).unwrap();
            assert_eq!(
                &state_from_fsrs_card(row.card_id, &card, row.last_rating),
                row
            );
        }
    }

    #[test]
    fn rows_fsrs_cannot_interpret_are_refused_and_never_scheduled_as_they_are() {
        let scheduler = SrsScheduler::default();
        let now = at("2026-11-09T08:00:00.000Z");
        let valid = review_state(7);
        let new = scheduler.initial_state(7, at("2026-10-31T08:00:00.000Z"));
        let cases = [
            (
                "another algorithm",
                SrsStateRow {
                    algorithm: "fsrs-6".into(),
                    ..valid.clone()
                },
            ),
            (
                "an unknown state",
                SrsStateRow {
                    fsrs_state: "suspended".into(),
                    ..valid.clone()
                },
            ),
            (
                "a capitalised state",
                SrsStateRow {
                    fsrs_state: "Review".into(),
                    ..valid.clone()
                },
            ),
            (
                "no stability",
                SrsStateRow {
                    stability: None,
                    ..valid.clone()
                },
            ),
            (
                "no difficulty",
                SrsStateRow {
                    difficulty: None,
                    ..valid.clone()
                },
            ),
            (
                "no last review",
                SrsStateRow {
                    last_reviewed_at: None,
                    ..valid.clone()
                },
            ),
            (
                "a zero stability",
                SrsStateRow {
                    stability: Some(0.0),
                    ..valid.clone()
                },
            ),
            (
                "a negative difficulty",
                SrsStateRow {
                    difficulty: Some(-1.0),
                    ..valid.clone()
                },
            ),
            (
                "an infinite stability",
                SrsStateRow {
                    stability: Some(f64::INFINITY),
                    ..valid.clone()
                },
            ),
            (
                "a NaN difficulty",
                SrsStateRow {
                    difficulty: Some(f64::NAN),
                    ..valid.clone()
                },
            ),
            (
                "a new card with a stability",
                SrsStateRow {
                    stability: Some(3.0),
                    ..new.clone()
                },
            ),
            (
                "a new card with a difficulty",
                SrsStateRow {
                    difficulty: Some(5.0),
                    ..new.clone()
                },
            ),
            (
                "a new card with a last review",
                SrsStateRow {
                    last_reviewed_at: Some(now),
                    ..new.clone()
                },
            ),
            (
                "negative lapses",
                SrsStateRow {
                    lapses: -1,
                    ..valid.clone()
                },
            ),
            (
                "negative repetitions",
                SrsStateRow {
                    repetitions: -2,
                    ..valid.clone()
                },
            ),
            (
                "more repetitions than rs-fsrs can count",
                SrsStateRow {
                    repetitions: i64::from(i32::MAX) + 1,
                    ..valid.clone()
                },
            ),
            (
                "repetitions that one more review would overflow",
                SrsStateRow {
                    repetitions: i64::from(i32::MAX),
                    ..valid.clone()
                },
            ),
            (
                "lapses that one more lapse would overflow",
                SrsStateRow {
                    lapses: i64::from(i32::MAX),
                    ..valid.clone()
                },
            ),
        ];
        for (what, row) in cases {
            match fsrs_card_from_state(&row) {
                Err(StoreError::Invalid(message)) => {
                    assert!(
                        message.starts_with("srs_state row 7: "),
                        "{what}: {message}"
                    )
                }
                other => panic!("{what}: expected StoreError::Invalid, got {other:?}"),
            }
            assert!(scheduler.preview(&row, now).is_empty(), "{what}");

            // `review` cannot fail: it starts such a card over as unseen.
            let outcome = scheduler.review(&row, Good, now, day("2026-11-09"));
            assert_eq!(outcome.log.state_before, "new", "{what}");
            assert_eq!(
                outcome.next,
                scheduler
                    .review(
                        &scheduler.initial_state(7, now),
                        Good,
                        now,
                        day("2026-11-09")
                    )
                    .next,
                "{what}"
            );
        }
        assert!(fsrs_card_from_state(&valid).is_ok());
    }

    #[test]
    fn the_largest_counts_rs_fsrs_can_still_add_to_are_scheduled() {
        let scheduler = SrsScheduler::default();
        let now = at("2026-11-09T08:00:00.000Z");
        let almost_full = i64::from(i32::MAX) - 1;
        let row = SrsStateRow {
            repetitions: almost_full,
            lapses: almost_full,
            ..review_state(7)
        };
        assert!(fsrs_card_from_state(&row).is_ok());
        assert_eq!(scheduler.preview(&row, now).len(), 4);

        let lapse = scheduler.review(&row, Again, now, day("2026-11-09"));
        assert_eq!(lapse.log.state_before, "review");
        assert_eq!(
            (lapse.next.repetitions, lapse.next.lapses),
            (almost_full + 1, almost_full + 1)
        );
    }
}

mod serialized_shapes {
    use super::*;

    #[test]
    fn ratings_serialize_as_their_numbers() {
        for (rating, number) in ReviewRating::ALL.into_iter().zip(1u8..) {
            assert_eq!(
                serde_json::to_value(rating).unwrap(),
                serde_json::json!(number)
            );
            assert_eq!(
                serde_json::from_value::<ReviewRating>(serde_json::json!(number)).unwrap(),
                rating
            );
            assert_eq!(ReviewRating::try_from(number), Ok(rating));
        }
        for bad in [
            serde_json::json!(0),
            serde_json::json!(5),
            serde_json::json!(-1),
            serde_json::json!(2.5),
            serde_json::json!("3"),
            serde_json::json!("good"),
        ] {
            assert!(
                serde_json::from_value::<ReviewRating>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        assert_eq!(ReviewRating::try_from(7), Err(InvalidRating(7)));
        assert_eq!(
            InvalidRating(7).to_string(),
            "a review rating is 1 (Again) to 4 (Easy), not 7"
        );
    }

    #[test]
    fn previews_and_outcomes_serialize_for_ipc() {
        let scheduler = SrsScheduler::default();
        let now = at("2026-10-31T09:15:00.000Z");
        let new = scheduler.initial_state(5, now);
        let previews = scheduler.preview(&new, now);
        assert_eq!(
            serde_json::to_value(&previews[2]).unwrap(),
            serde_json::json!({
                "rating": 3,
                "due_date": "2026-10-31T09:25:00Z",
                "interval_days": 0
            })
        );

        let outcome = scheduler.review(&new, Easy, now, day("2026-10-31"));
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["next"]["card_id"], 5);
        assert_eq!(json["next"]["fsrs_state"], "review");
        assert_eq!(json["next"]["last_rating"], 4);
        assert_eq!(json["log"]["rating"], 4);
        assert_eq!(json["log"]["session_date"], "2026-10-31");
        let back: ReviewOutcome = serde_json::from_value(json).unwrap();
        assert_eq!(back, outcome);
    }
}
