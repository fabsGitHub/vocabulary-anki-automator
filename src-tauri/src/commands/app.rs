//! `capture_in_progress`, `restart_app` (Milestone N4): the updater banner
//! asks before it checks or restarts, so an update never interrupts a
//! capture or throws away a card waiting for approval.

use tauri::{AppHandle, Runtime, State};

use crate::state::AppState;

/// True from the hotkey until the card is approved or discarded.
#[tauri::command]
pub fn capture_in_progress(state: State<'_, AppState>) -> bool {
    state.capture_in_progress()
}

/// Restarts the app, e.g. to apply a downloaded update. Never returns.
#[tauri::command]
pub fn restart_app<R: Runtime>(app: AppHandle<R>) {
    log::info!("Restarting.");
    app.restart()
}
