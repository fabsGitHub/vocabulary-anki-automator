/**
 * The two webview-window calls the frontend makes. Both windows load the same
 * bundle (docs/architecture.md § 8.3), so the label is what tells them apart,
 * and `hide` (never `close`) keeps the `preview` window alive for the next
 * capture.
 */

import { getCurrentWindow } from '@tauri-apps/api/window'

/** `main` or `preview`. */
export function currentWindowLabel(): string {
  return getCurrentWindow().label
}

export function hideCurrentWindow(): Promise<void> {
  return getCurrentWindow().hide()
}
