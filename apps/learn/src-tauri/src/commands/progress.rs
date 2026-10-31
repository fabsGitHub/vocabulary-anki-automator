//! `get_stats`, `list_lessons`, `complete_lesson` (Milestone I).

use tauri::State;
use vocab_core::{
    gamification::{stats_summary, StatsSummary},
    lessons::{lessons_from_store, Lesson},
};

use crate::{
    error::Error,
    progress::{self, CARDS_PER_LESSON},
    state::AppState,
};

/// XP, streak and how much is learned and due, after spending any streak
/// freezes that missed days call for.
#[tauri::command]
pub async fn get_stats(state: State<'_, AppState>) -> Result<StatsSummary, Error> {
    let clock = state.clock();
    state.with_store(|store| stats_summary(store, &clock))
}

/// The lessons made from the learner's own clipboard captures. Titles are
/// English fallbacks; the UI builds localized ones from the languages and
/// each lesson's position.
#[tauri::command]
pub async fn list_lessons(state: State<'_, AppState>) -> Result<Vec<Lesson>, Error> {
    state.with_store(|store| lessons_from_store(store, CARDS_PER_LESSON))
}

/// Records a finished lesson; returns the XP awarded, at most once per lesson
/// per day.
#[tauri::command]
pub async fn complete_lesson(state: State<'_, AppState>, lesson_id: String) -> Result<i64, Error> {
    let clock = state.clock();
    state.with_store(|store| progress::complete_lesson(store, &lesson_id, &clock))
}
