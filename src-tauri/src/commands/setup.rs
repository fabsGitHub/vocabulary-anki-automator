//! `get_setup_status`, `grant_consent`, `download_and_install_anki`,
//! `auto_provision_ankiconnect`, `check_ankiconnect_installed` (Milestone E).
//!
//! The two install commands are the only way into the install code, and the
//! install code checks the recorded consent itself; nothing here runs at
//! startup.

use tauri::State;

use crate::{
    config::{ConsentAction, ConsentState},
    setup::{self, anki_install, ankiconnect, SetupStatus},
    state::AppState,
};

/// Why an install command refused to start.
const INSTALL_RUNNING: &str = "Another installation is still running. Wait for it to finish.";

/// Whether Anki and AnkiConnect are installed, whether this platform has a
/// pinned installer, and the recorded consents.
#[tauri::command]
pub fn get_setup_status(state: State<'_, AppState>) -> SetupStatus {
    setup::setup_status(state.setup(), &state.config().consent)
}

/// Records, with the current time, that the user agreed to `action` after
/// the consent dialog explained it.
#[tauri::command]
pub fn grant_consent(
    state: State<'_, AppState>,
    action: ConsentAction,
) -> Result<ConsentState, String> {
    let now = state.now();
    let consent = state
        .update_config(|config| {
            config.consent.grant(action, now);
            config.consent.clone()
        })
        .map_err(|err| format!("The consent could not be saved: {err}"))?;
    log::info!("Consent recorded: {}.", action.describe());
    Ok(consent)
}

/// Downloads, verifies and installs the pinned Anki; refuses without consent
/// and while another install is running.
#[tauri::command]
pub async fn download_and_install_anki(state: State<'_, AppState>) -> Result<String, String> {
    let Some(_installing) = state.begin_install() else {
        return Err(INSTALL_RUNNING.to_string());
    };
    let consent = state.config().consent;
    let env = state.setup();
    let paths = env.paths().map_err(|err| err.to_string())?;
    anki_install::install(
        &consent,
        paths,
        env.downloader.as_ref(),
        env.runner.as_ref(),
    )
    .await
    .map_err(|err| err.to_string())
}

/// Downloads, verifies and installs the pinned AnkiConnect; refuses without
/// consent and while another install is running, and never overwrites an
/// existing add-on.
#[tauri::command]
pub async fn auto_provision_ankiconnect(state: State<'_, AppState>) -> Result<String, String> {
    let Some(_installing) = state.begin_install() else {
        return Err(INSTALL_RUNNING.to_string());
    };
    let consent = state.config().consent;
    let env = state.setup();
    let paths = env.paths().map_err(|err| err.to_string())?;
    let installed = ankiconnect::provision(&consent, paths, env.downloader.as_ref())
        .await
        .map_err(|err| err.to_string())?;
    Ok(format!(
        "AnkiConnect was installed into {}. Restart Anki to load it.",
        installed.display()
    ))
}

/// Whether an AnkiConnect add-on is in Anki's add-on folder.
#[tauri::command]
pub fn check_ankiconnect_installed(state: State<'_, AppState>) -> bool {
    state
        .setup()
        .paths()
        .is_ok_and(setup::check_ankiconnect_installed)
}
