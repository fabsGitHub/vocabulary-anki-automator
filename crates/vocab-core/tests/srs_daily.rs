//! G+ tests for the once-per-day onboarding prompt and the day's session
//! (`vocab_core::srs`, docs/architecture.md § 7.1, Milestone L2).
//!
//! Every scenario runs at UTC+1 and at UTC-5, where the local date and the
//! UTC date disagree for part of every day, and moves a `FixedClock` across
//! local midnight. "Today" must always be the local date.

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use vocab_core::{
    srs::{
        daily_prompt_state, dismiss_daily_prompt, record_session_progress, reopen_daily_prompt,
        should_prompt, submit_daily_quota, submit_review, DailyPromptState, ReviewRating,
        SessionProgress, SrsScheduler, LAST_PROMPTED_KEY,
    },
    store::{DailySessionRow, SrsStateRow, Store},
    time::{Clock, FixedClock},
    CaptureOrigin, Card,
};

/// The offsets every scenario runs in, in hours east of UTC.
const OFFSETS: [i32; 2] = [1, -5];

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("valid test date")
}

/// The instant of a wall-clock time (`YYYY-MM-DD HH:MM:SS.fff`) at `offset`.
fn local_instant(offset: FixedOffset, text: &str) -> DateTime<Utc> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f")
        .expect("valid local test time");
    offset
        .from_local_datetime(&naive)
        .single()
        .expect("fixed offsets are unambiguous")
        .with_timezone(&Utc)
}

/// A learner's database and clock in one UTC offset.
struct Learner {
    store: Store,
    clock: FixedClock,
    offset: FixedOffset,
}

impl Learner {
    /// Starts at 07:00 local time on 2026-10-31.
    fn new(hours: i32) -> Self {
        let offset = FixedOffset::east_opt(hours * 3600).expect("valid offset");
        Self {
            store: Store::open_in_memory().expect("in-memory store"),
            clock: FixedClock::new(local_instant(offset, "2026-10-31 07:00:00.000"), offset),
            offset,
        }
    }

    fn local(&self, text: &str) -> DateTime<Utc> {
        local_instant(self.offset, text)
    }

    fn set_local(&self, text: &str) {
        self.clock.set(self.local(text));
    }

    fn label(&self) -> String {
        format!("UTC{:+}", self.offset.local_minus_utc() / 3600)
    }

    /// A card reviewed before, due again at a local time: one of the day's
    /// reviews.
    fn add_card_due(&self, local_due: &str) -> i64 {
        let id = self.add_capture(local_due);
        let due = self.local(local_due);
        let state = SrsStateRow {
            stability: Some(3.0),
            difficulty: Some(5.0),
            interval_days: 3,
            repetitions: 2,
            fsrs_state: "review".into(),
            elapsed_days: 3,
            last_reviewed_at: Some(due - Duration::days(3)),
            last_rating: Some(3),
            ..SrsScheduler::default().initial_state(id, due)
        };
        self.store.upsert_srs_state(&state).unwrap();
        id
    }

    /// A word introduced at a local time and not rated since: due, but its
    /// first rating will count as a new item, not as a review.
    fn add_card_waiting(&self, local_introduced: &str) -> i64 {
        let id = self.add_capture(local_introduced);
        let state = SrsScheduler::default().initial_state(id, self.local(local_introduced));
        self.store.upsert_srs_state(&state).unwrap();
        id
    }

    fn add_capture(&self, local_due: &str) -> i64 {
        let card = Card::new(
            format!("word due {local_due}"),
            "Bedeutung",
            "en",
            "de",
            self.local("2026-10-01 00:00:00.000"),
        );
        self.store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .unwrap()
    }

    /// Two cards due on Oct 31 (one this evening) and one on Nov 1.
    fn with_reviews(self) -> Self {
        self.add_card_due("2026-10-31 06:00:00.000");
        self.add_card_due("2026-10-31 22:00:00.000");
        self.add_card_due("2026-11-01 06:00:00.000");
        self
    }

    fn prompt(&self) -> DailyPromptState {
        daily_prompt_state(&self.store, &self.clock).unwrap()
    }

    fn submit(&self, new_count: u32) -> DailySessionRow {
        let session = submit_daily_quota(&self.store, new_count, &self.clock).unwrap();
        self.assert_stored(&session);
        session
    }

    fn dismiss(&self) -> DailySessionRow {
        let session = dismiss_daily_prompt(&self.store, &self.clock).unwrap();
        self.assert_stored(&session);
        session
    }

    fn reopen(&self) {
        reopen_daily_prompt(&self.store, &self.clock).unwrap();
    }

    fn progress(&self, kind: SessionProgress) -> DailySessionRow {
        let session = record_session_progress(&self.store, kind, &self.clock).unwrap();
        self.assert_stored(&session);
        session
    }

    fn assert_stored(&self, session: &DailySessionRow) {
        assert_eq!(
            self.store
                .get_daily_session(session.session_date)
                .unwrap()
                .as_ref(),
            Some(session),
            "{}: the returned session is the stored one",
            self.label()
        );
    }
}

fn session(
    date: &str,
    requested_new_count: u32,
    reviews_due_count: u32,
    reviews_completed: u32,
    new_items_completed: u32,
) -> DailySessionRow {
    DailySessionRow {
        session_date: day(date),
        requested_new_count,
        reviews_due_count,
        reviews_completed,
        new_items_completed,
        completed_at: None,
    }
}

#[test]
fn should_prompt_unless_already_prompted_on_that_very_day() {
    let today = day("2026-10-31");
    assert!(should_prompt(None, today));
    assert!(!should_prompt(Some(today), today));
    assert!(should_prompt(Some(day("2026-10-30")), today));
    // A date ahead of today (the clock was set back) does not count as today.
    assert!(should_prompt(Some(day("2026-11-01")), today));
}

#[test]
fn the_prompt_appears_once_per_local_day() {
    for hours in OFFSETS {
        let learner = Learner::new(hours).with_reviews();
        let label = learner.label();

        assert_eq!(
            learner.prompt(),
            DailyPromptState {
                needs_prompt: true,
                today: day("2026-10-31"),
                session: None,
            },
            "{label}: first launch"
        );
        let answered = learner.submit(5);
        assert_eq!(answered, session("2026-10-31", 5, 2, 0, 0), "{label}");
        assert_eq!(
            learner
                .store
                .get_state(LAST_PROMPTED_KEY)
                .unwrap()
                .as_deref(),
            Some("2026-10-31"),
            "{label}"
        );

        let mut utc_date_differed = false;
        for later in [
            "2026-10-31 07:00:00.001",
            "2026-10-31 18:00:00.000",
            "2026-10-31 23:30:00.000",
            "2026-10-31 23:59:59.999",
        ] {
            learner.set_local(later);
            utc_date_differed |= learner.clock.now().date_naive() != day("2026-10-31");
            assert_eq!(
                learner.prompt(),
                DailyPromptState {
                    needs_prompt: false,
                    today: day("2026-10-31"),
                    session: Some(answered.clone()),
                },
                "{label}: no second prompt at {later}"
            );
        }

        for next_day in ["2026-11-01 00:00:00.000", "2026-11-01 00:30:00.000"] {
            learner.set_local(next_day);
            utc_date_differed |= learner.clock.now().date_naive() != day("2026-11-01");
            assert_eq!(
                learner.prompt(),
                DailyPromptState {
                    needs_prompt: true,
                    today: day("2026-11-01"),
                    session: None,
                },
                "{label}: a new local day prompts again at {next_day}"
            );
        }
        assert!(
            utc_date_differed,
            "{label}: some checkpoint must fall on another UTC date"
        );

        // The unreviewed cards of Oct 31 are still due on Nov 1.
        assert_eq!(
            learner.submit(3),
            session("2026-11-01", 3, 3, 0, 0),
            "{label}"
        );
        assert!(!learner.prompt().needs_prompt, "{label}");
        assert_eq!(
            learner.store.get_daily_session(day("2026-10-31")).unwrap(),
            Some(answered),
            "{label}: yesterday's session is untouched"
        );
    }
}

#[test]
fn dismissing_means_reviews_only_and_no_prompt_again_that_day() {
    for hours in OFFSETS {
        let learner = Learner::new(hours).with_reviews();
        let label = learner.label();

        let dismissed = learner.dismiss();
        assert_eq!(dismissed, session("2026-10-31", 0, 2, 0, 0), "{label}");
        for later in ["2026-10-31 07:00:00.000", "2026-10-31 23:59:59.999"] {
            learner.set_local(later);
            let state = learner.prompt();
            assert!(!state.needs_prompt, "{label}: at {later}");
            assert_eq!(state.session.as_ref(), Some(&dismissed), "{label}");
        }

        learner.set_local("2026-11-01 00:00:00.000");
        assert!(
            learner.prompt().needs_prompt,
            "{label}: the next day asks again"
        );
    }
}

#[test]
fn reopening_prompts_again_and_a_later_answer_keeps_the_progress() {
    for hours in OFFSETS {
        let learner = Learner::new(hours).with_reviews();
        let label = learner.label();
        learner.dismiss();
        learner.set_local("2026-10-31 09:00:00.000");
        assert_eq!(
            learner.progress(SessionProgress::Review),
            session("2026-10-31", 0, 2, 1, 0),
            "{label}"
        );

        learner.set_local("2026-10-31 20:00:00.000");
        learner.reopen();
        let state = learner.prompt();
        assert!(
            state.needs_prompt,
            "{label}: the menu action shows the prompt"
        );
        assert_eq!(
            state.session,
            Some(session("2026-10-31", 0, 2, 1, 0)),
            "{label}: reopening keeps today's session"
        );

        assert_eq!(
            learner.submit(4),
            session("2026-10-31", 4, 2, 1, 0),
            "{label}: the new quota, the same progress and review target"
        );
        assert!(!learner.prompt().needs_prompt, "{label}");
        assert_eq!(
            learner.progress(SessionProgress::NewItem),
            session("2026-10-31", 4, 2, 1, 1),
            "{label}"
        );

        // Reopening and then closing the prompt keeps the answer.
        learner.reopen();
        assert_eq!(
            learner.dismiss(),
            session("2026-10-31", 4, 2, 1, 1),
            "{label}"
        );
        assert!(!learner.prompt().needs_prompt, "{label}");
    }
}

#[test]
fn reopening_without_an_earlier_prompt_is_harmless() {
    let learner = Learner::new(1);
    learner.reopen();
    assert!(learner.prompt().needs_prompt);
    assert_eq!(learner.prompt().session, None);
}

#[test]
fn progress_completes_the_session_exactly_when_both_targets_are_met() {
    for hours in OFFSETS {
        for reviews_first in [true, false] {
            let learner = Learner::new(hours).with_reviews();
            let label = format!("{}, reviews first: {reviews_first}", learner.label());
            assert_eq!(
                learner.submit(2),
                session("2026-10-31", 2, 2, 0, 0),
                "{label}"
            );

            let order = if reviews_first {
                [
                    SessionProgress::Review,
                    SessionProgress::Review,
                    SessionProgress::NewItem,
                    SessionProgress::NewItem,
                ]
            } else {
                [
                    SessionProgress::NewItem,
                    SessionProgress::NewItem,
                    SessionProgress::Review,
                    SessionProgress::Review,
                ]
            };
            let mut last = None;
            for (step, kind) in order.into_iter().enumerate() {
                learner.clock.advance(Duration::minutes(3));
                let progressed = learner.progress(kind);
                if step < 3 {
                    assert_eq!(progressed.completed_at, None, "{label}: step {step}");
                } else {
                    assert_eq!(
                        progressed.completed_at,
                        Some(learner.clock.now()),
                        "{label}: the update that meets both targets"
                    );
                }
                last = Some(progressed);
            }
            let completed = last.unwrap();
            assert_eq!(
                (completed.reviews_completed, completed.new_items_completed),
                (2, 2),
                "{label}"
            );

            // Extra work is counted; the session stays completed when it was.
            learner.clock.advance(Duration::minutes(30));
            let extra = learner.progress(SessionProgress::Review);
            assert_eq!(extra.reviews_completed, 3, "{label}");
            assert_eq!(extra.completed_at, completed.completed_at, "{label}");
            let extra = learner.progress(SessionProgress::NewItem);
            assert_eq!(extra.new_items_completed, 3, "{label}");
            assert_eq!(extra.completed_at, completed.completed_at, "{label}");
            assert_eq!(
                learner.store.list_completed_session_dates().unwrap(),
                [day("2026-10-31")],
                "{label}"
            );
        }
    }
}

#[test]
fn asking_for_more_after_finishing_does_not_undo_the_day() {
    for hours in OFFSETS {
        let learner = Learner::new(hours);
        let label = learner.label();
        learner.submit(1);
        let done = learner.progress(SessionProgress::NewItem);
        assert!(done.completed_at.is_some(), "{label}");

        learner.clock.advance(Duration::hours(2));
        learner.reopen();
        let more = learner.submit(5);
        assert_eq!(more.requested_new_count, 5, "{label}");
        assert_eq!(more.completed_at, done.completed_at, "{label}");
    }
}

#[test]
fn lowering_the_quota_to_what_is_done_completes_the_session() {
    for hours in OFFSETS {
        let learner = Learner::new(hours);
        let label = learner.label();
        learner.submit(3);
        assert_eq!(
            learner.progress(SessionProgress::NewItem).completed_at,
            None,
            "{label}"
        );

        learner.clock.advance(Duration::minutes(10));
        learner.reopen();
        let lowered = learner.submit(1);
        assert_eq!(lowered.completed_at, Some(learner.clock.now()), "{label}");
    }
}

/// Completion feeds the streak and the session XP bonus, which are tied to
/// review work: a day with nothing due and no new words never completes, no
/// matter how its session was set up.
#[test]
fn a_day_with_nothing_to_do_never_completes() {
    for hours in OFFSETS {
        let learner = Learner::new(hours);
        let label = learner.label();
        let dismissed = learner.dismiss();
        assert_eq!(dismissed.reviews_due_count, 0, "{label}");
        assert_eq!(dismissed.completed_at, None, "{label}");

        learner.set_local("2026-11-01 08:00:00.000");
        assert_eq!(learner.submit(0).completed_at, None, "{label}");
        learner.set_local("2026-11-02 08:00:00.000");
        assert_eq!(learner.submit(1).completed_at, None, "{label}");
    }
}

#[test]
fn the_review_target_is_counted_when_the_day_is_set_up() {
    for hours in OFFSETS {
        let learner = Learner::new(hours);
        let label = learner.label();
        let overdue = learner.add_card_due("2026-10-29 10:00:00.000");
        learner.add_card_due("2026-10-31 06:59:59.999");
        learner.add_card_due("2026-10-31 23:59:59.999");
        learner.add_card_due("2026-11-01 00:00:00.000");
        learner.add_card_due("2026-11-05 12:00:00.000");

        assert_eq!(
            learner.submit(2).reviews_due_count,
            3,
            "{label}: overdue, earlier today and late tonight; not tomorrow"
        );

        // Reviewing a card and answering again does not recount the target.
        submit_review(
            &learner.store,
            &SrsScheduler::default(),
            overdue,
            ReviewRating::Easy,
            &learner.clock,
        )
        .unwrap();
        learner.progress(SessionProgress::Review);
        learner.reopen();
        assert_eq!(
            learner.submit(6),
            session("2026-10-31", 6, 3, 1, 0),
            "{label}"
        );
    }
}

/// A word introduced on an earlier day and not rated since (the app was
/// closed on its card) is due today, and its first rating counts as a new
/// item (§ 7.1), never as a review. So it is no review target: rating it
/// could never meet one, and once it is rated nothing else is due, so the
/// day would stay open for good.
#[test]
fn a_word_waiting_for_its_first_rating_is_not_a_review_target() {
    let scheduler = SrsScheduler::default();
    let first_rating = |learner: &Learner, card_id: i64| {
        let outcome = submit_review(
            &learner.store,
            &scheduler,
            card_id,
            ReviewRating::Easy,
            &learner.clock,
        )
        .unwrap();
        assert_eq!(outcome.log.state_before, "new", "{}", learner.label());
        learner.progress(SessionProgress::NewItem)
    };
    for hours in OFFSETS {
        // Reviews only, and the waiting word is all there is to do.
        let learner = Learner::new(hours);
        let label = learner.label();
        let waiting = learner.add_card_waiting("2026-10-30 21:00:00.000");
        assert_eq!(
            learner.dismiss(),
            session("2026-10-31", 0, 0, 0, 0),
            "{label}"
        );
        let done = first_rating(&learner, waiting);
        assert_eq!(done.completed_at, Some(learner.clock.now()), "{label}");

        // With a review due too, the day completes once both are done.
        let learner = Learner::new(hours);
        let waiting = learner.add_card_waiting("2026-10-30 21:00:00.000");
        let reviewed = learner.add_card_due("2026-10-31 06:00:00.000");
        assert_eq!(
            learner.submit(1),
            session("2026-10-31", 1, 1, 0, 0),
            "{label}"
        );
        assert_eq!(
            first_rating(&learner, waiting),
            session("2026-10-31", 1, 1, 0, 1),
            "{label}"
        );
        submit_review(
            &learner.store,
            &scheduler,
            reviewed,
            ReviewRating::Good,
            &learner.clock,
        )
        .unwrap();
        let done = learner.progress(SessionProgress::Review);
        assert_eq!(done.completed_at, Some(learner.clock.now()), "{label}");
    }
}

#[test]
fn progress_before_the_days_prompt_opens_a_reviews_only_session_and_still_prompts() {
    for hours in OFFSETS {
        let learner = Learner::new(hours).with_reviews();
        let label = learner.label();
        learner.submit(2);

        // A review session that started last evening runs past midnight.
        learner.set_local("2026-11-01 00:05:00.000");
        let opened = learner.progress(SessionProgress::Review);
        assert_eq!(opened, session("2026-11-01", 0, 3, 1, 0), "{label}");
        let state = learner.prompt();
        assert!(
            state.needs_prompt,
            "{label}: the new day was not answered yet"
        );
        assert_eq!(state.session, Some(opened), "{label}");

        assert_eq!(
            learner.submit(4),
            session("2026-11-01", 4, 3, 1, 0),
            "{label}"
        );
    }
}

#[test]
fn an_unreadable_prompt_date_counts_as_never_prompted() {
    let learner = Learner::new(-5);
    learner
        .store
        .set_state(LAST_PROMPTED_KEY, "31.10.2026")
        .unwrap();
    assert!(learner.prompt().needs_prompt);
    learner.submit(1);
    assert_eq!(
        learner
            .store
            .get_state(LAST_PROMPTED_KEY)
            .unwrap()
            .as_deref(),
        Some("2026-10-31")
    );
    assert!(!learner.prompt().needs_prompt);
}

#[test]
fn prompt_state_and_progress_kinds_serialize_for_ipc() {
    let learner = Learner::new(1);
    assert_eq!(
        serde_json::to_value(learner.prompt()).unwrap(),
        serde_json::json!({ "needs_prompt": true, "today": "2026-10-31", "session": null })
    );
    let answered = learner.submit(3);
    let json = serde_json::to_value(learner.prompt()).unwrap();
    assert_eq!(json["needs_prompt"], false);
    assert_eq!(json["session"]["session_date"], "2026-10-31");
    assert_eq!(json["session"]["requested_new_count"], 3);
    let back: DailyPromptState = serde_json::from_value(json).unwrap();
    assert_eq!(back.session, Some(answered));

    assert_eq!(
        serde_json::to_value(SessionProgress::Review).unwrap(),
        "review"
    );
    assert_eq!(
        serde_json::to_value(SessionProgress::NewItem).unwrap(),
        "new_item"
    );
}
