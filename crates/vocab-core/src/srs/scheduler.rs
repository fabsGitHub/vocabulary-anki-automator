//! FSRS scheduling through rs-fsrs (Milestone L1, decision 8).
//!
//! The memory model is entirely rs-fsrs's: stability, difficulty, learning
//! steps and intervals come out of `rs_fsrs::FSRS` unchanged. This module only
//! converts between the store's [`SrsStateRow`] and rs-fsrs's `Card`, one to
//! one, and supplies what rs-fsrs leaves to its caller. A mistake here would
//! not crash anything; it would schedule every card slightly wrong for as long
//! as nobody compared the intervals with a reference. So the G+ tests replay
//! rs-fsrs's own reference cases through this module, storing and reloading
//! the state between reviews, and the module follows these rules:
//!
//! - **Whole milliseconds.** Every `now` is truncated to the millisecond
//!   before rs-fsrs sees it, because the store keeps no more. A due date
//!   computed from a finer instant would change when written, and the next
//!   review of the stored card would differ from one of the card in memory.
//! - **No hidden clock.** `rs_fsrs::Card::new()` stamps the card with
//!   `Utc::now()`, so cards are built from rows here instead.
//!   (`Parameters::default()` reads the clock too, but only for a fuzz seed
//!   that rs-fsrs replaces before every review with one derived from the
//!   review instant.)
//! - **Reviews do not travel back in time.** rs-fsrs trusts the days since the
//!   last review. With the system clock set back by a day or more, that count
//!   turns negative, retrievability exceeds 1, and the new stability shrinks or
//!   becomes NaN, which SQLite stores as NULL: a row that no later review could
//!   read. A review that would precede the card's last one is therefore taken
//!   to happen at the instant of the last one.
//! - **Errors, not guesses.** A row whose `algorithm`, `fsrs_state` or memory
//!   state FSRS cannot interpret is refused by [`fsrs_card_from_state`], and so
//!   by `submit_review`: a state written by a newer build is never silently
//!   rescheduled by an older one.
//! - **`review_log.scheduled_days` is the interval the review set.** rs-fsrs's
//!   own `ReviewLog.scheduled_days` repeats the previous interval; the store
//!   defines the column as the new one (what Anki's revlog calls `ivl`).

use chrono::{DateTime, NaiveDate, Utc};
use rs_fsrs::{Card, Parameters, Rating, SchedulingInfo, State, FSRS};
use serde::{Deserialize, Serialize};

use super::whole_millis;
use crate::store::{ReviewLogRow, SrsStateRow, StoreError, StoreResult};

/// The `srs_state.algorithm` this module writes and accepts.
const ALGORITHM: &str = "fsrs";

/// The learner's answer to a review. Serialized as its number, the one Anki
/// and FSRS use: 1 = Again, 2 = Hard, 3 = Good, 4 = Easy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
pub enum ReviewRating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl ReviewRating {
    /// Every rating, in the order the review buttons show them.
    pub const ALL: [ReviewRating; 4] = [
        ReviewRating::Again,
        ReviewRating::Hard,
        ReviewRating::Good,
        ReviewRating::Easy,
    ];
}

impl From<ReviewRating> for u8 {
    fn from(rating: ReviewRating) -> Self {
        rating as u8
    }
}

impl TryFrom<u8> for ReviewRating {
    type Error = InvalidRating;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ReviewRating::Again),
            2 => Ok(ReviewRating::Hard),
            3 => Ok(ReviewRating::Good),
            4 => Ok(ReviewRating::Easy),
            other => Err(InvalidRating(other)),
        }
    }
}

impl From<ReviewRating> for Rating {
    fn from(rating: ReviewRating) -> Self {
        match rating {
            ReviewRating::Again => Rating::Again,
            ReviewRating::Hard => Rating::Hard,
            ReviewRating::Good => Rating::Good,
            ReviewRating::Easy => Rating::Easy,
        }
    }
}

/// A number that is not a [`ReviewRating`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a review rating is 1 (Again) to 4 (Easy), not {0}")]
pub struct InvalidRating(pub u8);

/// What one review did: the card's next state and the review as logged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub next: SrsStateRow,
    /// `id` is 0 until the row is stored; `submit_review` fills it in.
    pub log: ReviewLogRow,
}

/// What one rating would do, for labelling the review buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatingPreview {
    pub rating: ReviewRating,
    pub due_date: DateTime<Utc>,
    /// The interval in days; 0 for a learning step, which is minutes away.
    pub interval_days: i64,
}

/// FSRS over `srs_state` rows. Holds nothing but the FSRS parameters, so one
/// scheduler serves every card. `Default` uses `rs_fsrs::Parameters::default()`.
#[derive(Debug, Clone, Default)]
pub struct SrsScheduler {
    fsrs: FSRS,
}

impl SrsScheduler {
    /// A scheduler with `params` as given. rs-fsrs panics while reviewing if
    /// `params.maximum_interval` is below 1, so such a value must not get here.
    pub fn new(params: Parameters) -> Self {
        Self {
            fsrs: FSRS::new(params),
        }
    }

    /// The state of a card that has never been reviewed: `new`, due at `now`.
    pub fn initial_state(&self, card_id: i64, now: DateTime<Utc>) -> SrsStateRow {
        state_from_fsrs_card(card_id, &unseen_card(now), None)
    }

    /// The state after answering `rating` at `now`, and the review as it is
    /// logged for `session_date`. `now` counts from its whole millisecond and,
    /// for a card reviewed before, from no earlier than that previous review.
    ///
    /// This never fails: a row that [`fsrs_card_from_state`] refuses is
    /// scheduled as a card never seen before. `submit_review` refuses such
    /// rows instead, so a stored state is never reset that way.
    pub fn review(
        &self,
        state: &SrsStateRow,
        rating: ReviewRating,
        now: DateTime<Utc>,
        session_date: NaiveDate,
    ) -> ReviewOutcome {
        let card = fsrs_card_from_state(state).unwrap_or_else(|_| unseen_card(now));
        self.review_card(state.card_id, card, rating, now, session_date)
    }

    /// [`SrsScheduler::review`], refusing a row FSRS cannot interpret.
    pub(super) fn try_review(
        &self,
        state: &SrsStateRow,
        rating: ReviewRating,
        now: DateTime<Utc>,
        session_date: NaiveDate,
    ) -> StoreResult<ReviewOutcome> {
        let card = fsrs_card_from_state(state)?;
        Ok(self.review_card(state.card_id, card, rating, now, session_date))
    }

    /// The due date and interval each rating would give at `now`, from Again
    /// to Easy: what [`SrsScheduler::review`] would produce for that rating.
    /// Empty for a row that [`fsrs_card_from_state`] refuses.
    pub fn preview(&self, state: &SrsStateRow, now: DateTime<Utc>) -> Vec<RatingPreview> {
        let Ok(card) = fsrs_card_from_state(state) else {
            return Vec::new();
        };
        let now = review_instant(&card, now);
        let outcomes = self.fsrs.repeat(card, now);
        ReviewRating::ALL
            .iter()
            .filter_map(|&rating| {
                let outcome = outcomes.get(&rating.into())?;
                Some(RatingPreview {
                    rating,
                    due_date: outcome.card.due,
                    interval_days: outcome.card.scheduled_days,
                })
            })
            .collect()
    }

    fn review_card(
        &self,
        card_id: i64,
        card: Card,
        rating: ReviewRating,
        now: DateTime<Utc>,
        session_date: NaiveDate,
    ) -> ReviewOutcome {
        let now = review_instant(&card, now);
        let SchedulingInfo { card, review_log } = self.fsrs.next(card, now, rating.into());
        let next = state_from_fsrs_card(card_id, &card, Some(rating.into()));
        let log = ReviewLogRow {
            id: 0,
            card_id,
            rating: rating.into(),
            reviewed_at: now,
            session_date,
            state_before: state_name(review_log.state).to_owned(),
            scheduled_days: next.interval_days,
            elapsed_days: review_log.elapsed_days,
        };
        ReviewOutcome { next, log }
    }
}

/// The rs-fsrs card for a stored state.
///
/// The mapping is one to one: `due_date` and `due`, `interval_days` and
/// `scheduled_days`, `repetitions` and `reps`, `lapses`, `elapsed_days`, and
/// `fsrs_state` and `state` (`new`, `learning`, `review`, `relearning`). A
/// `new` card has no memory state yet: `stability`, `difficulty` and
/// `last_reviewed_at` are `None` in the row, the first two are `0.0` in
/// rs-fsrs, and its `last_review` (which rs-fsrs ignores for new cards) is the
/// due date. Every other card has all three. [`state_from_fsrs_card`] maps
/// back, so a row survives the round trip unchanged.
///
/// Fails with [`StoreError::Invalid`] for a row FSRS cannot interpret: another
/// `algorithm`, an unknown `fsrs_state`, a memory state that contradicts the
/// `fsrs_state` or is not positive and finite, or a negative count or one
/// beyond rs-fsrs's `i32`.
pub fn fsrs_card_from_state(state: &SrsStateRow) -> StoreResult<Card> {
    let invalid =
        |reason: String| StoreError::Invalid(format!("srs_state row {}: {reason}", state.card_id));
    if state.algorithm != ALGORITHM {
        return Err(invalid(format!(
            "algorithm {:?} is not {ALGORITHM:?}",
            state.algorithm
        )));
    }
    let fsrs_state = parse_state(&state.fsrs_state)
        .ok_or_else(|| invalid(format!("unknown fsrs_state {:?}", state.fsrs_state)))?;
    let count = |name: &str, value: i64| {
        i32::try_from(value)
            .ok()
            .filter(|count| *count >= 0)
            .ok_or_else(|| invalid(format!("{name} {value} is not a count rs-fsrs can hold")))
    };
    let reps = count("repetitions", state.repetitions)?;
    let lapses = count("lapses", state.lapses)?;
    let (stability, difficulty, last_review) = match (
        fsrs_state,
        state.stability,
        state.difficulty,
        state.last_reviewed_at,
    ) {
        (State::New, None, None, None) => (0.0, 0.0, state.due_date),
        (State::New, ..) => {
            return Err(invalid(
                "a new card cannot have a stability, difficulty or last review".into(),
            ))
        }
        (_, Some(stability), Some(difficulty), Some(last_review))
            if is_memory_value(stability) && is_memory_value(difficulty) =>
        {
            (stability, difficulty, last_review)
        }
        _ => {
            return Err(invalid(format!(
                "a {} card needs a positive stability and difficulty and a last review",
                state.fsrs_state
            )))
        }
    };
    Ok(Card {
        due: state.due_date,
        stability,
        difficulty,
        elapsed_days: state.elapsed_days,
        scheduled_days: state.interval_days,
        reps,
        lapses,
        state: fsrs_state,
        last_review,
    })
}

/// The stored state for an rs-fsrs card: the inverse of
/// [`fsrs_card_from_state`]. `last_rating` is stored as given, because rs-fsrs
/// cards do not carry it.
pub fn state_from_fsrs_card(card_id: i64, card: &Card, last_rating: Option<u8>) -> SrsStateRow {
    let reviewed = card.state != State::New;
    SrsStateRow {
        card_id,
        algorithm: ALGORITHM.to_owned(),
        stability: reviewed.then_some(card.stability),
        difficulty: reviewed.then_some(card.difficulty),
        interval_days: card.scheduled_days,
        repetitions: card.reps.into(),
        lapses: card.lapses.into(),
        fsrs_state: state_name(card.state).to_owned(),
        elapsed_days: card.elapsed_days,
        due_date: card.due,
        last_reviewed_at: reviewed.then_some(card.last_review),
        last_rating,
    }
}

/// A card never reviewed, due at `now`. Built without `Card::new()`, which
/// would read the system clock.
fn unseen_card(now: DateTime<Utc>) -> Card {
    Card {
        due: whole_millis(now),
        ..Card::default()
    }
}

/// The instant rs-fsrs reviews `card` at: `now` in whole milliseconds, and no
/// earlier than the card's previous review.
fn review_instant(card: &Card, now: DateTime<Utc>) -> DateTime<Utc> {
    let now = whole_millis(now);
    match card.state {
        State::New => now,
        State::Learning | State::Review | State::Relearning => {
            now.max(whole_millis(card.last_review))
        }
    }
}

fn is_memory_value(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn state_name(state: State) -> &'static str {
    match state {
        State::New => "new",
        State::Learning => "learning",
        State::Review => "review",
        State::Relearning => "relearning",
    }
}

fn parse_state(name: &str) -> Option<State> {
    match name {
        "new" => Some(State::New),
        "learning" => Some(State::Learning),
        "review" => Some(State::Review),
        "relearning" => Some(State::Relearning),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_names_round_trip_and_are_exact() {
        for state in [
            State::New,
            State::Learning,
            State::Review,
            State::Relearning,
        ] {
            assert_eq!(parse_state(state_name(state)), Some(state));
        }
        for name in ["", "New", "REVIEW", " review", "suspended"] {
            assert_eq!(parse_state(name), None, "{name:?}");
        }
    }

    #[test]
    fn ratings_map_onto_the_rs_fsrs_ratings_with_the_same_number() {
        for rating in ReviewRating::ALL {
            let fsrs = Rating::from(rating);
            assert_eq!(fsrs as u8, u8::from(rating));
            assert_eq!(ReviewRating::try_from(fsrs as u8), Ok(rating));
        }
    }
}
