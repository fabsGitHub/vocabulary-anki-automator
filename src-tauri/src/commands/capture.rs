//! `get_pending_payload`, `discard_pending_payload`,
//! `execute_card_generation`, `capture_text`.

use tauri::{AppHandle, Manager, Runtime, State};
use vocab_core::Card;

use crate::{
    capture::{self, PendingCapture, PREVIEW_LABEL},
    push::{self, SubmitResult},
    state::AppState,
};

/// The capture waiting for approval, if any. A freshly opened approval
/// window calls this, since it missed the `translation-payload` event.
#[tauri::command]
pub fn get_pending_payload(state: State<'_, AppState>) -> Option<PendingCapture> {
    state.pending().get()
}

/// Drops the pending capture and hides the approval window.
#[tauri::command]
pub fn discard_pending_payload<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) {
    state.pending().clear();
    hide_preview(&app);
}

/// Saves the approved `card` and pushes it to Anki (docs/architecture.md
/// § 8.2). Anki being unreachable is reported in the result, not as an
/// error; an error means the card was saved nowhere and stays pending.
#[tauri::command]
pub async fn execute_card_generation<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    card: Card,
) -> Result<SubmitResult, String> {
    let Some(_submitting) = state.begin_submit() else {
        return Err("This card is already being saved.".to_string());
    };
    let config = state.config();
    let pending = state.pending().snapshot();
    let result = push::submit_card(&state, &config, card, pending.as_ref().map(|(_, p)| p)).await?;
    if let Some((generation, _)) = pending {
        state.pending().clear_if(generation);
    }
    if state.has_store() {
        capture::emit_captures_changed(&app);
    }
    Ok(result)
}

/// Runs `text` through the capture flow exactly as if the hotkey of
/// `profile_id` (`None`: the default hotkey) had copied it: the internal
/// capture event for tests and E2E runs. Returns once the capture is pending
/// (or reported as a `capture-error`).
#[tauri::command]
pub async fn capture_text<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    text: String,
    profile_id: Option<String>,
) -> Result<(), String> {
    let profile = state
        .config()
        .profile(profile_id.as_deref())
        .ok_or_else(|| {
            format!(
                "There is no hotkey profile \"{}\".",
                profile_id.unwrap_or_default()
            )
        })?;
    let Some(_running) = state.begin_capture() else {
        return Err("A capture is already running.".to_string());
    };
    // A refused capture has been reported as `capture-error`, as on the hotkey path.
    let _ = capture::run_capture(&app, &state, text, &profile).await;
    Ok(())
}

pub(crate) fn hide_preview<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(PREVIEW_LABEL) {
        if let Err(err) = window.hide() {
            log::warn!("Could not hide the approval window: {err}");
        }
    }
}
