/**
 * Update checks for the main window (Milestone N4).
 *
 * Capturing is what the app is for, so the updater stays out of its way: the
 * first check waits until the app has been idle for a while after start-up, a
 * check never runs while a capture is in progress (it is postponed instead),
 * and finding an update only shows a dismissible banner. Downloading and
 * restarting happen solely on the user's click. Every failure is logged and
 * otherwise swallowed: an update problem must never interrupt the user with a
 * dialog, and never restarts the app.
 *
 * Download and install are two steps on purpose. A download can take a while,
 * and a hotkey pressed meanwhile starts a capture; installing then exits the
 * app (Windows) and the restart follows (macOS, Linux), either of which would
 * drop that card. So capture state is asked again right before installing, and
 * a capture that started during the download keeps the downloaded update for
 * the next click instead.
 */

import { check, type Update } from '@tauri-apps/plugin-updater'
import {
  computed,
  getCurrentScope,
  onScopeDispose,
  ref,
  shallowRef,
  type ComputedRef,
  type Ref,
} from 'vue'

import { captureInProgress, restartApp } from '../api'

export interface UpdaterOptions {
  /** Idle time after start-up before the first check. */
  initialDelayMs?: number
  /** How long a check waits when a capture is in progress. */
  retryDelayMs?: number
  /** Checks attempted before giving up for this session. */
  maxAttempts?: number
}

export interface Updater {
  /** The update found by the check, until it is dismissed. */
  update: Ref<Update | null>
  version: ComputedRef<string | null>
  /** Downloading and installing after the user's click. */
  installing: Ref<boolean>
  /** The last install attempt failed (already logged). */
  failed: Ref<boolean>
  /** A capture was running when the click came or when the download finished; nothing was installed. */
  blockedByCapture: Ref<boolean>
  /** Schedules the first check; called once when the main window mounts. */
  start(): void
  /**
   * Downloads the update (once), asks again whether a capture runs, and only
   * then installs it and restarts the app.
   */
  install(): Promise<void>
  /** Hides the banner for this session. */
  dismiss(): void
  dispose(): void
}

const LOG_PREFIX = '[updater]'

export function useUpdater(options: UpdaterOptions = {}): Updater {
  const initialDelayMs = options.initialDelayMs ?? 10_000
  const retryDelayMs = options.retryDelayMs ?? 10_000
  const maxAttempts = options.maxAttempts ?? 6

  const update = shallowRef<Update | null>(null)
  const installing = ref(false)
  const failed = ref(false)
  const blockedByCapture = ref(false)
  const version = computed(() => update.value?.version ?? null)

  let timer: ReturnType<typeof setTimeout> | null = null
  let attempts = 0
  let started = false
  let disposed = false
  /** The update whose package is downloaded and waiting to be installed. */
  let downloaded: Update | null = null

  /** `true` when a capture runs, or when that cannot be determined: checking then could interrupt one. */
  async function captureBusy(): Promise<boolean> {
    try {
      return await captureInProgress()
    } catch (err) {
      console.warn(LOG_PREFIX, 'capture_in_progress failed; postponing', err)
      return true
    }
  }

  function schedule(delayMs: number): void {
    timer = setTimeout(() => {
      timer = null
      void attempt()
    }, delayMs)
  }

  async function attempt(): Promise<void> {
    if (disposed) return
    attempts++
    if (await captureBusy()) {
      if (!disposed && attempts < maxAttempts) schedule(retryDelayMs)
      return
    }
    if (disposed) return
    try {
      const found = await check()
      if (disposed) {
        await found?.close()
        return
      }
      update.value = found
    } catch (err) {
      console.error(LOG_PREFIX, 'update check failed', err)
    }
  }

  function start(): void {
    if (started || disposed) return
    started = true
    schedule(initialDelayMs)
  }

  async function install(): Promise<void> {
    const pending = update.value
    if (!pending || installing.value) return
    failed.value = false
    blockedByCapture.value = false
    installing.value = true
    try {
      if (await captureBusy()) {
        blockedByCapture.value = true
        return
      }
      if (downloaded !== pending) {
        await pending.download()
        downloaded = pending
      }
      if (await captureBusy()) {
        blockedByCapture.value = true
        return
      }
      await pending.install()
      await restartApp()
    } catch (err) {
      console.error(LOG_PREFIX, 'installing the update failed', err)
      downloaded = null
      failed.value = true
    } finally {
      installing.value = false
    }
  }

  function dismiss(): void {
    const pending = update.value
    update.value = null
    downloaded = null
    pending
      ?.close()
      .catch((err: unknown) => console.warn(LOG_PREFIX, 'releasing the update failed', err))
  }

  function dispose(): void {
    disposed = true
    if (timer !== null) clearTimeout(timer)
    timer = null
  }

  if (getCurrentScope()) onScopeDispose(dispose)

  return { update, version, installing, failed, blockedByCapture, start, install, dismiss, dispose }
}
