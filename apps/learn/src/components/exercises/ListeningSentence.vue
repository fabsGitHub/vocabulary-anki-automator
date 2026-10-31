<script setup lang="ts">
/**
 * Listening sentence (Milestone M2, variant b): hear a learned example
 * sentence, rebuild it from its tiles. The sentence is synthesized (the
 * exercise's `audio`; captures keep recordings of words only) and its text
 * is shown only once answered.
 */
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { AudioSource } from '../../composables/useAudio'
import { useExerciseAnswer } from '../../composables/useExerciseAnswer'
import type { ListeningSentenceExercise } from '../../platform/types'
import AudioButton from '../AudioButton.vue'
import ExerciseFeedback from './ExerciseFeedback.vue'
import TileBoard from './TileBoard.vue'

const props = defineProps<{ exercise: ListeningSentenceExercise }>()
const emit = defineEmits<{ answered: [correct: boolean] }>()

const { t } = useI18n()
const answer = useExerciseAnswer(
  () => props.exercise,
  (correct) => emit('answered', correct),
)
const { status } = answer
const placed = ref<number[]>([])

const source = computed<AudioSource>(() => ({
  cardId: props.exercise.audio.audio_card_id,
  text: props.exercise.audio.text,
  lang: props.exercise.audio.lang,
}))

function check(): void {
  if (placed.value.length > 0) void answer.submit({ kind: 'order', tile_ids: [...placed.value] })
}
</script>

<template>
  <div class="exercise" data-test="listening-sentence">
    <p class="exercise-instruction">{{ t('exercise.listenBuild') }}</p>
    <AudioButton :source="source" :label="t('exercise.play')" autoplay />
    <TileBoard
      v-model="placed"
      :tiles="exercise.tiles"
      :lang="exercise.audio.lang"
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
      :lang="exercise.audio.lang"
    />
  </div>
</template>
