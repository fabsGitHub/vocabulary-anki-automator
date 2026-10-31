/**
 * XP outside review sessions: vocab-learn's `progress` module
 * (apps/learn/src-tauri/src/progress.rs). The amounts and reasons are the
 * core's; what this adds is who may earn them.
 *
 * - **Exercises** earn `XP_EXERCISE_CORRECT` for a correct answer about a
 *   learned card. A card that is not learned has no exercises, so a result
 *   for it is refused rather than paid.
 * - **Lessons** earn `XP_LESSON_COMPLETE` once per lesson per local day. The
 *   day of each lesson's last paid completion is kept in `app_state` under
 *   {@link lessonKey}; only lessons that exist can be completed.
 */

import { LearnError } from './errors'
import { XP_EXERCISE_CORRECT, XP_LESSON_COMPLETE, awardXp, reason } from './gamification'
import { lessonsFromStore } from './lessons'
import type { Store } from './store'
import { type Clock, today as clockToday } from './time'

/** Cards per lesson; lesson ids name their first card, so both builds must use the same number. */
export const CARDS_PER_LESSON = 5

/** Prefix of the `app_state` keys that remember lesson completions. */
export const LESSON_COMPLETED_PREFIX = 'learn.lesson_completed.'

/** The `app_state` key holding the local date on which `lessonId` last earned XP. */
export function lessonKey(lessonId: string): string {
  return `${LESSON_COMPLETED_PREFIX}${lessonId}`
}

/** Records the result of an exercise about `cardId`; returns the XP awarded. */
export async function recordExercise(
  store: Store,
  cardId: number,
  correct: boolean,
  clock: Clock,
): Promise<number> {
  if ((await store.getSrsState(cardId)) === null) throw LearnError.notLearned(cardId)
  if (!correct) return 0
  await awardXp(store, XP_EXERCISE_CORRECT, reason.EXERCISE, clock)
  return XP_EXERCISE_CORRECT
}

/** Records a finished lesson; returns the XP awarded, 0 when it already earned it today. */
export async function completeLesson(
  store: Store,
  lessonId: string,
  clock: Clock,
): Promise<number> {
  const lessons = await lessonsFromStore(store, CARDS_PER_LESSON)
  if (!lessons.some((lesson) => lesson.id === lessonId)) throw LearnError.noSuchLesson(lessonId)
  const key = lessonKey(lessonId)
  const today = clockToday(clock)
  return store.withTransaction(async (store) => {
    if ((await store.getState(key)) === today) return 0
    await awardXp(store, XP_LESSON_COMPLETE, reason.LESSON, clock)
    await store.setState(key, today)
    return XP_LESSON_COMPLETE
  })
}
