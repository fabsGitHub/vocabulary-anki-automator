//! `get_accessibility_status`, `request_accessibility`,
//! `close_setup_assistant`: the parts of the first-run setup assistant that
//! are not Anki setup (that is `commands::setup`).
//!
//! The assistant itself lives in the settings window. Showing it at startup
//! is fine; everything it installs goes through the consent-gated commands
//! of `commands::setup`, and these commands install nothing.

use tauri::State;

use crate::{accessibility::AccessibilityStatus, config::SetupAssistantState, state::AppState};

/// Whether this platform needs the Accessibility permission, and whether
/// Polywort has it (`AXIsProcessTrusted`). Polled every second while the
/// assistant shows that step; asks nothing.
#[tauri::command]
pub fn get_accessibility_status(state: State<'_, AppState>) -> AccessibilityStatus {
    state.accessibility().status()
}

/// The assistant's "Open System Settings": adds Polywort to the
/// Accessibility list with macOS's own alert
/// (`AXIsProcessTrustedWithOptions` with the prompt option), then opens
/// System Settings at that list. Does nothing where the permission is not
/// required, or already granted. Only a click calls it.
#[tauri::command]
pub async fn request_accessibility(
    state: State<'_, AppState>,
) -> Result<AccessibilityStatus, String> {
    let accessibility = state.accessibility();
    tauri::async_runtime::spawn_blocking(move || {
        let status = accessibility.status();
        if !status.required || status.trusted {
            return Ok(status);
        }
        let trusted = accessibility.prompt();
        log::info!("Asked macOS for the Accessibility permission.");
        if !trusted {
            accessibility.open_settings()?;
        }
        Ok(AccessibilityStatus {
            required: true,
            trusted,
        })
    })
    .await
    .map_err(|err| format!("The permission could not be requested: {err}"))?
}

/// Records that the user finished (`finished`) or dismissed the setup
/// assistant, so the settings window stops opening it at startup.
#[tauri::command]
pub fn close_setup_assistant(
    state: State<'_, AppState>,
    finished: bool,
) -> Result<SetupAssistantState, String> {
    let now = state.now();
    let closed = state
        .update_config(|config| {
            config.setup_assistant.close(finished, now);
            config.setup_assistant.clone()
        })
        .map_err(|err| format!("The setup assistant could not be closed: {err}"))?;
    log::info!(
        "Setup assistant {}.",
        if finished { "finished" } else { "dismissed" }
    );
    Ok(closed)
}
