<script setup lang="ts">
/**
 * The result line under an exercise. The live region is always in the
 * document, so screen readers announce the verdict when it appears; what it
 * reveals (the answer, the sentence) comes from the exercise itself.
 */
import { useI18n } from 'vue-i18n'

import type { ExerciseAnswerState } from '../../composables/useExerciseAnswer'

defineProps<{
  answer: ExerciseAnswerState
  /** Shown after a wrong answer: `Answer: …`. */
  solution?: string | null
  /** Shown after any answer: `Sentence: …`. */
  sentence?: string | null
  /** Language of `solution` and `sentence`. */
  lang?: string
}>()

const { t } = useI18n()
</script>

<template>
  <div class="exercise-feedback" role="status" data-test="feedback">
    <p v-if="answer.status.value === 'checking'" class="input-hint">
      {{ t('exercise.checking') }}
    </p>
    <template v-else-if="answer.status.value === 'answered'">
      <p
        class="feedback-verdict"
        :class="answer.correct.value ? 'is-correct' : 'is-wrong'"
        data-test="verdict"
        :data-correct="String(answer.correct.value)"
      >
        {{ answer.correct.value ? t('exercise.correct') : t('exercise.wrong') }}
        <span v-if="answer.xp.value > 0" class="badge badge-success" data-test="xp">{{
          t('exercise.xp', { xp: answer.xp.value })
        }}</span>
      </p>
      <i18n-t
        v-if="!answer.correct.value && solution"
        keypath="exercise.answer"
        scope="global"
        tag="p"
        data-test="solution"
      >
        <template #answer>
          <span :lang="lang" class="feedback-text">{{ solution }}</span>
        </template>
      </i18n-t>
      <i18n-t
        v-if="sentence"
        keypath="exercise.sentence"
        scope="global"
        tag="p"
        data-test="sentence"
      >
        <template #sentence>
          <span :lang="lang" class="feedback-text">{{ sentence }}</span>
        </template>
      </i18n-t>
    </template>
    <p v-if="answer.checkError.value" class="notice error" role="alert" data-test="check-error">
      {{ t('exercise.checkFailed', { error: answer.checkError.value }) }}
    </p>
    <p v-if="answer.recordError.value" class="notice warning" data-test="record-error">
      {{ t('exercise.recordFailed', { error: answer.recordError.value }) }}
    </p>
  </div>
</template>
