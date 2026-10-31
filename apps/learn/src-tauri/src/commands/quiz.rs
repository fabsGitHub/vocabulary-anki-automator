//! `list_learned_cards`, `next_exercise`, `check_exercise`,
//! `record_exercise_result` (Milestone M).
//!
//! Every exercise is generated from a freshly loaded
//! `LearnedVocabularyPool` and from nothing else (Milestone M1's isolation
//! guarantee, docs/architecture.md § 7.2): this module hands the core the
//! pool and the request, never extra vocabulary.

use tauri::State;
use vocab_core::quiz::{
    check_answer, generate_exercise, Exercise, ExerciseAnswer, ExerciseKind, LearnedVocabularyPool,
    PoolEntry,
};

use crate::{error::Error, progress, state::AppState};

/// The learned vocabulary: every card with an `srs_state` row.
#[tauri::command]
pub async fn list_learned_cards(state: State<'_, AppState>) -> Result<Vec<PoolEntry>, Error> {
    let pool = state.with_store(LearnedVocabularyPool::load)?;
    Ok(pool.entries().to_vec())
}

/// An exercise of `kind` about the learned card `card_id`; the same seed
/// gives the same exercise.
#[tauri::command]
pub async fn next_exercise(
    state: State<'_, AppState>,
    card_id: i64,
    kind: ExerciseKind,
    seed: u32,
) -> Result<Exercise, Error> {
    let pool = state.with_store(LearnedVocabularyPool::load)?;
    Ok(generate_exercise(&pool, card_id, kind, seed)?)
}

/// Whether `answer` solves `exercise`.
#[tauri::command]
pub fn check_exercise(exercise: Exercise, answer: ExerciseAnswer) -> bool {
    check_answer(&exercise, &answer)
}

/// Records an answer about the learned card `card_id`; returns the XP
/// awarded, which only a correct answer earns.
#[tauri::command]
pub async fn record_exercise_result(
    state: State<'_, AppState>,
    card_id: i64,
    kind: ExerciseKind,
    correct: bool,
) -> Result<i64, Error> {
    let clock = state.clock();
    let xp =
        state.with_store(|store| progress::record_exercise(store, card_id, correct, &clock))?;
    log::debug!(
        "{} exercise about card {card_id}: {}.",
        kind.as_str(),
        if correct { "correct" } else { "wrong" }
    );
    Ok(xp)
}
