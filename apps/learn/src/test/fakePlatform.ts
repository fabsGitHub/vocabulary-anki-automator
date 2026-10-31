/**
 * An in-memory `PlatformAdapter` for component tests, plus the mounting
 * helper that injects it.
 *
 * Every method is a `vi.fn` with a plausible default, so a test overrides
 * only the calls it is about (`platform.getDailyQueue.mockResolvedValue(...)`)
 * and asserts on the exact arguments the components pass. Components get the
 * fake, a fixed clock and counting seeds through the same `learnContext`
 * plugin the app uses, so nothing in a test reads the real clock or a random
 * number.
 */

import { mount, type ComponentMountingOptions } from '@vue/test-utils'
import { vi, type Mock } from 'vitest'
import type { Component } from 'vue'

import { learnContext, type Clock, type SeedSource } from '../context'
import { i18n } from '../i18n'
import type { ExerciseAnswer, PlatformAdapter, PlatformCapabilities } from '../platform/types'
import * as fx from './fixtures'

type Methods = {
  [
    K in keyof PlatformAdapter as PlatformAdapter[K] extends (...args: never[]) => unknown
      ? K
      : never
  ]: PlatformAdapter[K]
}

export type FakePlatform = { capabilities: PlatformCapabilities } & {
  [K in keyof Methods]: Mock<Methods[K]>
}

/** The core's `check_answer`, reduced to what the fixtures need. */
function checkAnswer(
  exercise: Parameters<PlatformAdapter['checkExercise']>[0],
  answer: ExerciseAnswer,
): boolean {
  if (exercise.kind === 'multiple_choice' || exercise.kind === 'listening_choice') {
    return answer.kind === 'choice' && answer.index === exercise.correct_index
  }
  return answer.kind === 'order' && answer.tile_ids.join() === exercise.solution.join()
}

export function fakePlatform(capabilities: Partial<PlatformCapabilities> = {}): FakePlatform {
  return {
    capabilities: { kind: 'tauri', tts: true, anki: true, ...capabilities },
    getDailyPromptState: vi.fn<Methods['getDailyPromptState']>(async () => fx.promptState()),
    submitDailyQuota: vi.fn<Methods['submitDailyQuota']>(async (newCount) =>
      fx.session({ requested_new_count: newCount }),
    ),
    dismissDailyPrompt: vi.fn<Methods['dismissDailyPrompt']>(async () =>
      fx.session({ requested_new_count: 0 }),
    ),
    reopenDailyPrompt: vi.fn<Methods['reopenDailyPrompt']>(async () => undefined),
    getDailyQueue: vi.fn<Methods['getDailyQueue']>(async () => []),
    startNewItem: vi.fn<Methods['startNewItem']>(async () => 100),
    getCard: vi.fn<Methods['getCard']>(async (cardId) => fx.capture({ id: cardId })),
    previewRatings: vi.fn<Methods['previewRatings']>(async () => fx.previews()),
    submitReviewRating: vi.fn<Methods['submitReviewRating']>(async () => fx.reviewResult()),
    listLearnedCards: vi.fn<Methods['listLearnedCards']>(async () => []),
    nextExercise: vi.fn<Methods['nextExercise']>(async () => fx.multipleChoice()),
    checkExercise: vi.fn<Methods['checkExercise']>(async (exercise, answer) =>
      checkAnswer(exercise, answer),
    ),
    recordExerciseResult: vi.fn<Methods['recordExerciseResult']>(async (_card, _kind, correct) =>
      correct ? 5 : 0,
    ),
    listLessons: vi.fn<Methods['listLessons']>(async () => []),
    completeLesson: vi.fn<Methods['completeLesson']>(async () => 20),
    getStats: vi.fn<Methods['getStats']>(async () => fx.stats()),
    getCardAudio: vi.fn<Methods['getCardAudio']>(async () => null),
    synthesizeAudio: vi.fn<Methods['synthesizeAudio']>(async () => 'U1lOVEg='),
    getSettings: vi.fn<Methods['getSettings']>(async () => fx.settings()),
    saveSettings: vi.fn<Methods['saveSettings']>(async () => undefined),
    ankiExport: vi.fn<Methods['ankiExport']>(async () => fx.report()),
    ankiImport: vi.fn<Methods['ankiImport']>(async () => fx.report()),
  }
}

/** A clock that stands still until a test moves it. */
export function fixedClock(start = fx.NOW): Clock & { set(ms: number): void } {
  let now = start
  return {
    now: () => now,
    set: (ms) => {
      now = ms
    },
  }
}

/** Seeds 1, 2, 3, ... */
export function countingSeeds(): SeedSource {
  let next = 0
  return { next: () => ++next }
}

export interface Harness {
  platform: FakePlatform
  clock?: Clock
  seeds?: SeedSource
}

/** Mounts `component` with i18n and the Learn context of `harness`. */
export function mountWith<C extends Component>(
  component: C,
  harness: Harness,
  options: ComponentMountingOptions<C> = {},
) {
  const context = learnContext({
    platform: harness.platform,
    clock: harness.clock ?? fixedClock(),
    seeds: harness.seeds ?? countingSeeds(),
  })
  return mount(component, {
    ...options,
    global: { ...options.global, plugins: [i18n, context, ...(options.global?.plugins ?? [])] },
  })
}

/** A promise with its resolve/reject exposed, to hold a fake call open. */
export function deferred<T>(): {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason: unknown) => void
} {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/** A keydown on `window`, as a key press anywhere in the window delivers it. */
export function pressKey(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init })
  window.dispatchEvent(event)
  return event
}
