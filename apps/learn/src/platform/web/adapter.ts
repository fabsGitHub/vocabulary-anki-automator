/**
 * The web build's {@link PlatformAdapter}: every vocab-learn command of
 * docs/architecture.md § 10, run in the browser over IndexedDB with the
 * TypeScript port of the core.
 *
 * Each method is what the desktop command does, composed the same way from
 * the same core functions (`./session.ts`, `./progress.ts` and
 * `./settings.ts` mirror vocab-learn's modules of the same names):
 *
 * - **One storage transaction and one clock reading per command**, as the
 *   desktop app holds its store lock and reads its clock once per command,
 *   so a review, its XP and its session progress land on the same local
 *   date together or not at all.
 * - **Failures reject with the desktop build's sentence**, a plain string,
 *   as the Tauri adapter's do.
 * - **The curriculum is seeded lazily**: the first command that needs it
 *   (the queue, a new word) fetches the bundled asset and seeds the store
 *   once per fingerprint; a failure is reported by that command and retried
 *   by the next one, and every other command keeps working.
 * - **What a browser cannot do is refused**: text-to-speech and AnkiConnect
 *   reject with "not available in the web version", and
 *   {@link WebAdapter.capabilities} says so. Stored recordings still play.
 */

import type {
  AnkiSyncReport,
  DailyPromptState,
  DailySession,
  Exercise,
  ExerciseAnswer,
  ExerciseKind,
  LearnSettings,
  LearnedCard,
  Lesson,
  PlatformAdapter,
  PlatformCapabilities,
  QueueItem,
  Rating,
  RatingPreview,
  ReviewResult,
  StatsSummary,
  StoredCapture,
} from '../types'
import { loadBundledCurriculum } from './curriculum'
import {
  dailyPromptState,
  dailyPromptStateToJson,
  dismissDailyPrompt,
  reopenDailyPrompt,
  submitDailyQuota,
} from './daily'
import { LearnError, StoreError, errorMessage } from './errors'
import { EXERCISE_KINDS, checkAnswer, generateExercise } from './exercises'
import { statsSummary } from './gamification'
import { encodeBase64, type CurriculumItem, storedCaptureToJson } from './model'
import { LearnedVocabularyPool } from './pool'
import { CARDS_PER_LESSON, completeLesson, recordExercise } from './progress'
import { introduceNewItem, queueItemToJson } from './queue'
import { lessonsFromStore } from './lessons'
import { SrsScheduler, ratingPreviewToJson, reviewRating } from './scheduler'
import { dailyQueue, rate, reviewResultToJson, trackCompletion } from './session'
import {
  loadSettings,
  normalizeSettings,
  parseSettings,
  saveSettings,
  validateSettings,
} from './settings'
import type { Storage } from './storage'
import { Store, dailySessionToJson } from './store'
import { trim } from './text'
import { type Clock, snapshot, systemClock } from './time'

/** What a {@link WebAdapter} runs on; tests replace every part. */
export interface WebAdapterOptions {
  /** The store, or the promise of one (opening IndexedDB is asynchronous). */
  storage: Storage | Promise<Storage>
  clock?: Clock
  /** The curriculum to seed; the bundled asset by default. */
  loadCurriculum?: () => Promise<CurriculumItem[]>
  scheduler?: SrsScheduler
}

const U32_MAX = 4_294_967_295

/** A command's body: it gets the store inside one transaction and one clock reading. */
type Body<T> = (store: Store, clock: Clock) => Promise<T>

/** {@link PlatformAdapter} over the ported core. */
export class WebAdapter implements PlatformAdapter {
  /** No TTS and no AnkiConnect in a browser. */
  readonly capabilities: PlatformCapabilities = { kind: 'web', tts: false, anki: false }

  private readonly storage: Promise<Storage>
  private readonly clock: Clock
  private readonly loadCurriculum: () => Promise<CurriculumItem[]>
  private readonly scheduler: SrsScheduler
  private seeding: Promise<void> | null = null

  constructor(options: WebAdapterOptions) {
    this.storage = Promise.resolve(options.storage)
    // A rejection is reported by every command; nobody else has to handle it.
    this.storage.catch(() => undefined)
    this.clock = options.clock ?? systemClock
    this.loadCurriculum = options.loadCurriculum ?? (() => loadBundledCurriculum())
    this.scheduler = options.scheduler ?? new SrsScheduler()
  }

  getDailyPromptState(): Promise<DailyPromptState> {
    return this.command(
      () => async (store, clock) => dailyPromptStateToJson(await dailyPromptState(store, clock)),
    )
  }

  submitDailyQuota(newCount: number): Promise<DailySession> {
    return this.command(() => {
      const count = u32(newCount, 'newCount', 'submit_daily_quota')
      return async (store, clock) => {
        const [session] = await trackCompletion(store, clock, (store) =>
          submitDailyQuota(store, count, clock),
        )
        return dailySessionToJson(session)
      }
    })
  }

  dismissDailyPrompt(): Promise<DailySession> {
    return this.command(() => async (store, clock) => {
      const [session] = await trackCompletion(store, clock, (store) =>
        dismissDailyPrompt(store, clock),
      )
      return dailySessionToJson(session)
    })
  }

  reopenDailyPrompt(): Promise<void> {
    return this.command(() => (store, clock) => reopenDailyPrompt(store, clock))
  }

  getDailyQueue(): Promise<QueueItem[]> {
    return this.command(
      () => async (store, clock) => (await dailyQueue(store, clock)).map(queueItemToJson),
      { curriculum: true },
    )
  }

  startNewItem(hanzi: string): Promise<number> {
    return this.command(
      () => {
        const word = trim(text(hanzi, 'hanzi', 'start_new_item'))
        return async (store, clock) => {
          const { meaning_lang: meaningLang } = await loadSettings(store)
          try {
            return await introduceNewItem(store, this.scheduler, word, meaningLang, clock)
          } catch (err) {
            if (err instanceof StoreError && err.kind === 'not_found') {
              throw LearnError.notInCurriculum(word)
            }
            throw err
          }
        }
      },
      { curriculum: true },
    )
  }

  getCard(cardId: number): Promise<StoredCapture> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'get_card')
      return async (store) => {
        const capture = await store.getCapture(id)
        if (capture === null) throw LearnError.noSuchCard(id)
        return storedCaptureToJson(capture)
      }
    })
  }

  previewRatings(cardId: number): Promise<RatingPreview[]> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'preview_ratings')
      return async (store, clock) => {
        const state = await store.getSrsState(id)
        if (state === null) throw LearnError.notScheduled(id)
        return this.scheduler.preview(state, clock.now()).map(ratingPreviewToJson)
      }
    })
  }

  submitReviewRating(cardId: number, rating: Rating): Promise<ReviewResult> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'submit_review_rating')
      const given = argument('rating', 'submit_review_rating', () => reviewRating(rating))
      return async (store, clock) => {
        if ((await store.getSrsState(id)) === null) throw LearnError.notScheduled(id)
        return reviewResultToJson(await rate(store, this.scheduler, id, given, clock))
      }
    })
  }

  listLearnedCards(): Promise<LearnedCard[]> {
    return this.command(
      () => async (store) =>
        (await LearnedVocabularyPool.load(store)).entries().map((entry) => ({ ...entry })),
    )
  }

  nextExercise(cardId: number, kind: ExerciseKind, seed: number): Promise<Exercise> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'next_exercise')
      const exerciseKind = exerciseKindArg(kind, 'next_exercise')
      const draw = u32(seed, 'seed', 'next_exercise')
      return async (store) =>
        generateExercise(await LearnedVocabularyPool.load(store), id, exerciseKind, draw)
    })
  }

  /** Needs no store: the exercise carries its solution. */
  checkExercise(exercise: Exercise, answer: ExerciseAnswer): Promise<boolean> {
    return this.call(async () =>
      checkAnswer(exerciseArg(exercise), answerArg(answer, 'check_exercise')),
    )
  }

  recordExerciseResult(cardId: number, kind: ExerciseKind, correct: boolean): Promise<number> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'record_exercise_result')
      exerciseKindArg(kind, 'record_exercise_result')
      const right = bool(correct, 'correct', 'record_exercise_result')
      return (store, clock) => recordExercise(store, id, right, clock)
    })
  }

  listLessons(): Promise<Lesson[]> {
    return this.command(() => (store) => lessonsFromStore(store, CARDS_PER_LESSON))
  }

  completeLesson(lessonId: string): Promise<number> {
    return this.command(() => {
      const id = text(lessonId, 'lessonId', 'complete_lesson')
      return (store, clock) => completeLesson(store, id, clock)
    })
  }

  getStats(): Promise<StatsSummary> {
    return this.command(() => (store, clock) => statsSummary(store, clock))
  }

  /** The card's stored recording; the web build has no TTS to make a missing one. */
  getCardAudio(cardId: number): Promise<string | null> {
    return this.command(() => {
      const id = i64(cardId, 'cardId', 'get_card_audio')
      return async (store) => {
        const capture = await store.getCapture(id)
        if (capture === null) throw LearnError.noSuchCard(id)
        const audio = capture.card.audio_front
        return audio !== null && audio.length > 0 ? encodeBase64(audio) : null
      }
    })
  }

  synthesizeAudio(_text: string, _lang: string): Promise<string | null> {
    return this.call(async () => {
      throw LearnError.notAvailable('Text-to-speech')
    })
  }

  getSettings(): Promise<LearnSettings> {
    return this.command(() => (store) => loadSettings(store))
  }

  /** Normalized, then refused with every problem or stored. */
  saveSettings(settings: LearnSettings): Promise<void> {
    return this.command(() => {
      const parsed = argument('settings', 'save_settings', () => parseSettings(settings))
      const normalized = normalizeSettings(parsed)
      const problems = validateSettings(normalized)
      if (problems.length > 0) throw LearnError.invalidSettings(problems)
      return (store) => saveSettings(store, normalized)
    })
  }

  ankiExport(): Promise<AnkiSyncReport> {
    return this.call(async () => {
      throw LearnError.notAvailable('Anki export')
    })
  }

  ankiImport(_deck: string): Promise<AnkiSyncReport> {
    return this.call(async () => {
      throw LearnError.notAvailable('Anki import')
    })
  }

  /**
   * Runs a command: `prepare` checks the arguments, as Tauri deserializes
   * them before a command runs, and returns the body, which then runs in one
   * storage transaction with one clock reading, after the curriculum is
   * seeded when `needs.curriculum`.
   */
  private command<T>(prepare: () => Body<T>, needs: { curriculum?: boolean } = {}): Promise<T> {
    return this.call(async () => {
      const body = prepare()
      const clock = snapshot(this.clock)
      const storage = await this.openStorage()
      if (needs.curriculum) await this.ensureCurriculum(storage)
      return storage.transaction((tx) => body(new Store(tx), clock))
    })
  }

  /** Runs `body`, turning any failure into the sentence the contract promises. */
  private async call<T>(body: () => Promise<T>): Promise<T> {
    try {
      return await body()
    } catch (err) {
      throw LearnError.from(err).message
    }
  }

  private async openStorage(): Promise<Storage> {
    try {
      return await this.storage
    } catch (err) {
      throw LearnError.storeUnavailable(errorMessage(err))
    }
  }

  /** Seeds the curriculum once; a failure is reported now and retried next time. */
  private ensureCurriculum(storage: Storage): Promise<void> {
    if (this.seeding === null) {
      this.seeding = (async () => {
        try {
          const items = await this.loadCurriculum()
          await storage.transaction((tx) => new Store(tx).ensureCurriculumSeeded(items))
        } catch (err) {
          this.seeding = null
          throw LearnError.curriculumUnavailable(errorMessage(err))
        }
      })()
    }
    return this.seeding
  }
}

/** Runs `parse` for the command argument `name`, reporting a failure as Tauri does. */
function argument<T>(name: string, command: string, parse: () => T): T {
  try {
    return parse()
  } catch (err) {
    throw LearnError.invalidArgs(name, command, errorMessage(err))
  }
}

function i64(value: unknown, name: string, command: string): number {
  return argument(name, command, () => {
    if (typeof value !== 'number' || !Number.isSafeInteger(value)) {
      throw new Error(`invalid type: ${describe(value)}, expected i64`)
    }
    return value
  })
}

function u32(value: unknown, name: string, command: string): number {
  return argument(name, command, () => {
    if (typeof value !== 'number' || !Number.isInteger(value)) {
      throw new Error(`invalid type: ${describe(value)}, expected u32`)
    }
    if (value < 0 || value > U32_MAX) {
      throw new Error(`invalid value: integer \`${value}\`, expected u32`)
    }
    return value
  })
}

function bool(value: unknown, name: string, command: string): boolean {
  return argument(name, command, () => {
    if (typeof value !== 'boolean')
      throw new Error(`invalid type: ${describe(value)}, expected a boolean`)
    return value
  })
}

function text(value: unknown, name: string, command: string): string {
  return argument(name, command, () => {
    if (typeof value !== 'string') {
      throw new Error(`invalid type: ${describe(value)}, expected a string`)
    }
    return value
  })
}

function exerciseKindArg(value: unknown, command: string): ExerciseKind {
  return argument('kind', command, () => variant(value, EXERCISE_KINDS))
}

/** An exercise's shape, as far as checking an answer reads it. */
function exerciseArg(value: unknown): Exercise {
  return argument('exercise', 'check_exercise', () => {
    const exercise = map(value)
    variant(exercise.kind, EXERCISE_KINDS)
    const choice = exercise.kind === 'multiple_choice' || exercise.kind === 'listening_choice'
    if (choice ? !Array.isArray(exercise.options) : !Array.isArray(exercise.tiles)) {
      throw new Error(`missing field \`${choice ? 'options' : 'tiles'}\``)
    }
    return value as Exercise
  })
}

function answerArg(value: unknown, command: string): ExerciseAnswer {
  return argument('answer', command, () => {
    const answer = map(value)
    variant(answer.kind, ['choice', 'order'])
    if (
      answer.kind === 'choice' ? typeof answer.index !== 'number' : !Array.isArray(answer.tile_ids)
    ) {
      throw new Error(`missing field \`${answer.kind === 'choice' ? 'index' : 'tile_ids'}\``)
    }
    return value as ExerciseAnswer
  })
}

function map(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`invalid type: ${describe(value)}, expected a map`)
  }
  return value as Record<string, unknown>
}

function variant<T extends string>(value: unknown, variants: readonly T[]): T {
  if (!variants.includes(value as T)) {
    const expected = variants.map((each) => `\`${each}\``).join(', ')
    throw new Error(`unknown variant \`${String(value)}\`, expected one of ${expected}`)
  }
  return value as T
}

/** How serde names the type of a JSON value it did not expect. */
function describe(value: unknown): string {
  if (typeof value === 'number') {
    return Number.isInteger(value) ? `integer \`${value}\`` : `floating point \`${value}\``
  }
  if (typeof value === 'string') return `string ${JSON.stringify(value)}`
  if (typeof value === 'boolean') return `boolean \`${value}\``
  if (value === null || value === undefined) return 'null'
  return Array.isArray(value) ? 'sequence' : 'map'
}
