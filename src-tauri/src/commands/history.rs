//! `list_recent_captures`, `delete_capture`, `sync_pending_captures`
//! (Milestone F).

use tauri::{AppHandle, Runtime, State};
use vocab_core::StoredCapture;

use crate::{
    capture,
    push::{self, deck_key, SyncReport},
    state::AppState,
};

/// Most captures `list_recent_captures` returns at once.
pub const MAX_LIST_LIMIT: u32 = 500;

/// The newest captures, at most `limit` (clamped to `1..=500`). The audio is
/// left out: the history list shows text only, and each clip would add
/// kilobytes of base64 to the IPC payload (docs/architecture.md § 6.3).
#[tauri::command]
pub async fn list_recent_captures(
    state: State<'_, AppState>,
    limit: i64,
) -> Result<Vec<StoredCapture>, String> {
    let limit = u32::try_from(limit.clamp(1, i64::from(MAX_LIST_LIMIT))).unwrap_or(MAX_LIST_LIMIT);
    let mut captures = state.with_store(|store| store.list_recent_captures(limit))?;
    for capture in &mut captures {
        capture.card.audio_front = None;
    }
    Ok(captures)
}

/// Deletes capture `id` (and its review state); `false` when it did not
/// exist.
#[tauri::command]
pub async fn delete_capture<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    id: i64,
) -> Result<bool, String> {
    let deleted = state.with_store(|store| {
        store.with_transaction(|store| {
            let deleted = store.delete_capture(id)?;
            store.delete_state(&deck_key(id))?;
            Ok(deleted)
        })
    })?;
    if deleted {
        log::info!("Deleted capture {id}.");
        capture::emit_captures_changed(&app);
    }
    Ok(deleted)
}

/// Pushes every capture Anki has not received yet (docs/architecture.md
/// § 8.2).
#[tauri::command]
pub async fn sync_pending_captures<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<SyncReport, String> {
    let report = push::sync_pending(&state, &state.config()).await?;
    if report.synced > 0 {
        capture::emit_captures_changed(&app);
    }
    Ok(report)
}
