/**
 * Typed subscriptions to the Automator's events (docs/architecture.md § 8.2).
 *
 * `listen` (not the window-scoped variant) is used on purpose: it receives an
 * event whether the backend emitted it app-wide or to one window label, so the
 * frontend does not depend on how the Rust side targets `translation-payload`.
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type { CaptureErrorPayload, PendingCapture } from './types'

export type { UnlistenFn }

/** A new capture is ready for approval; sent to the `preview` window. */
export function onTranslationPayload(
  handler: (payload: PendingCapture) => void,
): Promise<UnlistenFn> {
  return listen<PendingCapture>('translation-payload', (event) => handler(event.payload))
}

/** A capture failed before it produced a card (empty selection, text too long, ...). */
export function onCaptureError(
  handler: (payload: CaptureErrorPayload) => void,
): Promise<UnlistenFn> {
  return listen<CaptureErrorPayload>('capture-error', (event) => handler(event.payload))
}

/** The history changed (a card was saved, deleted or synced). */
export function onCapturesChanged(handler: () => void): Promise<UnlistenFn> {
  return listen<null>('captures-changed', () => handler())
}
