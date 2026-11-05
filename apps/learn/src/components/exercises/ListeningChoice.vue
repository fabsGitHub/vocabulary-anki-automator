<script setup lang="ts">
/**
 * Listening choice (Milestone M2, variant a): hear a learned word, pick it
 * among learned fronts. The sound is the card's stored recording when the
 * exercise names one (`audio.audio_card_id`), else TTS of `audio.text`; it
 * plays once on its own and again on request.
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { AudioSource } from '../../composables/useAudio'
import { useExerciseAnswer } from '../../composables/useExerciseAnswer'
import type { ListeningChoiceExercise } from '../../platform/types'
import AudioButton from '../AudioButton.vue'
import ChoiceList from './ChoiceList.vue'
import ExerciseFeedback from './ExerciseFeedback.vue'

const props = defineProps<{ exercise: ListeningChoiceExercise }>()
const emit = defineEmits<{ answered: [correct: boolean] }>()

const { t } = useI18n()
const answer = useExerciseAnswer(
  () => props.exercise,
  (correct) => emit('answered', correct),
)
const { status } = answer
const chosen = ref<number | null>(null)

const source = computed<AudioSource>(() => ({
  cardId: props.exercise.audio.audio_card_id,
  text: props.exercise.audio.text,
  lang: props.exercise.audio.lang,
}))
const answered = computed(() => status.value === 'answered')
const solution = computed(() => props.exercise.options[props.exercise.correct_index]?.text ?? null)

function choose(index: number): void {
  if (status.value !== 'idle') return
  chosen.value = index
  void answer.submit({ kind: 'choice', index })
}
</script>

<template>
  <div class="exercise" data-test="listening-choice">
    <p class="exercise-instruction">{{ t('exercise.listenChoose') }}</p>
    <AudioButton :source="source" :label="t('exercise.play')" autoplay />
    <ChoiceList
      :options="exercise.options"
      :lang="exercise.audio.lang"
      :chosen="chosen"
      :correct-index="answered ? exercise.correct_index : null"
      :disabled="status !== 'idle'"
      @choose="choose"
    />
    <ExerciseFeedback :answer="answer" :solution="solution" :lang="exercise.audio.lang" />
  </div>
</template>
