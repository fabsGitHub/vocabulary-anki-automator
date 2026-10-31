//! `get_current_settings`, `apply_settings`.

use tauri::{AppHandle, Runtime, State};

use crate::{config::AppConfig, shortcuts, state::AppState};

/// The current settings.
#[tauri::command]
pub fn get_current_settings(state: State<'_, AppState>) -> AppConfig {
    state.config()
}

/// Normalizes and validates `new_cfg`, saves it and re-registers every
/// hotkey. Validation problems come back as one sentence per line and save
/// nothing. `consent` is ignored: only `grant_consent` records consent.
/// Settings that saved but whose shortcuts the OS refused (another app owns
/// them) are reported as an error too, so the form can say so.
#[tauri::command]
pub fn apply_settings<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    new_cfg: AppConfig,
) -> Result<(), String> {
    let mut candidate = new_cfg;
    candidate.normalize();
    candidate.validate().map_err(|err| err.to_string())?;

    let saved = state
        .update_config(|config| {
            let consent = std::mem::take(&mut config.consent);
            *config = AppConfig {
                consent,
                ..candidate
            };
            config.clone()
        })
        .map_err(|err| format!("The settings were not saved: {err}"))?;
    log::info!("Settings saved.");

    shortcuts::register_all(&app, &saved).map_err(|failures| {
        format!("The settings were saved, but some hotkeys are not active:\n{failures}")
    })
}
