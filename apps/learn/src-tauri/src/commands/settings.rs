//! `get_settings`, `save_settings`.

use tauri::State;

use crate::{error::Error, settings::LearnSettings, state::AppState};

/// The current settings.
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> LearnSettings {
    state.settings()
}

/// Normalizes, validates and stores `settings`. Validation problems come
/// back one sentence per line, and nothing is saved.
#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: LearnSettings,
) -> Result<(), Error> {
    state.save_settings(settings)?;
    log::info!("Settings saved.");
    Ok(())
}
