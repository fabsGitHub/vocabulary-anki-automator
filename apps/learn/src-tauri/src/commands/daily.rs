//! `get_daily_prompt_state`, `submit_daily_quota`, `dismiss_daily_prompt`,
//! `reopen_daily_prompt`, `get_daily_queue`, `start_new_item` (Milestones L2
//! and L3).
//!
//! The prompt's rules are vocab-core's (docs/architecture.md § 7.1): asked
//! once per local day, and closing it without an answer means reviews only
//! today, until the menu action reopens it.

use tauri::State;
use vocab_core::{
    srs::{self, DailyPromptState, QueueItem},
    store::{DailySessionRow, StoreError},
};

use crate::{error::Error, session, state::AppState};

/// Whether to show the prompt before anything else, and today's session.
#[tauri::command]
pub async fn get_daily_prompt_state(state: State<'_, AppState>) -> Result<DailyPromptState, Error> {
    let clock = state.clock();
    state.with_store(|store| srs::daily_prompt_state(store, &clock))
}

/// Records the answer "`new_count` new words today". An answer that lowers
/// the quota to what is already done completes the session.
#[tauri::command]
pub async fn submit_daily_quota(
    state: State<'_, AppState>,
    new_count: u32,
) -> Result<DailySessionRow, Error> {
    let clock = state.clock();
    state.with_store(|store| {
        session::track_completion(store, &clock, |store| {
            srs::submit_daily_quota(store, new_count, &clock)
        })
        .map(|(session, _)| session)
    })
}

/// "Not today": a session with no new words, reviews only (or today's session
/// unchanged, if the prompt had been answered before).
#[tauri::command]
pub async fn dismiss_daily_prompt(state: State<'_, AppState>) -> Result<DailySessionRow, Error> {
    let clock = state.clock();
    state.with_store(|store| {
        session::track_completion(store, &clock, |store| {
            srs::dismiss_daily_prompt(store, &clock)
        })
        .map(|(session, _)| session)
    })
}

/// The menu action that asks the prompt again today.
#[tauri::command]
pub async fn reopen_daily_prompt(state: State<'_, AppState>) -> Result<(), Error> {
    let clock = state.clock();
    state.with_store(|store| srs::reopen_daily_prompt(store, &clock))
}

/// Today's queue: due reviews, then what is left of the new-word quota.
#[tauri::command]
pub async fn get_daily_queue(state: State<'_, AppState>) -> Result<Vec<QueueItem>, Error> {
    let clock = state.clock();
    state.with_store_mut(|store| {
        state.ensure_curriculum(store)?;
        Ok::<_, Error>(session::daily_queue(store, &clock)?)
    })
}

/// Introduces the curriculum word `hanzi` with a meaning in the settings'
/// language and returns its card id, ready for its first rating. Idempotent,
/// and it counts no progress: the first rating does.
#[tauri::command]
pub async fn start_new_item(state: State<'_, AppState>, hanzi: String) -> Result<i64, Error> {
    let clock = state.clock();
    let meaning_lang = state.settings().meaning_lang;
    let hanzi = hanzi.trim();
    state.with_store_mut(|store| {
        state.ensure_curriculum(store)?;
        srs::introduce_new_item(
            store,
            state.scheduler(),
            hanzi,
            meaning_lang.as_str(),
            &clock,
        )
        .map_err(|err| match err {
            StoreError::NotFound(_) => Error::NotInCurriculum(hanzi.to_string()),
            other => other.into(),
        })
    })
}
