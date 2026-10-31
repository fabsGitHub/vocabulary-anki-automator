//! `anki_status`, `anki_model_field_names`, `launch_anki_desktop`.
//!
//! The frontend used to call AnkiConnect itself, which is why AnkiConnect had
//! to accept every web origin. These commands ask through `anki-bridge`
//! instead: requests from the backend carry no `Origin`, so AnkiConnect's
//! CORS list can stay limited to the app (Milestone E).

use std::{path::Path, time::Duration};

use anki_bridge::AnkiClient;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{setup::os_args, state::AppState};

/// How long the status probe waits for Anki: the settings window polls it,
/// and a closed Anki should show up as offline quickly.
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// What `anki_status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnkiStatus {
    /// AnkiConnect answered.
    pub online: bool,
    /// AnkiConnect's API version.
    pub version: Option<u32>,
    pub decks: Vec<String>,
    /// Note types.
    pub models: Vec<String>,
    /// The first thing that went wrong.
    pub error: Option<String>,
}

/// Whether AnkiConnect answers at the configured URL, plus the decks and
/// note types the settings form offers. Never fails: problems end up in
/// `error`.
#[tauri::command]
pub async fn anki_status(state: State<'_, AppState>) -> Result<AnkiStatus, String> {
    let client = AnkiClient::new(state.config().anki.connect_url).with_timeout(STATUS_TIMEOUT);
    let version = match client.version().await {
        Ok(version) => version,
        Err(err) => {
            return Ok(AnkiStatus {
                online: false,
                version: None,
                decks: Vec::new(),
                models: Vec::new(),
                error: Some(err.to_string()),
            })
        }
    };
    let mut error = None;
    let mut keep_first = |message: String| {
        error.get_or_insert(message);
    };
    let decks = client.deck_names().await.unwrap_or_else(|err| {
        keep_first(err.to_string());
        Vec::new()
    });
    let models = client.model_names().await.unwrap_or_else(|err| {
        keep_first(err.to_string());
        Vec::new()
    });
    Ok(AnkiStatus {
        online: true,
        version: Some(version),
        decks,
        models,
        error,
    })
}

/// The fields of note type `model`, in order: the settings form compares
/// them with the field mapping (Milestone J5).
#[tauri::command]
pub async fn anki_model_field_names(
    state: State<'_, AppState>,
    model: String,
) -> Result<Vec<String>, String> {
    AnkiClient::new(state.config().anki.connect_url)
        .model_field_names(&model)
        .await
        .map_err(|err| err.to_string())
}

/// Starts the Anki desktop app.
#[tauri::command]
pub fn launch_anki_desktop(state: State<'_, AppState>) -> Result<String, String> {
    let setup = state.setup();
    let started = match std::env::consts::OS {
        "macos" => setup
            .runner
            .spawn(Path::new("/usr/bin/open"), &os_args(["-a", "Anki"])),
        "windows" => {
            let app = setup
                .paths()
                .map_err(|err| err.to_string())?
                .anki_app()
                .filter(|app| app.exists())
                .ok_or("Anki is not installed in its default location.")?;
            setup.runner.spawn(&app, &[])
        }
        // Distribution packages put `anki` on the PATH.
        _ => setup.runner.spawn(Path::new("anki"), &[]),
    };
    started.map_err(|err| format!("Could not start Anki: {err}"))?;
    log::info!("Starting Anki.");
    Ok("Starting Anki…".to_string())
}
