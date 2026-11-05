/**
 * The first-run setup assistant (the view is `views/SetupAssistant.vue`): from
 * a fresh install to a first capture, in six steps.
 *
 *   anki -> ankiconnect -> running -> note_type -> accessibility -> try
 *
 * Each step is ticked from what the app can check, so work that is already
 * done is skipped: the disk check and the live AnkiConnect probe of the shared
 * {@link AnkiConnection}, the saved note type against the note types Anki
 * lists, and `AXIsProcessTrusted` (macOS only; elsewhere the step does not
 * exist). The assistant starts on the first open step, and when the step it
 * shows gets done (Anki starts, the user flips the Accessibility switch) it
 * moves on by itself. While "running" is shown Anki is probed every two
 * seconds, while "accessibility" is shown the permission every second.
 *
 * "Set up everything" runs the whole chain from one click. Before anything is
 * downloaded it opens one consent dialog listing every download; only "Agree"
 * records the consents and installs ({@link AnkiConnection.installAll}). Then
 * it starts Anki, creates and saves the recommended note type, and asks macOS
 * for the permission. It stops at the first step that needs the user, and
 * that step says what to do. Nothing here runs without a click: showing the
 * assistant at startup only checks.
 *
 * AnkiConnect loads only when Anki starts. Anki that was already open while
 * AnkiConnect was installed does not answer until it is quit and started
 * again; the "running" step says so, louder after an install from here.
 *
 * Whether the simulated Cmd+C works right after the switch is flipped, without
 * a restart, is not something macOS promises. The "try" step therefore offers
 * a restart of Polywort when a capture fails while the permission is granted.
 */

import {
  computed,
  getCurrentScope,
  onScopeDispose,
  ref,
  watch,
  type ComputedRef,
  type Ref,
} from 'vue'

import {
  closeSetupAssistant,
  errorMessage,
  getAccessibilityStatus,
  onCaptureError,
  onTranslationPayload,
  requestAccessibility as requestAccessibilityCommand,
  restartApp,
  type AccessibilityStatus,
  type AppConfig,
  type ConsentAction,
  type UnlistenFn,
} from '../api'
import { selectRecommendedNoteType } from '../domain/fieldMapping'
import { isSupportedLang, type LanguageCode } from '../domain/language'
import { IS_MAC } from '../domain/shortcuts'
import type { MessageKey } from '../i18n'
import type { AnkiConnection } from './useAnkiConnection'
import type { ConfigStore } from './useConfig'

export type AssistantStep =
  'anki' | 'ankiconnect' | 'running' | 'note_type' | 'accessibility' | 'try'

/** Every step, in order; {@link SetupAssistant.steps} leaves out the ones this platform lacks. */
export const ASSISTANT_STEPS: readonly AssistantStep[] = [
  'anki',
  'ankiconnect',
  'running',
  'note_type',
  'accessibility',
  'try',
]

/** A line for the view to translate. */
export interface AssistantMessage {
  key: MessageKey
  params?: Record<string, string>
}

/** What the hotkey did on the "try" step. */
export type TryOutcome = { kind: 'success' } | { kind: 'error'; message: string }

/** A word in each capture language for the "try" step: one meaning, so every pair translates it. */
export const PRACTICE_WORDS: Record<LanguageCode, string> = {
  en: 'butterfly',
  de: 'Schmetterling',
  fr: 'papillon',
  es: 'mariposa',
  it: 'farfalla',
  'zh-CN': '蝴蝶',
}

/** The practice word in the language the default hotkey expects first. */
export function practiceWord(config: AppConfig): string {
  const lang = config.learning.languages[0]?.lang ?? config.translation.source_lang
  return isSupportedLang(lang) ? PRACTICE_WORDS[lang] : PRACTICE_WORDS.en
}

export interface SetupAssistantOptions {
  anki: AnkiConnection
  store: ConfigStore
  /** Delay between permission checks while the "accessibility" step is shown. */
  accessibilityPollMs?: number
  /** Delay between Anki probes while the "running" step is shown. */
  ankiPollMs?: number
  /** Assumed until the backend says whether the permission is needed. */
  isMac?: boolean
}

export interface SetupAssistant {
  /** The steps of this platform, in order. */
  steps: ComputedRef<AssistantStep[]>
  /** The step shown. */
  current: Ref<AssistantStep>
  done: ComputedRef<Record<AssistantStep, boolean>>
  /** The first checks are in, and {@link SetupAssistant.current} is the first open step. */
  ready: Ref<boolean>
  accessibility: Ref<AccessibilityStatus | null>
  /** "Set up everything" is running. */
  running: Ref<boolean>
  /** The downloads "Set up everything" needs, in install order. */
  downloads: ComputedRef<ConsentAction[]>
  /** The downloads its open consent dialog asks for; `null` while none is open. */
  consent: Ref<ConsentAction[] | null>
  canSetUpEverything: ComputedRef<boolean>
  /** AnkiConnect was installed from the assistant, so an open Anki must restart. */
  provisionedHere: Ref<boolean>
  /** The Accessibility switch went on while the assistant watched. */
  grantedHere: Ref<boolean>
  /** A request for the permission is running. */
  requesting: Ref<boolean>
  noteTypeError: Ref<AssistantMessage | null>
  accessibilityError: Ref<AssistantMessage | null>
  tryOutcome: Ref<TryOutcome | null>
  restarting: Ref<boolean>
  go(step: AssistantStep): void
  /** The next step in order, done or not. */
  skip(): void
  /** Opens the consent dialog when anything is to be downloaded, else runs the chain at once. */
  setUpEverything(): Promise<void>
  /** The consent dialog's "Agree": installs, then runs the rest of the chain. */
  agree(): Promise<void>
  cancel(): void
  /** "Agree" in the dialog of a single install step (`anki.pendingConsent`). */
  agreeSingle(): Promise<void>
  /** Creates (or reuses) the recommended note type and saves it as the note type. */
  setUpNoteType(): Promise<boolean>
  requestAccessibility(): Promise<void>
  restart(): Promise<void>
  /** Records "Done" (`finished`) or a dismissal, so startup stops opening the assistant. */
  close(finished: boolean): Promise<void>
  dispose(): void
}

export function useSetupAssistant(options: SetupAssistantOptions): SetupAssistant {
  const { anki, store } = options
  const accessibilityPollMs = options.accessibilityPollMs ?? 1000
  const ankiPollMs = options.ankiPollMs ?? 2000
  const isMac = options.isMac ?? IS_MAC

  const current = ref<AssistantStep>('anki')
  const ready = ref(false)
  const accessibility = ref<AccessibilityStatus | null>(null)
  const running = ref(false)
  const consent = ref<ConsentAction[] | null>(null)
  const provisionedHere = ref(false)
  const grantedHere = ref(false)
  const requesting = ref(false)
  const noteTypeError = ref<AssistantMessage | null>(null)
  const accessibilityError = ref<AssistantMessage | null>(null)
  const tryOutcome = ref<TryOutcome | null>(null)
  const restarting = ref(false)

  const online = computed(() => anki.state.value === 'online')
  const needsPermission = computed(() => accessibility.value?.required ?? isMac)
  const steps = computed(() =>
    ASSISTANT_STEPS.filter((step) => step !== 'accessibility' || needsPermission.value),
  )
  const done = computed<Record<AssistantStep, boolean>>(() => {
    const disk = anki.setup.value
    return {
      anki: online.value || disk?.anki_installed === true,
      ankiconnect: online.value || disk?.ankiconnect_installed === true,
      running: online.value,
      note_type: online.value && anki.models.value.includes(store.saved.value.anki.model_name),
      accessibility: accessibility.value?.trusted === true,
      try: tryOutcome.value?.kind === 'success',
    }
  })
  const downloads = computed(() => {
    const actions: ConsentAction[] = []
    if (!done.value.anki && anki.setup.value?.platform_supported) actions.push('anki_install')
    if (!done.value.ankiconnect) actions.push('ankiconnect_provision')
    return actions
  })
  const canSetUpEverything = computed(
    () =>
      ready.value &&
      !running.value &&
      anki.activity.value === null &&
      steps.value.some((step) => step !== 'try' && !done.value[step]),
  )

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

  /** Runs `tick` every `ms` while `active()`; one loop at a time. */
  function pollWhile(active: () => boolean, ms: number, tick: () => Promise<unknown>): void {
    let looping = false
    async function loop(): Promise<void> {
      looping = true
      try {
        while (!disposed && active()) {
          await sleep(ms)
          if (disposed || !active()) break
          await tick()
        }
      } finally {
        looping = false
      }
    }
    watch(active, (on) => {
      if (on && !looping) void loop()
    })
  }

  /** The first step not done yet; the last one when everything is. */
  function firstOpen(): AssistantStep {
    return steps.value.find((step) => !done.value[step]) ?? 'try'
  }

  /** The first open step after `from`, else the first open one at all. */
  function nextOpen(from: AssistantStep): AssistantStep {
    const after = steps.value.slice(steps.value.indexOf(from) + 1)
    return after.find((step) => !done.value[step]) ?? firstOpen()
  }

  async function refreshAccessibility(): Promise<void> {
    try {
      const next = await getAccessibilityStatus()
      if (disposed) return
      if (accessibility.value?.required && !accessibility.value.trusted && next.trusted) {
        grantedHere.value = true
      }
      accessibility.value = next
    } catch (err) {
      console.warn('[assistant] get_accessibility_status failed:', err)
      // Keeps the step, and its polling, on the platform that has it.
      accessibility.value ??= { required: isMac, trusted: false }
    }
  }

  function go(step: AssistantStep): void {
    if (steps.value.includes(step)) current.value = step
  }

  function skip(): void {
    const next = steps.value[steps.value.indexOf(current.value) + 1]
    if (next) current.value = next
  }

  async function setUpEverything(): Promise<void> {
    if (!canSetUpEverything.value) return
    const needed = downloads.value
    if (needed.length > 0) consent.value = needed
    else await runEverything([])
  }

  async function agree(): Promise<void> {
    const actions = consent.value
    consent.value = null
    if (actions) await runEverything(actions)
  }

  function cancel(): void {
    consent.value = null
  }

  /** Notes an AnkiConnect install, so the "running" step can ask for a restart of Anki. */
  function noteProvisioning(actions: readonly ConsentAction[]): void {
    if (actions.includes('ankiconnect_provision') && anki.setup.value?.ankiconnect_installed) {
      provisionedHere.value = true
    }
  }

  async function agreeSingle(): Promise<void> {
    const action = anki.pendingConsent.value
    if (!action) return
    await anki.confirmConsent()
    noteProvisioning([action])
  }

  async function runEverything(actions: readonly ConsentAction[]): Promise<void> {
    if (running.value) return
    running.value = true
    try {
      if (actions.length > 0) {
        current.value = actions[0] === 'anki_install' ? 'anki' : 'ankiconnect'
        const installed = await anki.installAll(actions)
        noteProvisioning(actions)
        if (!installed) {
          current.value = firstOpen()
          return
        }
      }
      if (!online.value) {
        current.value = 'running'
        await anki.launch()
        // Not answering yet: the step keeps probing and says what to do.
        if (!online.value) return
      }
      if (!done.value.note_type) {
        current.value = 'note_type'
        if (!(await setUpNoteType())) return
      }
      if (needsPermission.value && !done.value.accessibility) {
        current.value = 'accessibility'
        await requestAccessibility()
        return
      }
      current.value = firstOpen()
    } finally {
      running.value = false
    }
  }

  async function setUpNoteType(): Promise<boolean> {
    noteTypeError.value = null
    const outcome = await anki.createRecommendedNoteType()
    if (!outcome) return false
    if (!outcome.ok) {
      noteTypeError.value = { key: 'noteType.failed', params: { error: outcome.error } }
      return false
    }
    const { noteType } = outcome
    const error = await store.persist((config) => selectRecommendedNoteType(config.anki, noteType))
    if (error) {
      noteTypeError.value = { key: 'assistant.noteType.saveFailed', params: { error } }
      return false
    }
    return true
  }

  async function requestAccessibility(): Promise<void> {
    if (requesting.value) return
    requesting.value = true
    accessibilityError.value = null
    try {
      const next = await requestAccessibilityCommand()
      if (!disposed) accessibility.value = next
    } catch (err) {
      accessibilityError.value = {
        key: 'assistant.accessibility.failed',
        params: { error: errorMessage(err) },
      }
    } finally {
      requesting.value = false
    }
  }

  async function restart(): Promise<void> {
    restarting.value = true
    try {
      await restartApp()
    } catch (err) {
      restarting.value = false
      tryOutcome.value = { kind: 'error', message: errorMessage(err) }
    }
  }

  async function close(finished: boolean): Promise<void> {
    try {
      store.setSetupAssistant(await closeSetupAssistant(finished))
    } catch (err) {
      // The assistant only opens again at the next start.
      console.warn('[assistant] close_setup_assistant failed:', err)
    }
  }

  // The first step to show, once the first checks are in.
  watch(
    () => anki.state.value !== 'checking' && accessibility.value !== null,
    (checked) => {
      if (checked && !ready.value) {
        current.value = firstOpen()
        ready.value = true
      }
    },
    { immediate: true },
  )

  // The step shown got done (Anki answered, the switch went on): move on.
  watch(
    [current, () => done.value[current.value]] as const,
    ([step, isDone], [previousStep, wasDone]) => {
      if (ready.value && step === previousStep && isDone && !wasDone && step !== 'try') {
        current.value = nextOpen(step)
      }
    },
  )

  pollWhile(
    () =>
      current.value === 'running' &&
      !online.value &&
      !running.value &&
      anki.activity.value === null,
    ankiPollMs,
    async () => {
      if (anki.activity.value === null) await anki.check()
    },
  )
  pollWhile(
    () => current.value === 'accessibility' && needsPermission.value && !done.value.accessibility,
    accessibilityPollMs,
    refreshAccessibility,
  )

  const stops: UnlistenFn[] = []
  function listen(subscribe: Promise<UnlistenFn>): void {
    subscribe
      .then((stop) => (disposed ? stop() : stops.push(stop)))
      .catch((err) => console.warn('[assistant] could not listen for captures:', err))
  }
  listen(
    onTranslationPayload(() => {
      if (current.value === 'try') tryOutcome.value = { kind: 'success' }
    }),
  )
  listen(
    onCaptureError(({ message }) => {
      if (current.value === 'try') tryOutcome.value = { kind: 'error', message }
    }),
  )

  void refreshAccessibility()

  function dispose(): void {
    disposed = true
    for (const timer of timers) clearTimeout(timer)
    timers.clear()
    for (const stop of stops.splice(0)) stop()
  }

  if (getCurrentScope()) onScopeDispose(dispose)

  return {
    steps,
    current,
    done,
    ready,
    accessibility,
    running,
    downloads,
    consent,
    canSetUpEverything,
    provisionedHere,
    grantedHere,
    requesting,
    noteTypeError,
    accessibilityError,
    tryOutcome,
    restarting,
    go,
    skip,
    setUpEverything,
    agree,
    cancel,
    agreeSingle,
    setUpNoteType,
    requestAccessibility,
    restart,
    close,
    dispose,
  }
}
