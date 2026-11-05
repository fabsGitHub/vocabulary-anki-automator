/**
 * The daily onboarding prompt (Milestone L2) as the main window sees it.
 *
 * On mount the window asks `getDailyPromptState`; when `needs_prompt` is set,
 * "how many new words today?" is shown before the normal UI renders,
 * because today's queue depends on the answer. The rules are the core's
 * (docs/architecture.md § 7.1), the UI only routes the answer:
 *
 * - **Start** submits the quota; **Not today** (also Esc) dismisses, which
 *   the core records as "reviews only today" (quota 0).
 * - The menu action reopens the prompt: `reopenDailyPrompt` clears the
 *   "asked today" flag, and a later answer replaces the quota while the
 *   day's progress is kept. Every answered quota bumps `round`, which tells
 *   the review session to rebuild its queue.
 * - A window left open past midnight asks again for the new day when it
 *   regains focus: the day is compared with the injected clock, not the
 *   system time, so tests can move it.
 */

import { computed, ref } from 'vue'

import type { Clock } from '../context'
import { errorMessage } from '../domain/errors'
import type { DailySession, IsoDate, PlatformAdapter } from '../platform/types'

/** The prompt's suggestion when today has no goal yet. */
export const DEFAULT_NEW_COUNT = 5
/** The range of new words a day the prompt accepts. */
export const MIN_NEW_COUNT = 0
export const MAX_NEW_COUNT = 50
/** The prompt's quick-pick buttons. */
export const QUICK_PICKS: readonly number[] = [3, 5, 10, 20]

export type PromptPhase = 'loading' | 'error' | 'ready'

/** `ms` as a local calendar date, `YYYY-MM-DD` (the core's `time::format_date`). */
export function localDate(ms: number): IsoDate {
  const date = new Date(ms)
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
}

export function useDailyPrompt(platform: PlatformAdapter, clock: Clock) {
  const phase = ref<PromptPhase>('loading')
  const loadError = ref<string | null>(null)
  /** The prompt is on screen. */
  const open = ref(false)
  /** Today was answered once; the normal UI may render. */
  const started = ref(false)
  const busy = ref(false)
  const actionError = ref<string | null>(null)
  const reopenError = ref<string | null>(null)
  const session = ref<DailySession | null>(null)
  const today = ref<IsoDate | null>(null)
  /** Bumped whenever an answer may have changed today's queue. */
  const round = ref(0)

  /** What the prompt suggests: today's goal when it has new words, else the default. */
  const suggestedCount = computed(() => session.value?.requested_new_count || DEFAULT_NEW_COUNT)

  async function load(): Promise<void> {
    phase.value = 'loading'
    loadError.value = null
    try {
      const state = await platform.getDailyPromptState()
      today.value = state.today
      session.value = state.session
      actionError.value = null
      open.value = state.needs_prompt
      if (!state.needs_prompt) started.value = true
      phase.value = 'ready'
    } catch (err) {
      loadError.value = errorMessage(err)
      phase.value = 'error'
    }
  }

  async function answer(run: () => Promise<DailySession>, changesQueue: boolean): Promise<void> {
    if (busy.value) return
    busy.value = true
    actionError.value = null
    try {
      session.value = await run()
      open.value = false
      // The first answer of the day renders the review for the first time,
      // which loads the queue anyway.
      if (started.value && changesQueue) round.value += 1
      started.value = true
    } catch (err) {
      actionError.value = errorMessage(err)
    } finally {
      busy.value = false
    }
  }

  /** "Start": today's quota of new words. */
  function submit(newCount: number): Promise<void> {
    return answer(() => platform.submitDailyQuota(newCount), true)
  }

  /**
   * "Not today": reviews only. A reopened prompt dismissed again keeps
   * today's session as it was, so only a day without a session yet (a new
   * day in a window left open) has a new queue.
   */
  function dismiss(): Promise<void> {
    return answer(() => platform.dismissDailyPrompt(), session.value === null)
  }

  /** The menu action "Change today's new-word goal". */
  async function reopen(): Promise<void> {
    if (busy.value || open.value) return
    busy.value = true
    reopenError.value = null
    try {
      await platform.reopenDailyPrompt()
      actionError.value = null
      open.value = true
    } catch (err) {
      reopenError.value = errorMessage(err)
    } finally {
      busy.value = false
    }
  }

  /**
   * Asks again once the local date moved on since the prompt was last
   * checked. The queue is rebuilt for the new day at once, and again when
   * the new day's quota is submitted.
   */
  async function checkNewDay(): Promise<void> {
    if (phase.value !== 'ready' || busy.value || open.value || today.value === null) return
    if (localDate(clock.now()) === today.value) return
    await load()
    if (phase.value === 'ready') round.value += 1
  }

  return {
    phase,
    loadError,
    open,
    started,
    busy,
    actionError,
    reopenError,
    session,
    today,
    round,
    suggestedCount,
    load,
    submit,
    dismiss,
    reopen,
    checkNewDay,
  }
}

export type DailyPrompt = ReturnType<typeof useDailyPrompt>
