<script setup lang="ts">
/**
 * Multiple choice (Milestone M2): the prompt is a card's front, translation
 * or hanzi, depending on the exercise's direction, and the options are what
 * the adapter generated from the learned pool, in its order.
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useExerciseAnswer } from '../../composables/useExerciseAnswer'
import { DIRECTION_KEYS } from '../../domain/exercises'
import type { MultipleChoiceExercise } from '../../platform/types'
import ChoiceList from './ChoiceList.vue'
import ExerciseFeedback from './ExerciseFeedback.vue'

const props = defineProps<{ exercise: MultipleChoiceExercise }>()
const emit = defineEmits<{ answered: [correct: boolean] }>()

const { t } = useI18n()
const answer = useExerciseAnswer(
  () => props.exercise,
  (correct) => emit('answered', correct),
)
const { status } = answer
const chosen = ref<number | null>(null)

const answered = computed(() => status.value === 'answered')
const solution = computed(() => props.exercise.options[props.exercise.correct_index]?.text ?? null)

function choose(index: number): void {
  if (status.value !== 'idle') return
  chosen.value = index
  void answer.submit({ kind: 'choice', index })
}
</script>

<template>
  <div class="exercise" data-test="multiple-choice">
    <p class="exercise-instruction">{{ t(DIRECTION_KEYS[exercise.direction]) }}</p>
    <p class="exercise-prompt" :lang="exercise.prompt_lang" data-test="prompt">
      {{ exercise.prompt }}
    </p>
    <ChoiceList
      :options="exercise.options"
      :lang="exercise.option_lang"
      :chosen="chosen"
      :correct-index="answered ? exercise.correct_index : null"
      :disabled="status !== 'idle'"
      @choose="choose"
    />
    <ExerciseFeedback :answer="answer" :solution="solution" :lang="exercise.option_lang" />
  </div>
</template>
