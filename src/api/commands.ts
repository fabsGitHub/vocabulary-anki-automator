/**
 * Typed wrappers around every Automator command (docs/architecture.md § 8.2).
 *
 * This is the only module that calls `invoke`: command names and the
 * camelCase argument names Tauri maps onto the Rust parameters exist here and
 * nowhere else, so a change to the IPC surface is a change to this file.
 * Commands reject with the backend's error string; see {@link errorMessage}.
 */

import { invoke } from '@tauri-apps/api/core'

import type {
  AnkiStatus,
  AppConfig,
  Card,
  ConsentAction,
  ConsentState,
  PendingCapture,
  SetupStatus,
  StoredCapture,
  SubmitResult,
  SyncReport,
} from './types'

export function getCurrentSettings(): Promise<AppConfig> {
  return invoke<AppConfig>('get_current_settings')
}

/** Validates, saves and re-registers the shortcuts; rejects with the validation error. */
export function applySettings(newCfg: AppConfig): Promise<null> {
  return invoke<null>('apply_settings', { newCfg })
}

export function getPendingPayload(): Promise<PendingCapture | null> {
  return invoke<PendingCapture | null>('get_pending_payload')
}

/** Forgets the pending capture and hides the `preview` window. */
export function discardPendingPayload(): Promise<null> {
  return invoke<null>('discard_pending_payload')
}

/** Saves `card` to the local history first, then pushes it to Anki. */
export function executeCardGeneration(card: Card): Promise<SubmitResult> {
  return invoke<SubmitResult>('execute_card_generation', { card })
}

/** Runs the capture pipeline on `text` as if the hotkey had copied it. */
export function captureText(text: string, profileId: string | null): Promise<null> {
  return invoke<null>('capture_text', { text, profileId })
}

export function ankiStatus(): Promise<AnkiStatus> {
  return invoke<AnkiStatus>('anki_status')
}

export function ankiModelFieldNames(model: string): Promise<string[]> {
  return invoke<string[]>('anki_model_field_names', { model })
}

export function launchAnkiDesktop(): Promise<string> {
  return invoke<string>('launch_anki_desktop')
}

export function getSetupStatus(): Promise<SetupStatus> {
  return invoke<SetupStatus>('get_setup_status')
}

/** Records the user's consent to one install action (§ 8.4). */
export function grantConsent(action: ConsentAction): Promise<ConsentState> {
  return invoke<ConsentState>('grant_consent', { action })
}

/** Fails unless `grant_consent('anki_install')` succeeded first. */
export function downloadAndInstallAnki(): Promise<string> {
  return invoke<string>('download_and_install_anki')
}

/** Fails unless `grant_consent('ankiconnect_provision')` succeeded first. */
export function autoProvisionAnkiconnect(): Promise<string> {
  return invoke<string>('auto_provision_ankiconnect')
}

export function checkAnkiconnectInstalled(): Promise<boolean> {
  return invoke<boolean>('check_ankiconnect_installed')
}

/** Newest first; `audio_front` is not included. */
export function listRecentCaptures(limit: number): Promise<StoredCapture[]> {
  return invoke<StoredCapture[]>('list_recent_captures', { limit })
}

/** `false` when no capture had this id. */
export function deleteCapture(id: number): Promise<boolean> {
  return invoke<boolean>('delete_capture', { id })
}

export function syncPendingCaptures(): Promise<SyncReport> {
  return invoke<SyncReport>('sync_pending_captures')
}

/** Whether a capture is running or waiting for approval. */
export function captureInProgress(): Promise<boolean> {
  return invoke<boolean>('capture_in_progress')
}

/** Restarts the app; the promise never settles on success. */
export function restartApp(): Promise<never> {
  return invoke<never>('restart_app')
}

/**
 * A human-readable message for a rejected command. Tauri commands reject with
 * the `String` of their `Result`, but a panic or a typed error arrives as an
 * `Error` or an object.
 */
export function errorMessage(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  if (err && typeof err === 'object' && 'message' in err) {
    const message = (err as { message: unknown }).message
    if (typeof message === 'string') return message
  }
  try {
    return JSON.stringify(err) ?? String(err)
  } catch {
    return String(err)
  }
}
