/**
 * The desktop `PlatformAdapter`: every method is one `invoke` of the
 * vocab-learn command of docs/architecture.md § 10, with the arguments in
 * the camelCase that Tauri maps onto the Rust parameter names.
 *
 * This is the only module of Learn that imports `@tauri-apps/api`. Command
 * names and argument names exist here and nowhere else, so a change to the
 * IPC surface is a change to this file (and to types.ts, which lists them).
 */

import { invoke, type InvokeArgs } from '@tauri-apps/api/core'

import { errorMessage } from '../domain/errors'
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
} from './types'

/**
 * Runs `command`. A rejection becomes the human-readable string the contract
 * promises: commands reject with their `Err` string already, but a panic or a
 * typed error arrives as an object.
 */
async function call<T>(command: string, args?: InvokeArgs): Promise<T> {
  try {
    return await (args === undefined ? invoke<T>(command) : invoke<T>(command, args))
  } catch (err) {
    throw errorMessage(err)
  }
}

export class TauriAdapter implements PlatformAdapter {
  /** The desktop backend has TTS (Google TTS through vocab-core) and AnkiConnect. */
  readonly capabilities: PlatformCapabilities = { kind: 'tauri', tts: true, anki: true }

  getDailyPromptState(): Promise<DailyPromptState> {
    return call('get_daily_prompt_state')
  }

  submitDailyQuota(newCount: number): Promise<DailySession> {
    return call('submit_daily_quota', { newCount })
  }

  dismissDailyPrompt(): Promise<DailySession> {
    return call('dismiss_daily_prompt')
  }

  async reopenDailyPrompt(): Promise<void> {
    await call<null>('reopen_daily_prompt')
  }

  getDailyQueue(): Promise<QueueItem[]> {
    return call('get_daily_queue')
  }

  startNewItem(hanzi: string): Promise<number> {
    return call('start_new_item', { hanzi })
  }

  startNewCapture(cardId: number): Promise<number> {
    return call('start_new_capture', { cardId })
  }

  getCard(cardId: number): Promise<StoredCapture> {
    return call('get_card', { cardId })
  }

  previewRatings(cardId: number): Promise<RatingPreview[]> {
    return call('preview_ratings', { cardId })
  }

  submitReviewRating(cardId: number, rating: Rating): Promise<ReviewResult> {
    return call('submit_review_rating', { cardId, rating })
  }

  listLearnedCards(): Promise<LearnedCard[]> {
    return call('list_learned_cards')
  }

  nextExercise(cardId: number, kind: ExerciseKind, seed: number): Promise<Exercise> {
    return call('next_exercise', { cardId, kind, seed })
  }

  checkExercise(exercise: Exercise, answer: ExerciseAnswer): Promise<boolean> {
    return call('check_exercise', { exercise, answer })
  }

  recordExerciseResult(cardId: number, kind: ExerciseKind, correct: boolean): Promise<number> {
    return call('record_exercise_result', { cardId, kind, correct })
  }

  listLessons(): Promise<Lesson[]> {
    return call('list_lessons')
  }

  completeLesson(lessonId: string): Promise<number> {
    return call('complete_lesson', { lessonId })
  }

  getStats(): Promise<StatsSummary> {
    return call('get_stats')
  }

  getCardAudio(cardId: number): Promise<string | null> {
    return call('get_card_audio', { cardId })
  }

  synthesizeAudio(text: string, lang: string): Promise<string | null> {
    return call('synthesize_audio', { text, lang })
  }

  getSettings(): Promise<LearnSettings> {
    return call('get_settings')
  }

  async saveSettings(settings: LearnSettings): Promise<void> {
    await call<null>('save_settings', { settings })
  }

  ankiExport(): Promise<AnkiSyncReport> {
    return call('anki_export')
  }

  ankiImport(deck: string): Promise<AnkiSyncReport> {
    return call('anki_import', { deck })
  }
}
