/**
 * The Anki connection state machine behind the "Anki connection" tab and the
 * header badge (Milestones E and G).
 *
 * The old view probed AnkiConnect at 127.0.0.1:8765 straight from the
 * webview, which only worked because the add-on's CORS list was opened to
 * every origin. Every probe now goes through the backend and resolves into
 * one state, most fundamental problem first:
 *
 *   checking -> anki_missing -> plugin_missing -> anki_closed -> online
 *
 * `anki_status` answers "online?" live; only when it is offline does the
 * disk-only `get_setup_status` say which step is missing. Installing is
 * always two-step (§ 8.4): an install request opens the consent dialog, and
 * only "Agree" records the consent with `grant_consent` and then starts the
 * download, so nothing is fetched without an explicit click.
 */

import {
  computed,
  getCurrentScope,
  inject,
  onScopeDispose,
  provide,
  ref,
  type ComputedRef,
  type InjectionKey,
  type Ref,
} from 'vue'

import {
  ankiStatus,
  autoProvisionAnkiconnect,
  downloadAndInstallAnki,
  errorMessage,
  getSetupStatus,
  grantConsent,
  launchAnkiDesktop,
  type AnkiStatus,
  type ConsentAction,
  type ConsentState,
  type SetupStatus,
} from '../api'
import type { MessageKey } from '../i18n'

export type AnkiConnectionState =
  'checking' | 'anki_missing' | 'plugin_missing' | 'anki_closed' | 'online'

/** The user-triggered operation in flight; at most one at a time. */
export type AnkiActivity = 'rescan' | 'launch' | 'install' | 'provision'

/** A message for the tab's feedback line, as a key the view translates. */
export interface AnkiFeedback {
  tone: 'info' | 'success' | 'error'
  key: MessageKey
  params?: Record<string, string | number>
}

export interface AnkiConnectionOptions {
  /** Delay between `anki_status` polls after a launch. */
  pollIntervalMs?: number
  /** Polls before a launch counts as timed out. */
  maxPollAttempts?: number
  /** Called with what `grant_consent` returned, so the settings form keeps it. */
  onConsentChanged?: (consent: ConsentState) => void
}

export interface AnkiConnection {
  state: Ref<AnkiConnectionState>
  /** The last live check; `null` when `anki_status` itself failed. */
  status: Ref<AnkiStatus | null>
  /** The last disk check; only refreshed while Anki is offline. */
  setup: Ref<SetupStatus | null>
  activity: Ref<AnkiActivity | null>
  /** The install action whose consent dialog is open. */
  pendingConsent: Ref<ConsentAction | null>
  feedback: Ref<AnkiFeedback | null>
  decks: ComputedRef<string[]>
  models: ComputedRef<string[]>
  /** "Install Anki" is offered only when Anki is missing and the platform has an installer. */
  canInstallAnki: ComputedRef<boolean>
  /** The pinned Anki release the installer fetches. */
  ankiVersion: ComputedRef<string>
  /** Re-resolves the state without passing through `checking` (startup, window focus). */
  check(): Promise<AnkiConnectionState>
  /** A visible re-check: `checking`, then the resolved state and a feedback line. */
  rescan(): Promise<void>
  /** Launches Anki, then polls `anki_status` until it answers or the attempts run out. */
  launch(): Promise<void>
  requestInstall(): void
  requestProvision(): void
  /** "Agree": `grant_consent`, then the install action, then a re-check. */
  confirmConsent(): Promise<void>
  /** "Cancel": closes the dialog; nothing is recorded or downloaded. */
  cancelConsent(): void
  /** Stops polling; called automatically when the owning scope ends. */
  dispose(): void
}

export function useAnkiConnection(options: AnkiConnectionOptions = {}): AnkiConnection {
  const pollIntervalMs = options.pollIntervalMs ?? 1500
  const maxPollAttempts = options.maxPollAttempts ?? 10

  const state = ref<AnkiConnectionState>('checking')
  const status = ref<AnkiStatus | null>(null)
  const setup = ref<SetupStatus | null>(null)
  const activity = ref<AnkiActivity | null>(null)
  const pendingConsent = ref<ConsentAction | null>(null)
  const feedback = ref<AnkiFeedback | null>(null)

  const decks = computed(() => (status.value?.online ? status.value.decks : []))
  const models = computed(() => (status.value?.online ? status.value.models : []))
  const canInstallAnki = computed(
    () => state.value === 'anki_missing' && setup.value?.platform_supported === true,
  )
  const ankiVersion = computed(() => setup.value?.anki_version ?? '')

  /** Bumped by every resolution, so a slow, older check never overwrites a newer state. */
  let generation = 0
  let disposed = false
  const timers = new Set<ReturnType<typeof setTimeout>>()

  function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        timers.delete(timer)
        resolve()
      }, ms)
      timers.add(timer)
    })
  }

  async function liveStatus(): Promise<AnkiStatus | null> {
    try {
      return await ankiStatus()
    } catch (err) {
      console.warn('[anki] anki_status failed:', err)
      return null
    }
  }

  async function resolveState(): Promise<AnkiConnectionState> {
    const run = ++generation
    const live = await liveStatus()
    let next: AnkiConnectionState = 'online'
    let disk: SetupStatus | null = null
    let failure: string | null = null
    if (!live?.online) {
      try {
        disk = await getSetupStatus()
      } catch (err) {
        failure = errorMessage(err)
      }
      if (!disk) next = 'anki_closed'
      else if (!disk.anki_installed) next = 'anki_missing'
      else if (!disk.ankiconnect_installed) next = 'plugin_missing'
      else next = 'anki_closed'
    }
    if (disposed || run !== generation) return state.value
    status.value = live
    if (disk) setup.value = disk
    state.value = next
    if (failure) {
      feedback.value = {
        tone: 'error',
        key: 'anki.feedback.checkFailed',
        params: { error: failure },
      }
    }
    return next
  }

  async function rescan(): Promise<void> {
    if (activity.value) return
    activity.value = 'rescan'
    feedback.value = null
    state.value = 'checking'
    try {
      const next = await resolveState()
      if (!feedback.value) {
        feedback.value =
          next === 'online'
            ? { tone: 'success', key: 'anki.feedback.online' }
            : { tone: 'error', key: 'anki.feedback.stillOffline' }
      }
    } finally {
      activity.value = null
    }
  }

  async function launch(): Promise<void> {
    if (activity.value) return
    activity.value = 'launch'
    feedback.value = null
    try {
      try {
        await launchAnkiDesktop()
      } catch (err) {
        feedback.value = {
          tone: 'error',
          key: 'anki.feedback.launchFailed',
          params: { error: errorMessage(err) },
        }
        return
      }
      feedback.value = { tone: 'info', key: 'anki.feedback.launching' }
      for (let attempt = 0; attempt < maxPollAttempts; attempt++) {
        await sleep(pollIntervalMs)
        if (disposed) return
        const live = await liveStatus()
        if (disposed) return
        if (live?.online) {
          generation++
          status.value = live
          state.value = 'online'
          feedback.value = { tone: 'success', key: 'anki.feedback.online' }
          return
        }
      }
      const next = await resolveState()
      if (next !== 'online') feedback.value = { tone: 'error', key: 'anki.feedback.launchTimeout' }
    } finally {
      activity.value = null
    }
  }

  function requestInstall(): void {
    if (!activity.value && canInstallAnki.value) pendingConsent.value = 'anki_install'
  }

  function requestProvision(): void {
    if (!activity.value) pendingConsent.value = 'ankiconnect_provision'
  }

  function cancelConsent(): void {
    pendingConsent.value = null
  }

  async function confirmConsent(): Promise<void> {
    const action = pendingConsent.value
    if (!action || activity.value) return
    pendingConsent.value = null
    const installing = action === 'anki_install'
    activity.value = installing ? 'install' : 'provision'
    feedback.value = { tone: 'info', key: installing ? 'anki.installing' : 'anki.provisioning' }
    try {
      let consent: ConsentState
      try {
        consent = await grantConsent(action)
      } catch (err) {
        feedback.value = {
          tone: 'error',
          key: 'anki.feedback.consentFailed',
          params: { error: errorMessage(err) },
        }
        return
      }
      if (setup.value) setup.value = { ...setup.value, consent }
      options.onConsentChanged?.(consent)

      try {
        if (installing) await downloadAndInstallAnki()
        else await autoProvisionAnkiconnect()
        feedback.value = {
          tone: 'success',
          key: installing ? 'anki.feedback.installed' : 'anki.feedback.provisioned',
        }
      } catch (err) {
        feedback.value = {
          tone: 'error',
          key: 'anki.feedback.installFailed',
          params: { error: errorMessage(err) },
        }
      }
      state.value = 'checking'
      await resolveState()
    } finally {
      activity.value = null
    }
  }

  function dispose(): void {
    disposed = true
    for (const timer of timers) clearTimeout(timer)
    timers.clear()
  }

  if (getCurrentScope()) onScopeDispose(dispose)

  return {
    state,
    status,
    setup,
    activity,
    pendingConsent,
    feedback,
    decks,
    models,
    canInstallAnki,
    ankiVersion,
    check: resolveState,
    rescan,
    launch,
    requestInstall,
    requestProvision,
    confirmConsent,
    cancelConsent,
    dispose,
  }
}

/** Injection key of the main window's connection; tests use it to mount a single tab. */
export const ANKI_CONNECTION_KEY: InjectionKey<AnkiConnection> = Symbol('anki-connection')

/** Shares one connection between the header badge and the tabs below the calling component. */
export function provideAnkiConnection(connection: AnkiConnection): void {
  provide(ANKI_CONNECTION_KEY, connection)
}

/** The connection of the nearest {@link provideAnkiConnection} ancestor. */
export function useAnkiConnectionContext(): AnkiConnection {
  const connection = inject(ANKI_CONNECTION_KEY, null)
  if (!connection)
    throw new Error('useAnkiConnectionContext() needs a provideAnkiConnection() ancestor')
  return connection
}
