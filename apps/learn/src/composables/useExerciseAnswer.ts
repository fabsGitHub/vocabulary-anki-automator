/**
 * Answering one exercise (Milestone M2): the answer is checked by the
 * adapter (`checkExercise`, the core's `check_answer`: equal tiles are
 * interchangeable, a reused tile is wrong), then the result is recorded with
 * `recordExerciseResult` for the card being practised, which awards the XP.
 *
 * One answer per exercise: the components show the next exercise instead of
 * a second try. A failed check can be repeated; a failed record still shows
 * whether the answer was right.
 */

import { ref } from 'vue'

import { usePlatform } from '../context'
import { errorMessage } from '../domain/errors'
import type { Exercise, ExerciseAnswer } from '../platform/types'

export type AnswerStatus = 'idle' | 'checking' | 'answered'

export function useExerciseAnswer(
  exercise: () => Exercise,
  onAnswered: (correct: boolean) => void,
) {
  const platform = usePlatform()
  const status = ref<AnswerStatus>('idle')
  const correct = ref<boolean | null>(null)
  const xp = ref(0)
  const checkError = ref<string | null>(null)
  const recordError = ref<string | null>(null)

  async function submit(answer: ExerciseAnswer): Promise<void> {
    if (status.value !== 'idle') return
    const current = exercise()
    status.value = 'checking'
    checkError.value = null
    let isCorrect: boolean
    try {
      isCorrect = await platform.checkExercise(current, answer)
    } catch (err) {
      checkError.value = errorMessage(err)
      status.value = 'idle'
      return
    }
    correct.value = isCorrect
    status.value = 'answered'
    onAnswered(isCorrect)
    try {
      xp.value = await platform.recordExerciseResult(current.card_id, current.kind, isCorrect)
    } catch (err) {
      recordError.value = errorMessage(err)
    }
  }

  return { status, correct, xp, checkError, recordError, submit }
}

export type ExerciseAnswerState = ReturnType<typeof useExerciseAnswer>
