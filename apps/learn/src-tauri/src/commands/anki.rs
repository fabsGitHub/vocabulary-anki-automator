//! `anki_export`, `anki_import` (Milestone I): see [`crate::anki`].

use tauri::State;

use crate::{
    anki::{self, AnkiSyncReport},
    error::Error,
    state::AppState,
};

/// Pushes every unsynced card Learn created into the configured deck. Anki
/// being unreachable is reported per card, not as an error.
#[tauri::command]
pub async fn anki_export(state: State<'_, AppState>) -> Result<AnkiSyncReport, Error> {
    anki::export(&state).await
}

/// Imports the Chinese vocabulary of Anki deck `deck`; fails when Anki cannot
/// be asked for the deck at all.
#[tauri::command]
pub async fn anki_import(
    state: State<'_, AppState>,
    deck: String,
) -> Result<AnkiSyncReport, Error> {
    anki::import(&state, &deck).await
}
