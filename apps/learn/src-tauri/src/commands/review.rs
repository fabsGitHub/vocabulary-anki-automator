//! `get_card`, `preview_ratings`, `submit_review_rating` (Milestones L1 and
//! I).

use tauri::State;
use vocab_core::{
    srs::{RatingPreview, ReviewRating},
    time::Clock,
    StoredCapture,
};

use crate::{
    error::Error,
    session::{self, ReviewResult},
    state::AppState,
};

/// Card `card_id`, its recording included.
#[tauri::command]
pub async fn get_card(state: State<'_, AppState>, card_id: i64) -> Result<StoredCapture, Error> {
    state.with_store(|store| {
        store
            .get_capture(card_id)?
            .ok_or(Error::NoSuchCard(card_id))
    })
}

/// When the card would be due again after each rating, Again to Easy, for
/// labelling the rating buttons.
#[tauri::command]
pub async fn preview_ratings(
    state: State<'_, AppState>,
    card_id: i64,
) -> Result<Vec<RatingPreview>, Error> {
    let clock = state.clock();
    let srs_state = state.with_store(|store| {
        store
            .get_srs_state(card_id)?
            .ok_or(Error::NotScheduled(card_id))
    })?;
    Ok(state.scheduler().preview(&srs_state, clock.now()))
}

/// Rates card `card_id` (1 = Again ... 4 = Easy): the FSRS review, the
/// session progress, XP and, for the rating that completes today's session,
/// the bonus and streak, all in one transaction.
#[tauri::command]
pub async fn submit_review_rating(
    state: State<'_, AppState>,
    card_id: i64,
    rating: ReviewRating,
) -> Result<ReviewResult, Error> {
    let clock = state.clock();
    state.with_store(|store| {
        if store.get_srs_state(card_id)?.is_none() {
            return Err(Error::NotScheduled(card_id));
        }
        Ok(session::rate(
            store,
            state.scheduler(),
            card_id,
            rating,
            &clock,
        )?)
    })
}
