<script setup lang="ts">
/**
 * Sentence builder (Milestone M2): rebuild a learned example sentence from
 * shuffled tiles, guided by its translation when the capture has one. The
 * sentence itself is the answer, so it is shown only once answered.
 */
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useExerciseAnswer } from '../../composables/useExerciseAnswer'
import type { SentenceBuilderExercise } from '../../platform/types'
import ExerciseFeedback from './ExerciseFeedback.vue'
import TileBoard from './TileBoard.vue'

const props = defineProps<{ exercise: SentenceBuilderExercise }>()
const emit = defineEmits<{ answered: [correct: boolean] }>()

const { t } = useI18n()
const answer = useExerciseAnswer(
  () => props.exercise,
  (correct) => emit('answered', correct),
)
const { status } = answer
const placed = ref<number[]>([])

function check(): void {
  if (placed.value.length > 0) void answer.submit({ kind: 'order', tile_ids: [...placed.value] })
}
</script>

<template>
  <div class="exercise" data-test="sentence-builder">
    <template v-if="exercise.prompt">
      <p class="exercise-instruction">{{ t('exercise.buildTranslation') }}</p>
      <p class="exercise-prompt exercise-prompt-sentence" data-test="prompt">
        {{ exercise.prompt }}
      </p>
    </template>
    <p v-else class="exercise-instruction">{{ t('exercise.buildSentence') }}</p>
    <TileBoard
      v-model="placed"
      :tiles="exercise.tiles"
      :lang="exercise.sentence_lang"
      :disabled="status !== 'idle'"
    />
    <div class="action-row">
      <button
        type="button"
        class="btn btn-primary"
        data-test="check"
        :disabled="status !== 'idle' || placed.length === 0"
        @click="check"
      >
        {{ status === 'checking' ? t('exercise.checking') : t('exercise.check') }}
      </button>
    </div>
    <ExerciseFeedback
      :answer="answer"
      :sentence="status === 'answered' ? exercise.sentence : null"
      :lang="exercise.sentence_lang"
    />
  </div>
</template>
